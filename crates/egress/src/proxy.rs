//! 正向代理。CONNECT 与绝对形式的 GET/HEAD。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use accessors::HttpRequest;

use crate::fail::transient;
use crate::gateway::Gateway;
use crate::persist::Persist;
use crate::policy::Policy;
use crate::{Dial, Resolver, Transport};

pub async fn serve<P, T, R, D>(gateway: &Gateway<P, T, R, D>, listener: TcpListener, policy: Policy)
where
    P: Persist,
    T: Transport,
    R: Resolver,
    D: Dial,
{
    serve_each(gateway, listener, move || policy.clone()).await;
}

pub async fn serve_each<P, T, R, D, F>(
    gateway: &Gateway<P, T, R, D>,
    listener: TcpListener,
    policy_of: F,
) where
    P: Persist,
    T: Transport,
    R: Resolver,
    D: Dial,
    F: Fn() -> Policy,
{
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            continue;
        };
        let policy = policy_of();
        let _ = handle(gateway, socket, policy).await;
    }
}

async fn handle<P, T, R, D>(
    gateway: &Gateway<P, T, R, D>,
    mut socket: TcpStream,
    policy: Policy,
) -> Result<(), accessors::AccessError>
where
    P: Persist,
    T: Transport,
    R: Resolver,
    D: Dial,
{
    let header = read_header(&mut socket).await?;
    let text = String::from_utf8_lossy(&header);
    let mut lines = text.split("\r\n");
    let Some(start) = lines.next() else {
        return Ok(());
    };
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let now_ms = clock_ms();
    let deadline_ms = now_ms.saturating_add(30_000);
    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = target.rsplit_once(':') else {
            write_status(&mut socket, 403).await?;
            return Ok(());
        };
        let Ok(port) = port.parse::<u16>() else {
            write_status(&mut socket, 403).await?;
            return Ok(());
        };
        let admission = match gateway
            .admit_host(&policy, host, port, now_ms, deadline_ms)
            .await
        {
            Ok(admission) => admission,
            Err(_) => {
                write_status(&mut socket, 403).await?;
                return Ok(());
            }
        };
        let mut remote = match gateway.dial().connect(admission.address).await {
            Ok(remote) => remote,
            Err(_) => {
                let _ = gateway.complete(&admission, false, now_ms).await;
                write_status(&mut socket, 403).await?;
                return Ok(());
            }
        };
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .map_err(|source| {
                accessors::AccessError::with_source(
                    strata_model::ErrorClass::Transient,
                    "connect",
                    source,
                )
            })?;
        let copied = tokio::io::copy_bidirectional(&mut socket, &mut remote)
            .await
            .is_ok();
        gateway.complete(&admission, copied, clock_ms()).await?;
        return Ok(());
    }
    if method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD") {
        if !target.starts_with("http://") && !target.starts_with("https://") {
            write_status(&mut socket, 405).await?;
            return Ok(());
        }
        let mut proxied = policy.clone();
        proxied.check_robots = false;
        let response = gateway
            .request(
                &proxied,
                HttpRequest {
                    method: method.to_string(),
                    url: target.to_string(),
                    headers: Vec::new(),
                    body: bytes::Bytes::new(),
                },
                now_ms,
                deadline_ms,
            )
            .await;
        match response {
            Ok(response) => {
                let status = format!(
                    "HTTP/1.1 {} OK\r\nContent-Length: {}\r\n\r\n",
                    response.status,
                    response.body.len()
                );
                socket
                    .write_all(status.as_bytes())
                    .await
                    .map_err(|source| {
                        accessors::AccessError::with_source(
                            strata_model::ErrorClass::Transient,
                            "connect",
                            source,
                        )
                    })?;
                socket.write_all(&response.body).await.map_err(|source| {
                    accessors::AccessError::with_source(
                        strata_model::ErrorClass::Transient,
                        "connect",
                        source,
                    )
                })?;
            }
            Err(_) => write_status(&mut socket, 403).await?,
        }
        return Ok(());
    }
    write_status(&mut socket, 405).await?;
    Ok(())
}

async fn read_header(socket: &mut TcpStream) -> Result<Vec<u8>, accessors::AccessError> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        socket.read_exact(&mut byte).await.map_err(|source| {
            accessors::AccessError::with_source(
                strata_model::ErrorClass::Transient,
                "connect",
                source,
            )
        })?;
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            return Ok(buf);
        }
        if buf.len() > 16 * 1024 {
            return Err(transient("connect"));
        }
    }
}

async fn write_status(socket: &mut TcpStream, status: u16) -> Result<(), accessors::AccessError> {
    let body = format!("HTTP/1.1 {status} status\r\nContent-Length: 0\r\n\r\n");
    socket.write_all(body.as_bytes()).await.map_err(|source| {
        accessors::AccessError::with_source(strata_model::ErrorClass::Transient, "connect", source)
    })
}

pub(crate) fn clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
