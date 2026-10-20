//! 正向代理与 push 监听。运维 HTTP 由组合根的 Port 承担。

use std::sync::Arc;

use accessors_builtin::Incoming;
use bytes::Bytes;
use serde_json::{Value, json};
use strata_model::{ErrorClass, Locator, Timestamp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::Services;
use crate::transport::EgressGw;

const HEADER_CAP: usize = 16 * 1024;
const BODY_CAP: usize = 8 * 1024 * 1024;

pub async fn serve_proxy(gateway: Arc<EgressGw>, listener: TcpListener, policy: egress::Policy) {
    egress::serve(gateway.as_ref(), listener, policy).await;
}

pub async fn serve_push(services: Arc<Services>, listener: TcpListener) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            continue;
        };
        let services = Arc::clone(&services);
        tokio::spawn(async move {
            let _ = handle_push(services, socket).await;
        });
    }
}

async fn handle_push(services: Arc<Services>, mut socket: TcpStream) -> std::io::Result<()> {
    let (header, leftover) = match read_headers(&mut socket).await {
        Ok(parts) => parts,
        Err(status) => {
            write_response(&mut socket, status, &json!({"message": "spec 无效"})).await?;
            return Ok(());
        }
    };
    let text = String::from_utf8_lossy(&header);
    let mut lines = text.split("\r\n");
    let Some(request) = lines.next() else {
        write_response(&mut socket, 400, &json!({"message": "spec 无效"})).await?;
        return Ok(());
    };
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "POST" {
        write_response(&mut socket, 404, &json!({"message": "not-found"})).await?;
        return Ok(());
    }
    let Some(channel) = channel_of(target) else {
        write_response(&mut socket, 404, &json!({"message": "not-found"})).await?;
        return Ok(());
    };
    let mut content_length = 0usize;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            content_length = value.parse().unwrap_or(usize::MAX);
        }
        headers.push((name, value));
    }
    if content_length > BODY_CAP {
        write_response(&mut socket, 400, &json!({"message": "spec 无效"})).await?;
        return Ok(());
    }
    let mut body = Vec::with_capacity(content_length);
    let take = leftover.len().min(content_length);
    body.extend_from_slice(&leftover[..take]);
    if body.len() < content_length {
        let mut rest = vec![0u8; content_length - body.len()];
        socket.read_exact(&mut rest).await?;
        body.extend_from_slice(&rest);
    }
    let snapshot = services.config().snapshot();
    let Some((spec, policy)) = push_source(&snapshot, &channel) else {
        write_response(&mut socket, 404, &json!({"message": "not-found"})).await?;
        return Ok(());
    };
    let incoming = Incoming {
        channel,
        headers,
        body: Bytes::from(body),
    };
    match services
        .push()
        .ingest(
            &spec,
            &incoming,
            &services.access_context(),
            &policy,
            Timestamp::now(),
        )
        .await
    {
        Ok(_) => write_response(&mut socket, 200, &json!({})).await?,
        Err(error) => {
            let status = match error.message.as_str() {
                "unauthorized" => 401,
                "rate-limit" => 429,
                "idempotency" => 409,
                "spec 无效" => 400,
                _ => {
                    if error.class == ErrorClass::Config {
                        400
                    } else {
                        500
                    }
                }
            };
            write_response(&mut socket, status, &json!({"message": error.message})).await?;
        }
    }
    Ok(())
}

fn channel_of(target: &str) -> Option<String> {
    let path = target.split('?').next().unwrap_or(target);
    let channel = path.strip_prefix('/')?;
    if channel.is_empty() || channel.contains('/') {
        return None;
    }
    Some(channel.to_string())
}

fn push_source(snapshot: &config::Snapshot, channel: &str) -> Option<(Value, Value)> {
    for module in snapshot.modules.values() {
        if module.get("kind").and_then(Value::as_str) != Some("source") {
            continue;
        }
        let locator = module.pointer("/access/locator")?;
        let Locator::PushChannel { channel: found } = locator_of(locator)? else {
            continue;
        };
        if found != channel {
            continue;
        }
        let spec = module
            .pointer("/access/spec")
            .filter(|value| value.get("token").and_then(Value::as_str).is_some())
            .cloned()
            .or_else(|| {
                let accessor = module.pointer("/access/accessor")?.as_str()?;
                snapshot.modules.get(accessor)?.get("spec").cloned()
            })
            .unwrap_or_else(|| Value::Object(Default::default()));
        let policy = module
            .get("policy")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        return Some((spec, policy));
    }
    None
}

fn locator_of(value: &Value) -> Option<Locator> {
    if let Some(channel) = value.get("channel").and_then(Value::as_str) {
        return Some(Locator::PushChannel {
            channel: channel.to_string(),
        });
    }
    None
}

async fn read_headers(socket: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>), u16> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = socket.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(index) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            let split = index + 4;
            let rest = buf.split_off(split);
            return Ok((buf, rest));
        }
        if buf.len() > HEADER_CAP {
            return Err(400);
        }
    }
}

async fn write_response(socket: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let payload = body.to_string();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(payload.as_bytes()).await?;
    Ok(())
}
