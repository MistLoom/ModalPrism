//! 库调用入口。每一跳都做 SSRF、robots、熔断和令牌桶。

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use url::Url;

use accessors::{AccessError, EgressGateway, HttpRequest, HttpResponse};

use crate::domain::registrable;
use crate::fail::{permanent, transient};
use crate::persist::{Persist, load_db, save_db};
use crate::policy::{CrawlDelayMode, Policy, PrivateNetworks, RateLimit, RobotsMode, Unreachable};
use crate::robots::{self, Decision};
use crate::ssrf::{is_blocked, is_fake_ip};
use crate::state::{self, Acquire};

/// robots 拉取失败只缓存 5 分钟，网络恢复后可以尽快重新判定。
const FAILED_TTL_MS: i64 = 5 * 60 * 1000;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
        address: Option<SocketAddr>,
    ) -> Result<HttpResponse, AccessError>;

    /// 该目标是否经上游代理发送。为真时本地不解析地址，也不做私网拦截。
    fn proxied(&self, _url: &Url) -> bool {
        false
    }
}

#[async_trait]
pub trait Resolver: Send + Sync {
    async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, AccessError>;
}

#[async_trait]
pub trait Dial: Send + Sync {
    async fn connect(&self, addr: SocketAddr) -> Result<TcpStream, std::io::Error>;
}

pub struct TcpDial;

#[async_trait]
impl Dial for TcpDial {
    async fn connect(&self, addr: SocketAddr) -> Result<TcpStream, std::io::Error> {
        TcpStream::connect(addr).await
    }
}

pub struct Gateway<P, T, R, D> {
    persist: P,
    transport: T,
    resolver: R,
    dial: D,
    policy: Policy,
    lock: Mutex<()>,
}

pub struct Admission {
    pub address: SocketAddr,
    key: String,
    host: String,
    limit: RateLimit,
}

/// 运维视图。不返回 robots 正文。
#[derive(Debug, serde::Serialize)]
pub struct Report {
    pub robots: Vec<RobotsView>,
    pub circuits: Vec<CircuitView>,
    pub rates: Vec<RateView>,
    pub audits: Vec<state::AuditRow>,
}

/// robots 缓存的一条记录。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RobotsView {
    pub origin: String,
    pub status: u16,
    pub failed: bool,
    pub fetched_at: i64,
    pub body_bytes: usize,
}

/// 熔断的一条记录。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CircuitView {
    pub host: String,
    pub consecutive: u32,
    pub open_until: Option<i64>,
    pub cooldown_ms: i64,
    pub openings: u32,
}

/// 令牌桶的一条记录。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateView {
    pub key: String,
    pub tokens: i64,
    pub in_flight: u32,
    pub not_before: Option<i64>,
}

impl<P, T, R, D> Gateway<P, T, R, D>
where
    P: Persist,
    T: Transport,
    R: Resolver,
    D: Dial,
{
    pub fn new(persist: P, transport: T, resolver: R, dial: D, policy: Policy) -> Self {
        Self {
            persist,
            transport,
            resolver,
            dial,
            policy,
            lock: Mutex::new(()),
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn dial(&self) -> &D {
        &self.dial
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub async fn audits(&self) -> Result<Vec<state::AuditRow>, AccessError> {
        let _guard = self.lock.lock().await;
        Ok(load_db(&self.persist).await?.audits)
    }

    /// 手动重置一台主机的熔断，返回是否存在该主机的熔断记录。
    pub async fn reset_circuit(&self, host: &str) -> Result<bool, AccessError> {
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        let existed = db.circuits.contains_key(host);
        state::circuit_reset(&mut db, host);
        save_db(&self.persist, &db).await?;
        Ok(existed)
    }

    /// 运维视图：robots、熔断、令牌桶与最近审计。不返回 robots 正文。
    pub async fn report(&self) -> Result<Report, AccessError> {
        let _guard = self.lock.lock().await;
        let db = load_db(&self.persist).await?;
        let robots = db
            .robots
            .iter()
            .map(|(origin, row)| RobotsView {
                origin: origin.clone(),
                status: row.status,
                failed: row.failed,
                fetched_at: row.fetched_ms,
                body_bytes: row.body.len(),
            })
            .collect();
        let circuits = db
            .circuits
            .iter()
            .map(|(host, row)| CircuitView {
                host: host.clone(),
                consecutive: row.consecutive,
                open_until: row.open_until,
                cooldown_ms: row.cooldown_ms,
                openings: row.openings,
            })
            .collect();
        let rates = db
            .rates
            .iter()
            .map(|(key, row)| RateView {
                key: key.clone(),
                tokens: row.tokens,
                in_flight: row.in_flight,
                not_before: row.not_before,
            })
            .collect();
        let mut audits: Vec<_> = db.audits.iter().rev().take(20).cloned().collect();
        audits.reverse();
        Ok(Report {
            robots,
            circuits,
            rates,
            audits,
        })
    }

    pub async fn request(
        &self,
        policy: &Policy,
        mut request: HttpRequest,
        mut now_ms: i64,
        deadline_ms: i64,
    ) -> Result<HttpResponse, AccessError> {
        prepare_headers(policy, &mut request);
        let mut redirects = 0u8;
        loop {
            let url = Url::parse(&request.url).map_err(|_| permanent("scheme"))?;
            if policy.check_robots {
                now_ms = self
                    .enforce_robots(policy, &url, &request, now_ms, deadline_ms)
                    .await?;
            }
            let response = self
                .send_direct(policy, &mut request, &mut now_ms, deadline_ms)
                .await?;
            let Some(next) = follow_redirect(&request, &response)? else {
                return Ok(response);
            };
            redirects = redirects.saturating_add(1);
            if redirects > 5 {
                return Err(transient("redirect"));
            }
            request = next;
        }
    }

    async fn send_direct(
        &self,
        policy: &Policy,
        request: &mut HttpRequest,
        now_ms: &mut i64,
        deadline_ms: i64,
    ) -> Result<HttpResponse, AccessError> {
        let url = Url::parse(&request.url).map_err(|_| permanent("scheme"))?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(permanent("scheme"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(permanent("ssrf"));
        }
        let address = if self.transport.proxied(&url) {
            None
        } else {
            Some(self.pick_address(&url, policy).await?)
        };
        self.ensure_circuit(&url, *now_ms).await?;
        let (key, limit) = bucket_of(policy, &url);
        *now_ms = self
            .acquire_ready(&key, &limit, *now_ms, deadline_ms)
            .await?;
        let started = std::time::Instant::now();
        let sent = self.transport.send(request.clone(), address).await;
        let elapsed_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let status = sent.as_ref().ok().map(|response| response.status);
        let retry_at = sent
            .as_ref()
            .ok()
            .and_then(|response| retry_after_ms(&response.headers, *now_ms));
        self.finish(HopResult {
            key: &key,
            host: host_of(&url),
            limit: &limit,
            status,
            elapsed_ms,
            retry_at,
            now_ms: *now_ms,
        })
        .await?;
        sent
    }

    /// 正向代理的主机准入：SSRF、熔断、令牌桶。不检查 robots。令牌留到 `complete` 归还。
    pub async fn admit_host(
        &self,
        policy: &Policy,
        host: &str,
        port: u16,
        now_ms: i64,
        deadline_ms: i64,
    ) -> Result<Admission, AccessError> {
        let host_for_url = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let url = Url::parse(&format!("http://{host_for_url}:{port}/"))
            .map_err(|_| permanent("scheme"))?;
        let address = self.pick_address(&url, policy).await?;
        self.ensure_circuit(&url, now_ms).await?;
        let (key, limit) = bucket_of(policy, &url);
        self.acquire_ready(&key, &limit, now_ms, deadline_ms)
            .await?;
        Ok(Admission {
            address,
            key,
            host: host_of(&url).to_string(),
            limit,
        })
    }

    pub async fn complete(
        &self,
        admission: &Admission,
        ok: bool,
        now_ms: i64,
    ) -> Result<(), AccessError> {
        let status = if ok { Some(200) } else { None };
        self.finish(HopResult {
            key: &admission.key,
            host: &admission.host,
            limit: &admission.limit,
            status,
            elapsed_ms: 0,
            retry_at: None,
            now_ms,
        })
        .await
    }

    async fn pick_address(&self, url: &Url, policy: &Policy) -> Result<SocketAddr, AccessError> {
        let host = url.host_str().ok_or_else(|| permanent("scheme"))?;
        let port = url.port_or_known_default().unwrap_or(0);
        let ips = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![ip]
        } else {
            match self.resolver.lookup(host).await {
                Ok(ips) if ips.is_empty() => return Err(permanent("dns")),
                Ok(ips) => ips,
                Err(error) => return Err(error),
            }
        };
        let mut chosen = None;
        for ip in ips {
            if policy.private_networks == PrivateNetworks::Deny && is_blocked(ip) {
                tracing::warn!(host, ip = %ip, "出站地址被私网规则拦截");
                return Err(permanent("ssrf"));
            }
            if is_fake_ip(ip) {
                tracing::warn!(host, ip = %ip, "地址落在 TUN fake-IP 段，按公网处理");
            }
            if chosen.is_none() {
                chosen = Some(SocketAddr::new(ip, port));
            }
        }
        chosen.ok_or_else(|| permanent("dns"))
    }

    async fn enforce_robots(
        &self,
        policy: &Policy,
        url: &Url,
        request: &HttpRequest,
        mut now_ms: i64,
        deadline_ms: i64,
    ) -> Result<i64, AccessError> {
        if policy.robots == RobotsMode::Ignore {
            return Ok(now_ms);
        }
        let origin = origin_of(url);
        if policy.robots == RobotsMode::ObeyWithExemptions {
            if let Some(exempt) = &policy.exempt {
                let _guard = self.lock.lock().await;
                let mut db = load_db(&self.persist).await?;
                db.audits.push(state::AuditRow {
                    origin: origin.clone(),
                    category: exempt.category.clone(),
                    reason: exempt.reason.clone(),
                    at_ms: now_ms,
                });
                save_db(&self.persist, &db).await?;
                return Ok(now_ms);
            }
        }
        let row = self.robots_row(policy, url, now_ms, deadline_ms).await?;
        now_ms = now_ms.max(row.fetched_ms);
        let user_agent =
            header(&request.headers, "user-agent").unwrap_or(policy.user_agent.as_str());
        let decision = verdict(policy, &row, user_agent, url.path());
        if decision == Decision::Disallow {
            if row.failed {
                tracing::warn!(
                    origin = %origin,
                    status = row.status,
                    "robots 不可达，按 RFC9309 判为禁止"
                );
                return Err(transient("robots-unreachable"));
            }
            tracing::warn!(origin = %origin, path = url.path(), "robots 禁止访问");
            return Err(permanent("robots"));
        }
        if row.failed {
            tracing::warn!(origin = %origin, status = row.status, "robots 不可达，按策略放行");
        }
        let delay = crawl_delay(policy, &row, user_agent, url.path());
        loop {
            let wait = {
                let _guard = self.lock.lock().await;
                let db = load_db(&self.persist).await?;
                state::origin_wait(&db, &origin, now_ms, deadline_ms)
            };
            match wait {
                Acquire::Ready => break,
                Acquire::Denied(message) => return Err(transient(message)),
                Acquire::Wait(ms) => now_ms = step(now_ms, ms, deadline_ms, "crawl-delay").await?,
            }
        }
        if let Some(delay) = delay {
            let _guard = self.lock.lock().await;
            let mut db = load_db(&self.persist).await?;
            state::origin_mark(&mut db, &origin, now_ms, delay);
            save_db(&self.persist, &db).await?;
        }
        Ok(now_ms)
    }

    async fn robots_row(
        &self,
        policy: &Policy,
        url: &Url,
        now_ms: i64,
        deadline_ms: i64,
    ) -> Result<state::RobotsRow, AccessError> {
        let origin = origin_of(url);
        let ttl_ms = i64::try_from(policy.robots_ttl.as_millis()).unwrap_or(i64::MAX);
        {
            let _guard = self.lock.lock().await;
            let db = load_db(&self.persist).await?;
            if let Some(row) = db.robots.get(&origin) {
                let ttl = if row.failed {
                    FAILED_TTL_MS.min(ttl_ms)
                } else {
                    ttl_ms
                };
                if now_ms.saturating_sub(row.fetched_ms) < ttl {
                    return Ok(row.clone());
                }
            }
        }
        let mut fetch_policy = policy.clone();
        fetch_policy.check_robots = false;
        fetch_policy.exempt = None;
        let robots_url = format!("{origin}/robots.txt");
        let mut robots_request = HttpRequest {
            method: "GET".into(),
            url: robots_url,
            headers: vec![("User-Agent".into(), policy.user_agent.clone())],
            body: bytes::Bytes::new(),
        };
        let mut fetch_now = now_ms;
        let mut response = self
            .send_direct(
                &fetch_policy,
                &mut robots_request,
                &mut fetch_now,
                deadline_ms,
            )
            .await;
        let mut hops = 0u8;
        while let Ok(current) = &response {
            let Some(next) = follow_redirect(&robots_request, current)? else {
                break;
            };
            hops = hops.saturating_add(1);
            if hops > 5 {
                break;
            }
            robots_request = next;
            response = self
                .send_direct(
                    &fetch_policy,
                    &mut robots_request,
                    &mut fetch_now,
                    deadline_ms,
                )
                .await;
        }
        let row = match response {
            Ok(response) => {
                let unfinished = matches!(response.status, 301 | 302 | 303 | 307 | 308);
                state::RobotsRow {
                    status: response.status,
                    body: if response.status < 400 && !unfinished {
                        String::from_utf8_lossy(&response.body).into_owned()
                    } else {
                        String::new()
                    },
                    fetched_ms: now_ms,
                    failed: response.status >= 500 || unfinished,
                }
            }
            Err(error) => {
                tracing::warn!(origin = %origin, error = %error.message, "robots 拉取失败");
                state::RobotsRow {
                    status: 0,
                    body: String::new(),
                    fetched_ms: now_ms,
                    failed: true,
                }
            }
        };
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        db.robots.insert(origin, row.clone());
        save_db(&self.persist, &db).await?;
        Ok(row)
    }

    async fn ensure_circuit(&self, url: &Url, now_ms: i64) -> Result<(), AccessError> {
        let _guard = self.lock.lock().await;
        let db = load_db(&self.persist).await?;
        if state::circuit_open(&db, host_of(url), now_ms) {
            Err(transient("circuit-open"))
        } else {
            Ok(())
        }
    }

    async fn acquire_ready(
        &self,
        key: &str,
        limit: &RateLimit,
        mut now_ms: i64,
        deadline_ms: i64,
    ) -> Result<i64, AccessError> {
        loop {
            let decision = {
                let _guard = self.lock.lock().await;
                let mut db = load_db(&self.persist).await?;
                let decision = state::acquire(&mut db, key, limit, now_ms, deadline_ms);
                if decision == Acquire::Ready {
                    save_db(&self.persist, &db).await?;
                }
                decision
            };
            match decision {
                Acquire::Ready => return Ok(now_ms),
                Acquire::Denied(message) => return Err(transient(message)),
                Acquire::Wait(ms) => now_ms = step(now_ms, ms, deadline_ms, "rate-limit").await?,
            }
        }
    }

    async fn finish(&self, hop: HopResult<'_>) -> Result<(), AccessError> {
        let _guard = self.lock.lock().await;
        let mut db = load_db(&self.persist).await?;
        state::release(&mut db, hop.key);
        match hop.status {
            Some(status) => {
                state::note_response(
                    &mut db,
                    hop.key,
                    hop.limit,
                    status,
                    hop.elapsed_ms,
                    hop.retry_at,
                );
                state::circuit_result(&mut db, hop.host, status_ok(status), hop.now_ms);
            }
            None => state::circuit_result(&mut db, hop.host, false, hop.now_ms),
        }
        save_db(&self.persist, &db).await
    }
}

struct HopResult<'a> {
    key: &'a str,
    host: &'a str,
    limit: &'a RateLimit,
    status: Option<u16>,
    elapsed_ms: i64,
    retry_at: Option<i64>,
    now_ms: i64,
}

#[async_trait]
impl<P, T, R, D> EgressGateway for Gateway<P, T, R, D>
where
    P: Persist,
    T: Transport,
    R: Resolver,
    D: Dial,
{
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, AccessError> {
        self.request_governed(&self.policy, request).await
    }
}

impl<P, T, R, D> Gateway<P, T, R, D>
where
    P: Persist,
    T: Transport,
    R: Resolver,
    D: Dial,
{
    pub async fn request_governed(
        &self,
        policy: &Policy,
        request: HttpRequest,
    ) -> Result<HttpResponse, AccessError> {
        let now_ms = crate::proxy::clock_ms();
        self.request(policy, request, now_ms, now_ms.saturating_add(30_000))
            .await
    }
}

fn status_ok(status: u16) -> bool {
    status != 429 && !(500..600).contains(&status)
}

fn bucket_of(policy: &Policy, url: &Url) -> (String, RateLimit) {
    let host = host_of(url).to_string();
    if let Some(limit) = policy.host_rates.get(&host) {
        return (host, limit.clone());
    }
    (registrable(&host), policy.rate.clone())
}

fn host_of(url: &Url) -> &str {
    url.host_str().unwrap_or("")
}

fn origin_of(url: &Url) -> String {
    let port = url.port_or_known_default().unwrap_or(0);
    format!("{}://{}:{port}", url.scheme(), host_of(url))
}

fn prepare_headers(policy: &Policy, request: &mut HttpRequest) {
    if header(&request.headers, "user-agent").is_none() {
        request
            .headers
            .push(("User-Agent".into(), policy.user_agent.clone()));
    }
    if let Some(contact) = policy.contact.as_ref().filter(|value| !value.is_empty()) {
        if header(&request.headers, "from").is_none() {
            request.headers.push(("From".into(), contact.clone()));
        }
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn follow_redirect(
    request: &HttpRequest,
    response: &HttpResponse,
) -> Result<Option<HttpRequest>, AccessError> {
    if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = header(&response.headers, "location") else {
        return Ok(None);
    };
    let current = Url::parse(&request.url).map_err(|_| permanent("scheme"))?;
    let next = current.join(location).map_err(|_| permanent("scheme"))?;
    if has_credentials(&request.headers) && !same_origin(&current, &next) {
        return Err(permanent("redirect-origin"));
    }
    let mut headers = request.headers.clone();
    let (method, body) = if response.status == 303 {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
        ("GET".into(), bytes::Bytes::new())
    } else {
        (request.method.clone(), request.body.clone())
    };
    Ok(Some(HttpRequest {
        method,
        url: next.to_string(),
        headers,
        body,
    }))
}

fn has_credentials(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, _)| {
        name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("cookie")
    })
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn verdict(policy: &Policy, row: &state::RobotsRow, user_agent: &str, path: &str) -> Decision {
    if row.failed || row.status >= 400 {
        return match policy.on_unreachable {
            Unreachable::Allow => Decision::Allow,
            Unreachable::Disallow => Decision::Disallow,
            Unreachable::Rfc9309 if row.failed || row.status >= 500 => Decision::Disallow,
            Unreachable::Rfc9309 => Decision::Allow,
        };
    }
    robots::decide(&robots::parse(&row.body), user_agent, path).0
}

fn crawl_delay(
    policy: &Policy,
    row: &state::RobotsRow,
    user_agent: &str,
    path: &str,
) -> Option<Duration> {
    if row.failed || row.status >= 400 {
        return None;
    }
    let delay = robots::decide(&robots::parse(&row.body), user_agent, path).1?;
    match policy.crawl_delay {
        CrawlDelayMode::Ignore => None,
        CrawlDelayMode::Honor => Some(delay),
        CrawlDelayMode::Cap(cap) => Some(delay.min(cap)),
    }
}

fn retry_after_ms(headers: &[(String, String)], now_ms: i64) -> Option<i64> {
    let value = header(headers, "retry-after")?;
    if let Ok(seconds) = value.trim().parse::<i64>() {
        return Some(now_ms.saturating_add(seconds.saturating_mul(1000).max(0)));
    }
    let parsed =
        time::OffsetDateTime::parse(value.trim(), &time::format_description::well_known::Rfc2822)
            .ok()?;
    let ms = parsed.unix_timestamp_nanos() / 1_000_000;
    i64::try_from(ms).ok()
}

async fn step(
    now_ms: i64,
    wait_ms: i64,
    deadline_ms: i64,
    message: &'static str,
) -> Result<i64, AccessError> {
    if wait_ms <= 0 || now_ms.saturating_add(wait_ms) > deadline_ms {
        return Err(transient(message));
    }
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(wait_ms).unwrap_or(u64::MAX),
    ))
    .await;
    Ok(now_ms.saturating_add(wait_ms))
}
