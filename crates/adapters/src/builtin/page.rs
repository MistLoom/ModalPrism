//! 从详情页抽出标题、摘要和发布时间。结果写入 `struct:page`。

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::{Map, Value};
use strata_model::FacetKind;

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, ContentOutput, DraftBody, FacetDraft, InputSelector,
    OutputDecl,
};
use crate::textutil::{capture_pattern, collapse_ws, first_capture, locator_text, parse_published};
use crate::traits::ContentAdapter;

pub struct PageFields {
    meta: AdapterMeta,
}

impl Default for PageFields {
    fn default() -> Self {
        Self::new()
    }
}

impl PageFields {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/page-fields".into(),
                version: "1".into(),
                role: AdapterRole::Content,
                consumes: vec![InputSelector {
                    kind: Some("raw".into()),
                    variant: None,
                    part_role: Some("document".into()),
                    media_type: Some("text/html".into()),
                }],
                produces: vec![OutputDecl::Facet {
                    kind: "struct".into(),
                    variant: Some("page".into()),
                }],
                deterministic: true,
            },
        }
    }
}

#[async_trait]
impl ContentAdapter for PageFields {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        let parsed = page_spec(spec)?;
        if parsed.published.is_none()
            && parsed.title.is_none()
            && parsed.summary.is_none()
            && parsed.published_regex.is_none()
        {
            return Err(AdapterError::config("缺少字段"));
        }
        Ok(())
    }

    async fn adapt(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        let parsed = page_spec(spec)?;
        let html = String::from_utf8_lossy(&input.view.bytes);
        let document = Html::parse_document(&html);
        let mut value = Map::new();
        if let Some(title) = parsed
            .title
            .as_deref()
            .and_then(|selector| node_text(&document, selector))
        {
            value.insert("title".into(), Value::String(title));
        }
        if let Some(summary) = parsed
            .summary
            .as_deref()
            .and_then(|selector| node_text(&document, selector))
        {
            value.insert("summary".into(), Value::String(summary));
        }
        let published = published_on(&document, &html, &parsed);
        if let Some(published) = published {
            if let Ok(stamp) = serde_json::to_value(published) {
                value.insert("publishedAt".into(), stamp);
            }
        }
        if value.is_empty() {
            return Ok(ContentOutput {
                facets: Vec::new(),
                links: Vec::new(),
                feedback: None,
            });
        }
        value.insert(
            "url".into(),
            Value::String(locator_text(&input.locator).to_string()),
        );
        Ok(ContentOutput {
            facets: vec![FacetDraft {
                kind: FacetKind::Struct,
                variant: Some("page".into()),
                body: DraftBody::Struct(Value::Object(value)),
                media_type: Some("application/json".into()),
                derived_from: Vec::new(),
            }],
            links: Vec::new(),
            feedback: None,
        })
    }
}

struct PageSpec {
    published: Option<String>,
    published_attr: Option<String>,
    published_regex: Option<String>,
    offset: Option<String>,
    title: Option<String>,
    summary: Option<String>,
}

fn page_spec(spec: &Value) -> Result<PageSpec, AdapterError> {
    let published = match spec.get("publishedAt") {
        None => None,
        Some(Value::String(selector)) => Some(selector.clone()),
        _ => return Err(AdapterError::config("publishedAt")),
    };
    if let Some(selector) = &published {
        must_selector(selector)?;
    }
    if let Some(selector) = optional_string(spec, "title") {
        must_selector(&selector)?;
    }
    if let Some(selector) = optional_string(spec, "summary") {
        must_selector(&selector)?;
    }
    if let Some(offset) = optional_string(spec, "publishedAtUtcOffset") {
        if !offset_ok(&offset) {
            return Err(AdapterError::config("publishedAtUtcOffset"));
        }
    }
    if let Some(pattern) = optional_string(spec, "publishedAtRegex") {
        capture_pattern(&pattern).map_err(|_| AdapterError::config("publishedAtRegex"))?;
    }
    Ok(PageSpec {
        published,
        published_attr: optional_string(spec, "publishedAtAttr"),
        published_regex: optional_string(spec, "publishedAtRegex"),
        offset: optional_string(spec, "publishedAtUtcOffset"),
        title: optional_string(spec, "title"),
        summary: optional_string(spec, "summary"),
    })
}

fn published_on(document: &Html, html: &str, spec: &PageSpec) -> Option<strata_model::Timestamp> {
    if let Some(selector) = &spec.published {
        if let Some(raw) = node_raw(document, selector, spec.published_attr.as_deref()) {
            if let Some(parsed) = parse_published(&raw, spec.offset.as_deref(), None) {
                return Some(parsed);
            }
        }
    }
    for key in [
        "article:published_time",
        "og:published_time",
        "article:modified_time",
    ] {
        if let Some(raw) = meta_content(document, "property", key) {
            if let Some(parsed) = parse_published(&raw, spec.offset.as_deref(), None) {
                return Some(parsed);
            }
        }
    }
    for key in ["pubdate", "publishdate", "date"] {
        if let Some(raw) = meta_content(document, "name", key) {
            if let Some(parsed) = parse_published(&raw, spec.offset.as_deref(), None) {
                return Some(parsed);
            }
        }
    }
    if let Some(raw) = json_ld_published(html) {
        if let Some(parsed) = parse_published(&raw, spec.offset.as_deref(), None) {
            return Some(parsed);
        }
    }
    if let Some(pattern) = spec.published_regex.as_deref() {
        let raw = first_capture(pattern, html)?;
        return parse_published(&raw, spec.offset.as_deref(), None);
    }
    None
}

fn meta_content(document: &Html, attr: &str, key: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[{attr}=\"{key}\"]")).ok()?;
    document
        .select(&selector)
        .next()?
        .attr("content")
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn json_ld_published(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("script[type='application/ld+json']").ok()?;
    for element in document.select(&selector) {
        let text = element.text().collect::<String>();
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(raw) = json_ld_date_value(&value) {
            return Some(raw);
        }
    }
    None
}

fn json_ld_date_value(value: &Value) -> Option<String> {
    for key in ["datePublished", "dateCreated", "uploadDate"] {
        if let Some(raw) = value.get(key).and_then(Value::as_str) {
            if !raw.trim().is_empty() {
                return Some(raw.trim().to_string());
            }
        }
    }
    if let Some(items) = value.get("@graph").and_then(Value::as_array) {
        for item in items {
            if let Some(raw) = json_ld_date_value(item) {
                return Some(raw);
            }
        }
    }
    if let Some(item) = value.get("mainEntity") {
        if let Some(raw) = json_ld_date_value(item) {
            return Some(raw);
        }
    }
    None
}

fn node_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let text = collapse_ws(
        &document
            .select(&selector)
            .next()?
            .text()
            .collect::<String>(),
    );
    if text.is_empty() { None } else { Some(text) }
}

fn node_raw(document: &Html, selector: &str, attr: Option<&str>) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    let node = document.select(&selector).next()?;
    if let Some(attr) = attr {
        return node
            .attr(attr)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string);
    }
    for key in ["datetime", "content", "title"] {
        if let Some(raw) = node.attr(key) {
            let raw = raw.trim();
            if !raw.is_empty() {
                return Some(raw.to_string());
            }
        }
    }
    let text = collapse_ws(&node.text().collect::<String>());
    if text.is_empty() { None } else { Some(text) }
}

fn must_selector(raw: &str) -> Result<Selector, AdapterError> {
    Selector::parse(raw).map_err(|error| AdapterError::config(error.to_string()))
}

fn optional_string(spec: &Value, key: &str) -> Option<String> {
    spec.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn offset_ok(offset: &str) -> bool {
    let bytes = offset.as_bytes();
    bytes.len() == 6
        && (bytes[0] == b'+' || bytes[0] == b'-')
        && bytes[3] == b':'
        && bytes[1..3].iter().all(u8::is_ascii_digit)
        && bytes[4..6].iter().all(u8::is_ascii_digit)
}
