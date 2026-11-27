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
        /// JSON Pointer。指向的数组短于 step 时，这一页就是上游的末页，游标清空。
        /// 缺省时保持只按步长前进，兼容已有声明。
        #[serde(default)]
        items: String,
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
        let deadline = request.deadline;
        let mut out = Vec::new();
        self.access_batched(
            request,
            ctx,
            &mut accessors::CollectingBatches { out: &mut out },
        )
        .await?;
        if out.is_empty() {
            ready(ctx, deadline)?;
        }
        Ok(out)
    }

    async fn access_batched(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
        batches: &mut (dyn accessors::BatchHandler + Send),
    ) -> Result<(), AccessError> {
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
                    body: render_body(
                        spec.body.as_deref().unwrap_or(""),
                        &spec.pagination,
                        offset,
                        token.as_deref(),
                    ),
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
            // 每页即建信封即推送：回调返回后本页内存即可丢弃，驻留不超过一页 + 缓冲。
            let envelope = from_page(
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
            )?;
            let page_cursor = envelope.cursor_candidate.clone();
            batches.batch(vec![envelope], page_cursor).await?;
            if stop {
                break;
            }
            if let Some(next) = cursor.get("url").and_then(Value::as_str) {
                url = next.to_string();
            }
        }
        Ok(())
    }
}

/// 把分页位置写进 POST/GET 正文。只替换约定占位符，其余字节保持原样。
fn render_body(
    template: &str,
    pagination: &Option<Pagination>,
    offset: Option<u64>,
    token: Option<&str>,
) -> Bytes {
    let mut body = template.to_string();
    if let Some(Pagination::Offset { start, step, .. }) = pagination {
        let value = offset.unwrap_or(*start);
        body = body.replace("{{offset}}", &value.to_string());
        body = body.replace("{{limit}}", &step.to_string());
    }
    if let Some(token) = token {
        body = body.replace("{{token}}", token);
    }
    Bytes::from(body)
}

/// `items` 为 JSON Pointer（以 `/` 开头）或 `tag:` 后的 XML 标签名。
/// 本页条数短于步长时视为上游已到末页。
fn page_short(body: &[u8], items: &str, step: u64) -> bool {
    let count = if let Some(tag) = items.strip_prefix("tag:") {
        count_open_tag(body, tag.trim())
    } else {
        serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|value| value.pointer(items).and_then(Value::as_array).map(Vec::len))
            .unwrap_or(0)
    };
    (count as u64) < step
}

fn count_open_tag(body: &[u8], tag: &str) -> usize {
    if tag.is_empty() {
        return 0;
    }
    let text = String::from_utf8_lossy(body);
    let needle = format!("<{tag}");
    text.matches(&needle).count()
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
        Some(Pagination::Offset { start, step, items, .. }) => {
            let current = offset.unwrap_or(*start);
            if !items.is_empty() && page_short(body, items, *step) {
                return Value::Null;
            }
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
