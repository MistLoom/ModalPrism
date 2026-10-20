//! 经出站端口发送一页，并做条件请求与 gzip。

use std::time::Instant;

use accessors::{AccessContext, AccessError, AccessHints, HttpRequest};
use bytes::Bytes;

use crate::headers::decode_body;
use crate::ready::ready;

pub struct Outbound {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub session: Option<String>,
    pub hints: AccessHints,
}

pub struct Fetched {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub final_url: String,
}

pub async fn send(
    ctx: &AccessContext,
    deadline: Instant,
    call: Outbound,
) -> Result<Fetched, AccessError> {
    ready(ctx, deadline)?;
    let mut headers = call.headers;
    if let Some(etag) = call.hints.etag {
        push_missing(&mut headers, "If-None-Match", etag);
    }
    if let Some(modified) = call.hints.last_modified {
        push_missing(&mut headers, "If-Modified-Since", modified);
    }
    if let Some(profile) = call.session.as_deref() {
        let session = ctx.sessions.get(profile).await?;
        for (name, value) in session.headers {
            headers.push((name, value));
        }
        if let Some(cookie) = session.cookie {
            push_missing(&mut headers, "Cookie", cookie);
        }
    }
    if header_missing(&headers, "accept-encoding") {
        headers.push(("Accept-Encoding".into(), "gzip".into()));
    }
    let response = ctx
        .egress
        .request(HttpRequest {
            method: call.method,
            url: call.url,
            headers,
            body: call.body,
        })
        .await?;
    let mut headers = response.headers;
    let body = decode_body(&mut headers, response.body)?;
    Ok(Fetched {
        status: response.status,
        headers,
        body,
        final_url: response.final_url,
    })
}

fn push_missing(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if header_missing(headers, name) {
        headers.push((name.into(), value));
    }
}

fn header_missing(headers: &[(String, String)], name: &str) -> bool {
    !headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case(name))
}

pub fn with_query(url: &str, key: &str, value: &str) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(found, _)| found != key)
        .map(|(found, item)| (found.into_owned(), item.into_owned()))
        .chain(std::iter::once((key.to_string(), value.to_string())))
        .collect();
    {
        let mut query = parsed.query_pairs_mut();
        query.clear();
        query.extend_pairs(&pairs);
    }
    Some(parsed.to_string())
}
