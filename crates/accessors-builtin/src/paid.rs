//! `paid-api` 访问器。分页规则与 `api` 相同，传输走付费端口。

use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
    PaidCall, PaidCapability,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Pagination, next_cursor};
use crate::envelope::{Page, from_page};
use crate::headers::is_http;
use crate::ready::{config, ready, spec_error};

pub struct PaidApiAccessor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    service: String,
    #[serde(default = "get_method")]
    method: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default = "empty_object")]
    params: Value,
    #[serde(default = "one_page")]
    pages: u32,
    #[serde(default)]
    pagination: Option<Pagination>,
}

fn get_method() -> String {
    "GET".into()
}

fn one_page() -> u32 {
    1
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = serde_json::from_value(spec.clone()).map_err(spec_error)?;
    if spec.service.is_empty() || !(1..=50).contains(&spec.pages) || !spec.params.is_object() {
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
        strata_model::Locator::ApiEndpoint { endpoint } if !endpoint.is_empty() => {
            Ok(endpoint.clone())
        }
        _ => Err(config("locator")),
    }
}

fn params_for(spec: &Spec, url: &str, offset: Option<u64>, token: Option<&str>) -> Value {
    let mut params = spec.params.clone();
    if spec.method != "GET" {
        params["method"] = json!(spec.method);
    }
    match &spec.pagination {
        Some(Pagination::Offset { param, start, .. }) => {
            params[param] = json!(offset.unwrap_or(*start));
        }
        Some(Pagination::Cursor { param, .. }) => {
            if let Some(token) = token {
                params[param] = json!(token);
            }
        }
        Some(Pagination::Link {}) => {
            params["url"] = json!(url);
        }
        None => {}
    }
    params
}

#[async_trait]
impl Accessor for PaidApiAccessor {
    fn id(&self) -> &str {
        "paid-api"
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
        let endpoint = endpoint(&request.locator)?;
        let mut url = request
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.get("url"))
            .and_then(Value::as_str)
            .filter(|item| is_http(item))
            .unwrap_or(&endpoint)
            .to_string();
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
            let requested = url.clone();
            let params = params_for(&spec, &url, offset, token.as_deref());
            let started = Instant::now();
            let response = ctx
                .paid
                .call(PaidCall {
                    service: spec.service.clone(),
                    endpoint: endpoint.clone(),
                    params,
                    body: Bytes::from(spec.body.clone().unwrap_or_default()),
                })
                .await?;
            let cursor = next_cursor(
                &spec.pagination,
                &response.headers,
                &response.body,
                &url,
                offset,
            );
            if let Some(value) = cursor.get("offset").and_then(Value::as_u64) {
                offset = Some(value);
            }
            if let Some(value) = cursor.get("token").and_then(Value::as_str) {
                token = Some(value.to_string());
            }
            let stop = cursor.is_null() || !(200..300).contains(&response.status);
            if let Some(next) = cursor.get("url").and_then(Value::as_str) {
                url = next.to_string();
            }
            out.push(from_page(
                ctx,
                &request.policy,
                started,
                Page {
                    locator: request.locator.clone(),
                    final_url: requested,
                    status: response.status,
                    headers: response.headers,
                    body: response.body,
                    cursor: if cursor.is_null() { None } else { Some(cursor) },
                    receipt_id: response.receipt_id,
                    render_level: None,
                    engine: None,
                    extra: Vec::new(),
                    document: true,
                },
            )?);
            if stop {
                break;
            }
        }
        if out.is_empty() {
            ready(ctx, request.deadline)?;
        }
        Ok(out)
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
        paid: Some(PaidCapability {
            service: "spec".into(),
        }),
        batchable: true,
        inbound: false,
    }
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(caps);
