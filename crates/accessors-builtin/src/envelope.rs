//! 把一页字节收成原始信封。

use std::time::Instant;

use accessors::{AccessContext, AccessError, redact_envelope};
use bytes::Bytes;
use serde_json::{Map, Value};
use strata_model::{
    EngineInfo, EnvelopeId, Locator, PartId, PartRole, RawEnvelope, RawPart, ReceiptId,
    RenderLevel, Timestamp, Transport,
};

use crate::headers::{media_type, signals};
use crate::ready::permanent;

pub struct Page {
    pub locator: Locator,
    pub final_url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub cursor: Option<Value>,
    pub receipt_id: Option<ReceiptId>,
    pub render_level: Option<RenderLevel>,
    pub engine: Option<EngineInfo>,
    pub extra: Vec<ExtraPart>,
    pub document: bool,
}

pub struct ExtraPart {
    pub role: PartRole,
    pub media_type: String,
    pub body: Bytes,
    pub locator: Option<Locator>,
    pub meta: Map<String, Value>,
}

pub fn from_page(
    ctx: &AccessContext,
    policy: &Value,
    started: Instant,
    page: Page,
) -> Result<RawEnvelope, AccessError> {
    let (kind, charset) = media_type(&page.headers);
    let mut parts = Vec::new();
    if page.document && page.status != 304 {
        parts.push(part(
            ctx,
            PartRole::Document,
            kind,
            charset,
            None,
            &page.body,
            Map::new(),
        )?);
    }
    for extra in page.extra {
        parts.push(part(
            ctx,
            extra.role,
            extra.media_type,
            None,
            extra.locator,
            &extra.body,
            extra.meta,
        )?);
    }
    let page_signals = signals(&page.headers);
    let protocol = url::Url::parse(&page.final_url)
        .ok()
        .map(|url| url.scheme().to_string())
        .filter(|scheme| !scheme.is_empty())
        .unwrap_or_else(|| "http".into());
    let mut envelope = RawEnvelope {
        envelope_id: EnvelopeId::new(fresh_id("env")).map_err(|_| permanent("id"))?,
        locator: page.locator,
        final_locator: Some(Locator::Url {
            url: page.final_url,
        }),
        transport: Transport {
            protocol,
            status: Some(page.status),
            headers: page.headers,
            started_at: Timestamp::now(),
            duration: started.elapsed(),
            bytes_in: u64::try_from(page.body.len()).unwrap_or(u64::MAX),
            not_modified: page.status == 304,
            render_level: page.render_level,
            engine: page.engine,
            receipt_id: page.receipt_id,
        },
        parts,
        cursor_candidate: page.cursor,
        signals: page_signals,
    };
    redact_envelope(&mut envelope, policy);
    Ok(envelope)
}

pub fn part(
    ctx: &AccessContext,
    role: PartRole,
    media_type: String,
    charset: Option<String>,
    locator: Option<Locator>,
    body: &[u8],
    meta: Map<String, Value>,
) -> Result<RawPart, AccessError> {
    Ok(RawPart {
        part_id: PartId::new(fresh_id("part")).map_err(|_| permanent("id"))?,
        role,
        media_type,
        charset,
        locator,
        bytes: ctx.staging.put(body)?,
        meta,
    })
}

fn fresh_id(prefix: &str) -> String {
    format!("{prefix}_{}", ulid::Ulid::new())
}
