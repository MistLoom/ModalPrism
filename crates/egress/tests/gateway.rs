use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use egress::{
    CrawlDelayMode, Dial, Exempt, Gateway, MemoryPersist, Policy, PrivateNetworks, RateLimit,
    Resolver, RobotsMode, Transport, Unreachable, is_blocked, registrable, serve,
};
use strata_model::ErrorClass;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use accessors::{AccessError, HttpRequest, HttpResponse};

struct SeqTransport {
    hits: AtomicUsize,
    urls: Mutex<Vec<String>>,
    responses: Mutex<Vec<Result<HttpResponse, AccessError>>>,
}

#[async_trait]
impl Transport for SeqTransport {
    async fn send(
        &self,
        request: HttpRequest,
        _address: SocketAddr,
    ) -> Result<HttpResponse, AccessError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.urls.lock().await.push(request.url);
        self.responses.lock().await.remove(0)
    }
}

struct MapResolver {
    ips: Vec<(String, IpAddr)>,
}

#[async_trait]
impl Resolver for MapResolver {
    async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, AccessError> {
        let found: Vec<_> = self
            .ips
            .iter()
            .filter(|(name, _)| name == host)
            .map(|(_, ip)| *ip)
            .collect();
        Ok(found)
    }
}

struct NoDial;

#[async_trait]
impl Dial for NoDial {
    async fn connect(&self, _addr: SocketAddr) -> Result<tokio::net::TcpStream, std::io::Error> {
        Err(std::io::Error::other("未连接"))
    }
}

fn public() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))
}

fn ok_body(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: Bytes::from(body.to_string()),
        final_url: String::new(),
    }
}

fn gateway(
    transport: SeqTransport,
    resolver: MapResolver,
    policy: Policy,
) -> Gateway<MemoryPersist, SeqTransport, MapResolver, NoDial> {
    Gateway::new(
        MemoryPersist::default(),
        transport,
        resolver,
        NoDial,
        policy,
    )
}

fn policy() -> Policy {
    let mut policy = Policy::library("ModalPrismBot/0.2");
    policy.check_robots = false;
    policy
}

fn request(url: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: Vec::new(),
        body: Bytes::new(),
    }
}

#[test]
fn registrable_domain_and_blocked_ranges() {
    assert_eq!(registrable("WWW.Example.com"), "example.com");
    assert_eq!(registrable("10.1.2.3"), "10.1.2.3");
    assert!(is_blocked("127.0.0.1".parse().unwrap()));
    assert!(is_blocked("10.1.1.1".parse().unwrap()));
    assert!(is_blocked("100.64.0.1".parse().unwrap()));
    assert!(is_blocked("169.254.1.1".parse().unwrap()));
    assert!(is_blocked("192.168.1.1".parse().unwrap()));
    assert!(is_blocked("::1".parse().unwrap()));
    assert!(is_blocked("::ffff:10.0.0.1".parse().unwrap()));
    assert!(is_blocked("::10.0.0.1".parse().unwrap()));
    assert!(is_blocked("2002:0a00:0001::".parse().unwrap()));
    assert!(is_blocked("64:ff9b::a00:1".parse().unwrap()));
    assert!(!is_blocked("2002:5db8:d822::".parse().unwrap()));
    assert!(!is_blocked(public()));
}

#[tokio::test]
async fn ssrf_rejects_private_and_allows_when_configured() {
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![Ok(ok_body(200, "x"))]),
    };
    let resolver = MapResolver {
        ips: vec![("evil.test".into(), "127.0.0.1".parse().unwrap())],
    };
    let gate = gateway(transport, resolver, policy());
    let error = gate
        .request(&policy(), request("http://evil.test/a"), 1_000, 1_000)
        .await
        .unwrap_err();
    assert_eq!(error.class, ErrorClass::Permanent);
    assert_eq!(error.message, "ssrf");
    assert_eq!(gate.transport().hits.load(Ordering::SeqCst), 0);

    let mut allow = policy();
    allow.private_networks = PrivateNetworks::Allow;
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![Ok(ok_body(200, "x"))]),
    };
    let resolver = MapResolver {
        ips: vec![("evil.test".into(), "10.1.1.1".parse().unwrap())],
    };
    let gate = gateway(transport, resolver, allow.clone());
    let response = gate
        .request(&allow, request("http://evil.test/a"), 1_000, 1_000)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn mixed_lookup_with_a_private_address_is_ssrf() {
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![Ok(ok_body(200, "x"))]),
    };
    let resolver = MapResolver {
        ips: vec![
            ("evil.test".into(), public()),
            ("evil.test".into(), "10.1.1.1".parse().unwrap()),
        ],
    };
    let gate = gateway(transport, resolver, policy());
    let error = gate
        .request(&policy(), request("http://evil.test/a"), 1_000, 1_000)
        .await
        .unwrap_err();
    assert_eq!(error.class, ErrorClass::Permanent);
    assert_eq!(error.message, "ssrf");
    assert_eq!(gate.transport().hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn token_bucket_is_per_registrable_domain() {
    let mut responses = Vec::new();
    for _ in 0..4 {
        responses.push(Ok(ok_body(200, "ok")));
    }
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(responses),
    };
    let resolver = MapResolver {
        ips: vec![
            ("a.example.com".into(), public()),
            ("b.example.com".into(), public()),
        ],
    };
    let gate = gateway(transport, resolver, policy());
    let policy = policy();
    for url in [
        "http://a.example.com/1",
        "http://b.example.com/2",
        "http://a.example.com/3",
    ] {
        gate.request(&policy, request(url), 5_000, 5_000)
            .await
            .unwrap();
    }
    let error = gate
        .request(&policy, request("http://b.example.com/4"), 5_000, 5_000)
        .await
        .unwrap_err();
    assert_eq!(error.message, "rate-limit");
}

#[tokio::test]
async fn host_override_has_its_own_bucket() {
    let mut responses = Vec::new();
    for _ in 0..4 {
        responses.push(Ok(ok_body(200, "ok")));
    }
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(responses),
    };
    let resolver = MapResolver {
        ips: vec![
            ("www.example.com".into(), public()),
            ("other.example.com".into(), public()),
        ],
    };
    let mut policy = policy();
    policy.host_rates.insert(
        "www.example.com".into(),
        RateLimit {
            per_sec_micros: 1_000_000,
            burst: 1,
            concurrency: 2,
        },
    );
    let gate = gateway(transport, resolver, policy.clone());
    gate.request(&policy, request("http://www.example.com/a"), 5_000, 5_000)
        .await
        .unwrap();
    let limited = gate
        .request(&policy, request("http://www.example.com/b"), 5_000, 5_000)
        .await
        .unwrap_err();
    assert_eq!(limited.message, "rate-limit");
    gate.request(&policy, request("http://other.example.com/c"), 5_000, 5_000)
        .await
        .unwrap();
}

#[tokio::test]
async fn circuit_opens_without_another_send() {
    let mut responses = Vec::new();
    for _ in 0..6 {
        responses.push(Ok(ok_body(500, "no")));
    }
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(responses),
    };
    let resolver = MapResolver {
        ips: vec![("example.com".into(), public())],
    };
    let mut policy = policy();
    policy.rate.burst = 8;
    policy.rate.concurrency = 8;
    let gate = gateway(transport, resolver, policy.clone());
    for _ in 0..5 {
        let response = gate
            .request(&policy, request("http://example.com/x"), 8_000, 8_000)
            .await
            .unwrap();
        assert_eq!(response.status, 500);
    }
    let error = gate
        .request(&policy, request("http://example.com/x"), 8_000, 8_000)
        .await
        .unwrap_err();
    assert_eq!(error.message, "circuit-open");
    assert_eq!(gate.transport().hits.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn robots_disallow_exemption_and_unreachable() {
    let body = "User-agent: *\nDisallow: /secret\nCrawl-delay: 5\n";
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![
            Ok(ok_body(200, body)),
            Ok(ok_body(200, "page")),
            Ok(ok_body(200, "exempt")),
            Ok(ok_body(503, "")),
            Ok(ok_body(404, "")),
            Ok(ok_body(200, "open")),
        ]),
    };
    let resolver = MapResolver {
        ips: vec![("example.com".into(), public())],
    };
    let mut policy = policy();
    policy.check_robots = true;
    policy.robots = RobotsMode::Obey;
    policy.crawl_delay = CrawlDelayMode::Cap(std::time::Duration::from_secs(1));
    let gate = gateway(transport, resolver, policy.clone());
    let denied = gate
        .request(&policy, request("http://example.com/secret"), 9_000, 9_000)
        .await
        .unwrap_err();
    assert_eq!(denied.message, "robots");
    gate.request(&policy, request("http://example.com/ok"), 9_000, 9_000)
        .await
        .unwrap();

    let mut exempted = policy.clone();
    exempted.robots = RobotsMode::ObeyWithExemptions;
    exempted.exempt = Some(Exempt {
        category: "feed".into(),
        reason: "公开 Feed".into(),
    });
    gate.request(
        &exempted,
        request("http://example.com/secret"),
        9_000,
        9_000,
    )
    .await
    .unwrap();
    assert_eq!(gate.audits().await.unwrap().len(), 1);

    let mut closed = policy.clone();
    closed.robots_ttl = std::time::Duration::from_secs(0);
    let blocked = gate
        .request(&closed, request("http://example.com/ok"), 20_000, 20_000)
        .await
        .unwrap_err();
    assert_eq!(blocked.message, "robots");

    let mut again = closed.clone();
    again.on_unreachable = Unreachable::Rfc9309;
    let response = gate
        .request(&again, request("http://example.com/ok"), 30_000, 30_000)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn robots_redirect_is_followed_before_the_verdict() {
    let mut redirect = ok_body(301, "");
    redirect.headers.push((
        "Location".into(),
        "https://example.com/robots-real.txt".into(),
    ));
    let body = "User-agent: *\nDisallow: /secret\n";
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![Ok(redirect), Ok(ok_body(200, body))]),
    };
    let resolver = MapResolver {
        ips: vec![("example.com".into(), public())],
    };
    let mut policy = policy();
    policy.check_robots = true;
    policy.robots = RobotsMode::Obey;
    let gate = gateway(transport, resolver, policy.clone());
    let denied = gate
        .request(&policy, request("http://example.com/secret"), 9_000, 9_000)
        .await
        .unwrap_err();
    assert_eq!(denied.message, "robots");
    let urls = gate.transport().urls.lock().await.clone();
    assert_eq!(
        urls,
        vec![
            "http://example.com:80/robots.txt".to_string(),
            "https://example.com/robots-real.txt".to_string(),
        ]
    );
}

#[tokio::test]
async fn credentialed_redirect_stays_on_origin() {
    let mut redirect = ok_body(302, "");
    redirect
        .headers
        .push(("Location".into(), "https://other.test/x".into()));
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(vec![Ok(redirect)]),
    };
    let resolver = MapResolver {
        ips: vec![
            ("example.com".into(), public()),
            ("other.test".into(), public()),
        ],
    };
    let gate = gateway(transport, resolver, policy());
    let mut request = request("https://example.com/a");
    request.headers.push(("Cookie".into(), "a=b".into()));
    let error = gate
        .request(&policy(), request, 1_000, 1_000)
        .await
        .unwrap_err();
    assert_eq!(error.message, "redirect-origin");
    assert_eq!(gate.transport().hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proxy_connect_filters_private_and_tunnels_when_allowed() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    let echo_task = tokio::spawn(async move {
        let (mut socket, _) = echo.accept().await.unwrap();
        let mut buf = [0u8; 4];
        socket.read_exact(&mut buf).await.unwrap();
        socket.write_all(&buf).await.unwrap();
    });
    let transport = SeqTransport {
        hits: AtomicUsize::new(0),
        urls: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    };
    let resolver = MapResolver { ips: Vec::new() };
    let mut policy = policy();
    policy.private_networks = PrivateNetworks::Deny;
    let gate = Arc::new(Gateway::new(
        MemoryPersist::default(),
        transport,
        resolver,
        egress::TcpDial,
        policy.clone(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let served = gate.clone();
    let serve_policy = policy.clone();
    tokio::spawn(async move {
        serve(&served, listener, serve_policy).await;
    });

    let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let payload = format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    client.write_all(payload.as_bytes()).await.unwrap();
    let mut status = [0u8; 12];
    client.read_exact(&mut status).await.unwrap();
    assert!(std::str::from_utf8(&status).unwrap().contains("403"));

    let mut allow = policy.clone();
    allow.private_networks = PrivateNetworks::Allow;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let served = gate.clone();
    tokio::spawn(async move {
        serve(&served, listener, allow).await;
    });
    let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    client.write_all(payload.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 64];
    let n = client.read(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf[..n]);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    client.write_all(b"ping").await.unwrap();
    let mut got = [0u8; 4];
    client.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"ping");
    echo_task.await.unwrap();
}
