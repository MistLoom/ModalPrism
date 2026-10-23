//! `search` 访问器。把查询串放进端点后取回原文。

use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::Value;
use strata_model::Locator;

use crate::envelope::{Page, from_page};
use crate::headers::is_http;
use crate::outbound::{Outbound, send};
use crate::ready::{config, ready, spec_error};

pub struct SearchAccessor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    endpoint: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    session: Option<String>,
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = serde_json::from_value(spec.clone()).map_err(spec_error)?;
    let Some((left, right)) = spec.endpoint.split_once("{query}") else {
        return Err(config("spec 无效"));
    };
    if !is_http(&spec.endpoint.replace("{query}", "q")) && !is_http(&format!("{left}q{right}")) {
        return Err(config("spec 无效"));
    }
    Ok(spec)
}

fn encode_query(query: &str) -> String {
    let mut out = String::new();
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte))
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[async_trait]
impl Accessor for SearchAccessor {
    fn id(&self) -> &str {
        "search"
    }

    fn version(&self) -> &str {
        "1"
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        &CAPS
    }

    fn validate(&self, spec: &Value) -> Result<(), AccessError> {
        parse_spec(spec).map(|_| ())
    }

    async fn access(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
    ) -> Result<Vec<strata_model::RawEnvelope>, AccessError> {
        ready(ctx, request.deadline)?;
        let spec = parse_spec(&request.spec)?;
        let Locator::Query { query } = &request.locator else {
            return Err(config("locator"));
        };
        let url = spec.endpoint.replacen("{query}", &encode_query(query), 1);
        if !is_http(&url) {
            return Err(config("spec 无效"));
        }
        let started = Instant::now();
        let fetched = send(
            ctx,
            request.deadline,
            Outbound {
                method: "GET".into(),
                url,
                headers: spec.headers,
                body: Bytes::new(),
                session: spec.session,
                hints: request.hints.clone(),
            },
        )
        .await?;
        Ok(vec![from_page(
            ctx,
            &request.policy,
            started,
            Page {
                locator: request.locator.clone(),
                final_url: fetched.final_url,
                status: fetched.status,
                headers: fetched.headers,
                body: fetched.body,
                cursor: None,
                receipt_id: None,
                render_level: Some(strata_model::RenderLevel::L0),
                engine: None,
                extra: Vec::new(),
                document: true,
            },
        )?])
    }
}

fn caps() -> AccessorCapabilities {
    AccessorCapabilities {
        protocols: vec!["http".into(), "https".into()],
        conditional: false,
        render: false,
        layout: false,
        capture_network: CaptureNetwork::None,
        interact: false,
        paid: None,
        batchable: false,
        inbound: false,
    }
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(caps);
