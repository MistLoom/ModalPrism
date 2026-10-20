//! `http` 访问器。一次请求一个信封。

use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::Value;
use strata_model::{Locator, RenderLevel};

use crate::envelope::{Page, from_page};
use crate::headers::is_http;
use crate::outbound::{Outbound, send};
use crate::ready::{config, ready, spec_error};

pub struct HttpAccessor;

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
}

fn get_method() -> String {
    "GET".into()
}

fn capabilities() -> AccessorCapabilities {
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

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    if spec.is_null() {
        return Ok(Spec {
            method: get_method(),
            headers: Vec::new(),
            body: None,
            session: None,
        });
    }
    serde_json::from_value(spec.clone()).map_err(spec_error)
}

pub fn url_of(locator: &Locator) -> Result<String, AccessError> {
    match locator {
        Locator::Url { url } if is_http(url) => Ok(url.clone()),
        _ => Err(config("locator")),
    }
}

pub async fn fetch_page(
    request: &AccessRequest,
    ctx: &AccessContext,
    spec: &Spec,
    url: String,
) -> Result<Page, AccessError> {
    let fetched = send(
        ctx,
        request.deadline,
        Outbound {
            method: spec.method.clone(),
            url,
            headers: spec.headers.clone(),
            body: Bytes::from(spec.body.clone().unwrap_or_default()),
            session: spec.session.clone(),
            hints: request.hints.clone(),
        },
    )
    .await?;
    Ok(Page {
        locator: request.locator.clone(),
        final_url: fetched.final_url,
        status: fetched.status,
        headers: fetched.headers,
        body: fetched.body,
        cursor: None,
        receipt_id: None,
        render_level: Some(if spec.session.is_some() {
            RenderLevel::L1
        } else {
            RenderLevel::L0
        }),
        engine: None,
        extra: Vec::new(),
        document: true,
    })
}

#[async_trait]
impl Accessor for HttpAccessor {
    fn id(&self) -> &str {
        "http"
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
        let url = url_of(&request.locator)?;
        let started = Instant::now();
        let page = fetch_page(&request, ctx, &spec, url).await?;
        Ok(vec![from_page(ctx, &request.policy, started, page)?])
    }
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(capabilities);
