//! `api` 访问器。按 spec 的分页模板逐页取原文。

use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::envelope::{Page, from_page};
use crate::headers::{is_http, link_next, resolve_url};
use crate::outbound::{Outbound, send, with_query};
use crate::ready::{config, ready, spec_error};

pub struct ApiAccessor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Spec {
    #[serde(default = "get_method")]
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) headers: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default = "one_page")]
    pub(crate) pages: u32,
    #[serde(default)]
    pub(crate) pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Pagination {
    Offset {
        param: String,
        #[serde(default)]
        start: u64,
        step: u64,
    },
    Cursor {
        param: String,
        pointer: String,
    },
    Link {},
}

fn get_method() -> String {
    "GET".into()
}

fn one_page() -> u32 {
    1
}

pub(crate) fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = if spec.is_null() {
        Spec {
            method: get_method(),
            headers: Vec::new(),
            body: None,
            session: None,
            pages: 1,
            pagination: None,
        }
    } else {
        serde_json::from_value(spec.clone()).map_err(spec_error)?
    };
    if !(1..=50).contains(&spec.pages) {
        return Err(config("spec 无效"));
    }
    if let Some(Pagination::Offset { step, .. }) = &spec.pagination {
        if *step == 0 {
            return Err(config("spec 无效"));
        }
    }
    Ok(spec)
}

fn endpoint(locator: &strata_model::Locator) -> Result<String, AccessError> {
    match locator {
        strata_model::Locator::ApiEndpoint { endpoint } if is_http(endpoint) => {
            Ok(endpoint.clone())
        }
        _ => Err(config("locator")),
    }
}

#[async_trait]
impl Accessor for ApiAccessor {
    fn id(&self) -> &str {
        "api"
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
        let mut url = endpoint(&request.locator)?;
        if let Some(resume) = request
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.get("url"))
            .and_then(Value::as_str)
        {
            if matches!(spec.pagination, Some(Pagination::Link {})) && is_http(resume) {
                url = resume.to_string();
            }
        }
        let mut offset = request
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.get("offset"))
            .and_then(Value::as_u64);
        let mut token = request
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.get("token"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut out = Vec::new();
        for _ in 0..spec.pages {
            if ctx.is_cancelled() {
                return Err(AccessError::cancelled());
            }
            if Instant::now() >= request.deadline {
                break;
            }
            let page_url = page_url(&url, &spec.pagination, offset, token.as_deref())?;
            let started = Instant::now();
            let fetched = send(
                ctx,
                request.deadline,
                Outbound {
                    method: spec.method.clone(),
                    url: page_url,
                    headers: spec.headers.clone(),
                    body: Bytes::from(spec.body.clone().unwrap_or_default()),
                    session: spec.session.clone(),
                    hints: request.hints.clone(),
                },
            )
            .await?;
            let status = fetched.status;
            let cursor = next_cursor(
                &spec.pagination,
                &fetched.headers,
                &fetched.body,
                &fetched.final_url,
                offset,
            );
            if let Some(value) = cursor.get("offset").and_then(Value::as_u64) {
                offset = Some(value);
            }
            if let Some(value) = cursor.get("token").and_then(Value::as_str) {
                token = Some(value.to_string());
            }
            let stop = cursor.is_null() || !(200..300).contains(&status);
            out.push(from_page(
                ctx,
                &request.policy,
                started,
                Page {
                    locator: request.locator.clone(),
                    final_url: fetched.final_url,
                    status: fetched.status,
                    headers: fetched.headers,
                    body: fetched.body,
                    cursor: if cursor.is_null() {
                        None
                    } else {
                        Some(cursor.clone())
                    },
                    receipt_id: None,
                    render_level: Some(if spec.session.is_some() {
                        strata_model::RenderLevel::L1
                    } else {
                        strata_model::RenderLevel::L0
                    }),
                    engine: None,
                    extra: Vec::new(),
                    document: true,
                },
            )?);
            if stop {
                break;
            }
            if let Some(next) = cursor.get("url").and_then(Value::as_str) {
                url = next.to_string();
            }
        }
        if out.is_empty() {
            ready(ctx, request.deadline)?;
        }
        Ok(out)
    }
}

fn page_url(
    url: &str,
    pagination: &Option<Pagination>,
    offset: Option<u64>,
    token: Option<&str>,
) -> Result<String, AccessError> {
    match pagination {
        Some(Pagination::Offset { param, start, .. }) => {
            let value = offset.unwrap_or(*start);
            with_query(url, param, &value.to_string()).ok_or_else(|| config("locator"))
        }
        Some(Pagination::Cursor { param, .. }) => match token {
            Some(token) => with_query(url, param, token).ok_or_else(|| config("locator")),
            None => Ok(url.to_string()),
        },
        _ => Ok(url.to_string()),
    }
}

pub(crate) fn next_cursor(
    pagination: &Option<Pagination>,
    headers: &[(String, String)],
    body: &[u8],
    final_url: &str,
    offset: Option<u64>,
) -> Value {
    match pagination {
        Some(Pagination::Offset { start, step, .. }) => {
            let current = offset.unwrap_or(*start);
            json!({ "offset": current.saturating_add(*step) })
        }
        Some(Pagination::Cursor { pointer, .. }) => {
            let Ok(value) = serde_json::from_slice::<Value>(body) else {
                return Value::Null;
            };
            match value.pointer(pointer).and_then(Value::as_str) {
                Some(token) => json!({ "token": token }),
                None => Value::Null,
            }
        }
        Some(Pagination::Link {}) => {
            match link_next(headers).and_then(|href| resolve_url(final_url, &href)) {
                Some(url) if is_http(&url) => json!({ "url": url }),
                _ => Value::Null,
            }
        }
        None => Value::Null,
    }
}

fn caps() -> AccessorCapabilities {
    AccessorCapabilities {
        protocols: vec!["http".into(), "https".into()],
        conditional: true,
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
