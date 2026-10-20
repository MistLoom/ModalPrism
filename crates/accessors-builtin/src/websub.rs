//! WebSub 订阅与回调。监听套接字由组合根持有。

use std::time::Instant;

use accessors::{AccessContext, AccessError, HttpRequest};
use bytes::Bytes;
use strata_model::{Locator, RawEnvelope};

use crate::envelope::{Page, from_page};
use crate::ready::{permanent, ready};

pub async fn subscribe(
    ctx: &AccessContext,
    deadline: Instant,
    hub: &str,
    topic: &str,
    callback: &str,
) -> Result<(), AccessError> {
    ready(ctx, deadline)?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("hub.mode", "subscribe")
        .append_pair("hub.topic", topic)
        .append_pair("hub.callback", callback)
        .finish();
    let response = ctx
        .egress
        .request(HttpRequest {
            method: "POST".into(),
            url: hub.to_string(),
            headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: Bytes::from(body),
        })
        .await?;
    if (200..300).contains(&response.status) {
        Ok(())
    } else {
        Err(permanent("hub"))
    }
}

pub struct Challenge {
    pub status: u16,
    pub body: String,
}

pub fn verify(query: &str, topic: &str) -> Challenge {
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let mode = pair(&pairs, "hub.mode");
    let found = pair(&pairs, "hub.topic");
    let challenge = pair(&pairs, "hub.challenge");
    if matches!(mode, Some("subscribe" | "unsubscribe")) && found == Some(topic) {
        Challenge {
            status: 200,
            body: challenge.unwrap_or("").to_string(),
        }
    } else {
        Challenge {
            status: 404,
            body: String::new(),
        }
    }
}

fn pair<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

pub struct Notice<'a> {
    pub topic: &'a str,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub policy: &'a serde_json::Value,
}

pub fn notify(ctx: &AccessContext, notice: Notice<'_>) -> Result<RawEnvelope, AccessError> {
    if ctx.is_cancelled() {
        return Err(AccessError::cancelled());
    }
    let started = Instant::now();
    let mut envelopes = vec![from_page(
        ctx,
        notice.policy,
        started,
        Page {
            locator: Locator::Url {
                url: notice.topic.to_string(),
            },
            final_url: notice.topic.to_string(),
            status: 200,
            headers: notice.headers,
            body: notice.body,
            cursor: None,
            receipt_id: None,
            render_level: None,
            engine: None,
            extra: Vec::new(),
            document: true,
        },
    )?];
    envelopes.pop().ok_or_else(|| permanent("id"))
}
