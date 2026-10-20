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
use crate::ssrf::is_blocked;
use crate::state::{self, Acquire};

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
        address: SocketAddr,
    ) -> Result<HttpResponse, AccessError>;
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
        let address = self.pick_address(&url, policy).await?;
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
        let mut blocked = false;
        for ip in ips {
            if policy.private_networks == PrivateNetworks::Deny && is_blocked(ip) {
                blocked = true;
                continue;
            }
            return Ok(SocketAddr::new(ip, port));
        }
        if blocked {
            Err(permanent("ssrf"))
        } else {
            Err(permanent("dns"))
        }
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
            return Err(permanent("robots"));
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
                if now_ms.saturating_sub(row.fetched_ms) < ttl_ms {
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
        let response = self
            .send_direct(
                &fetch_policy,
                &mut robots_request,
                &mut fetch_now,
                deadline_ms,
            )
            .await;
        let row = match response {
            Ok(response) => state::RobotsRow {
                status: response.status,
                body: if response.status < 400 {
                    String::from_utf8_lossy(&response.body).into_owned()
                } else {
                    String::new()
                },
                fetched_ms: now_ms,
                failed: response.status >= 500,
            },
            Err(_) => state::RobotsRow {
                status: 0,
                body: String::new(),
                fetched_ms: now_ms,
                failed: true,
            },
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
        let now_ms = crate::proxy::clock_ms();
        self.request(&self.policy, request, now_ms, now_ms.saturating_add(30_000))
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
