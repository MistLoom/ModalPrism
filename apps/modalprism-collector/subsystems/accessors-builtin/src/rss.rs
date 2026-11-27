//! `rss` 访问器。按页返回 Feed 原文。

use std::collections::BTreeSet;

use accessors::{AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::envelope::from_page;
use crate::headers::{body_next, is_http, link_next, resolve_url};
use crate::http::{HttpAccessor, fetch_page, url_of};
use crate::ready::{ready, spec_error};

pub struct RssAccessor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default = "default_pages")]
    pages: u32,
}

fn default_pages() -> u32 {
    10
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = if spec.is_null() {
        Spec {
            headers: Vec::new(),
            session: None,
            pages: default_pages(),
        }
    } else {
        serde_json::from_value(spec.clone()).map_err(spec_error)?
    };
    if !(1..=50).contains(&spec.pages) {
        return Err(crate::ready::config("spec 无效"));
    }
    Ok(spec)
}

fn http_spec(spec: &Spec) -> crate::http::Spec {
    // 复用 http 的请求字段。method 固定 GET，不带正文。
    crate::http::Spec {
        method: "GET".into(),
        headers: spec.headers.clone(),
        body: None,
        session: spec.session.clone(),
    }
}

#[async_trait]
impl Accessor for RssAccessor {
    fn id(&self) -> &str {
        "rss"
    }

    fn version(&self) -> &str {
        "1"
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        HttpAccessor.capabilities()
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
        let http_spec = http_spec(&spec);
        // 上一轮停在的下一页。没有游标时从 Feed 入口重读，用来接上新条目。
        let mut url = url_of(&request.locator)?;
        if let Some(resume) = request
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.get("url"))
            .and_then(Value::as_str)
            .filter(|item| is_http(item))
        {
            url = resume.to_string();
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for _ in 0..spec.pages {
            if ctx.is_cancelled() {
                return Err(AccessError::cancelled());
            }
            if std::time::Instant::now() >= request.deadline {
                break;
            }
            if !seen.insert(url.clone()) {
                break;
            }
            let mut page = fetch_page(&request, ctx, &http_spec, url.clone()).await?;
            let status = page.status;
            let headers = page.headers.clone();
            let body = page.body.clone();
            let final_url = page.final_url.clone();
            let next = if (200..300).contains(&status) {
                link_next(&headers)
                    .or_else(|| body_next(&body))
                    .and_then(|item| resolve_url(&final_url, &item))
                    .filter(|item| is_http(item))
            } else {
                None
            };
            // 本轮页数用尽时把下一页留给下一次计划，避免每次都从入口重抓同一段。
            page.cursor = next.clone().map(|item| json!({ "url": item }));
            out.push(from_page(
                ctx,
                &request.policy,
                std::time::Instant::now(),
                page,
            )?);
            let Some(next) = next else { break };
            url = next;
        }
        if out.is_empty() {
            ready(ctx, request.deadline)?;
        }
        Ok(out)
    }
}
