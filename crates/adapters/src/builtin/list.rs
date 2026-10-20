//! HTML 列表与 JSON 列表。

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::Value;
use strata_model::{Locator, RenderLevel};

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, DiscoverOutput, Discovery, DiscoveryHints, Feedback,
    Follow, InputSelector, OutputDecl, Quality,
};
use crate::textutil::{absolute_http, collapse_ws, locator_text, parse_timestamp, url_identity};
use crate::traits::SearchAdapter;

pub struct HtmlList {
    meta: AdapterMeta,
}

impl Default for HtmlList {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlList {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/html-list".into(),
                version: "1".into(),
                role: AdapterRole::Search,
                consumes: vec![InputSelector {
                    kind: Some("raw".into()),
                    variant: None,
                    part_role: Some("document".into()),
                    media_type: Some("text/html".into()),
                }],
                produces: vec![OutputDecl::Discovery],
                deterministic: true,
            },
        }
    }
}

pub struct JsonList {
    meta: AdapterMeta,
}

impl Default for JsonList {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonList {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/json-list".into(),
                version: "1".into(),
                role: AdapterRole::Search,
                consumes: vec![InputSelector {
                    kind: Some("raw".into()),
                    variant: None,
                    part_role: None,
                    media_type: Some("application/json".into()),
                }],
                produces: vec![OutputDecl::Discovery],
                deterministic: true,
            },
        }
    }
}

struct HtmlSpec {
    item: String,
    link: String,
    title: Option<String>,
    summary: Option<String>,
    published_selector: Option<String>,
    published_attr: Option<String>,
}

#[async_trait]
impl SearchAdapter for HtmlList {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        let parsed = html_spec(spec)?;
        Selector::parse(&parsed.item).map_err(|error| AdapterError::config(error.to_string()))?;
        Selector::parse(&parsed.link).map_err(|error| AdapterError::config(error.to_string()))?;
        parse_optional(&parsed.title)?;
        parse_optional(&parsed.summary)?;
        parse_optional(&parsed.published_selector)?;
        Ok(())
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let parsed = html_spec(spec)?;
        let item_sel = must_selector(&parsed.item)?;
        let link_sel = must_selector(&parsed.link)?;
        let title_sel = optional_selector(parsed.title.as_deref())?;
        let summary_sel = optional_selector(parsed.summary.as_deref())?;
        let published_sel = optional_selector(parsed.published_selector.as_deref())?;
        let html = String::from_utf8_lossy(&input.view.bytes);
        let document = Html::parse_document(&html);
        let base = locator_text(&input.locator);
        let mut items = Vec::new();
        for element in document.select(&item_sel) {
            let Some(link_el) = element.select(&link_sel).next() else {
                continue;
            };
            let Some(href) = link_el.attr("href") else {
                continue;
            };
            let Some(absolute) = absolute_http(base, href) else {
                continue;
            };
            let Some(identity) = url_identity(&absolute) else {
                continue;
            };
            let title = title_sel
                .as_ref()
                .and_then(|selector| element.select(selector).next())
                .map(|node| collapse_ws(&node.text().collect::<String>()))
                .filter(|text| !text.is_empty());
            let summary = summary_sel
                .as_ref()
                .and_then(|selector| element.select(selector).next())
                .map(|node| collapse_ws(&node.text().collect::<String>()))
                .filter(|text| !text.is_empty());
            let published = published_sel.as_ref().and_then(|selector| {
                let node = element.select(selector).next()?;
                let raw = parsed
                    .published_attr
                    .as_deref()
                    .and_then(|attr| node.attr(attr))
                    .map(str::to_string)
                    .unwrap_or_else(|| node.text().collect());
                parse_timestamp(&raw)
            });
            items.push(Discovery {
                locator: Locator::Url { url: absolute },
                identity: vec![identity],
                hints: DiscoveryHints {
                    title,
                    published_at: published,
                    author: None,
                    summary,
                    rank: None,
                    extra: Default::default(),
                },
                span: None,
                follow: follow_yes(input),
                materialize: Vec::new(),
            });
        }
        number_ranks(&mut items);
        Ok(DiscoverOutput {
            feedback: if items.is_empty() {
                Some(Feedback {
                    needs_render: Some(RenderLevel::L2),
                    quality: Quality::Empty,
                    missing: vec!["item".into()],
                })
            } else {
                None
            },
            items,
        })
    }
}

#[async_trait]
impl SearchAdapter for JsonList {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        let items = required_string(spec, "items")?;
        if !items.starts_with('/') {
            return Err(AdapterError::config("items 必须是 JSON Pointer"));
        }
        required_string(spec, "url")?;
        Ok(())
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let pointer = required_string(spec, "items")?;
        let url_key = required_string(spec, "url")?;
        let title_key = optional_string(spec, "title");
        let summary_key = optional_string(spec, "summary");
        let published_key = optional_string(spec, "publishedAt");
        let value: Value = serde_json::from_slice(&input.view.bytes).unwrap_or(Value::Null);
        let mut items = Vec::new();
        if let Some(entries) = value.pointer(&pointer).and_then(Value::as_array) {
            for entry in entries {
                let Some(url) = entry.get(&url_key).and_then(Value::as_str) else {
                    continue;
                };
                let Some(absolute) = absolute_http(locator_text(&input.locator), url) else {
                    continue;
                };
                let Some(identity) = url_identity(&absolute) else {
                    continue;
                };
                items.push(Discovery {
                    locator: Locator::Url { url: absolute },
                    identity: vec![identity],
                    hints: DiscoveryHints {
                        title: title_key
                            .as_deref()
                            .and_then(|key| entry.get(key))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        published_at: published_key
                            .as_deref()
                            .and_then(|key| entry.get(key))
                            .and_then(Value::as_str)
                            .and_then(parse_timestamp),
                        author: None,
                        summary: summary_key
                            .as_deref()
                            .and_then(|key| entry.get(key))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        rank: None,
                        extra: Default::default(),
                    },
                    span: None,
                    follow: follow_yes(input),
                    materialize: Vec::new(),
                });
            }
        }
        number_ranks(&mut items);
        Ok(DiscoverOutput {
            feedback: if items.is_empty() {
                Some(Feedback {
                    needs_render: None,
                    quality: Quality::Empty,
                    missing: vec!["item".into()],
                })
            } else {
                None
            },
            items,
        })
    }
}

fn html_spec(spec: &Value) -> Result<HtmlSpec, AdapterError> {
    let published = spec.get("publishedAt");
    let (published_selector, published_attr) = match published {
        None => (None, None),
        Some(Value::String(selector)) => (Some(selector.clone()), None),
        Some(Value::Object(_)) => (
            Some(required_string(published.unwrap(), "selector")?),
            optional_string(published.unwrap(), "attr"),
        ),
        Some(_) => return Err(AdapterError::config("publishedAt 类型无效")),
    };
    Ok(HtmlSpec {
        item: required_string(spec, "item")?,
        link: required_string(spec, "link")?,
        title: optional_string(spec, "title"),
        summary: optional_string(spec, "summary"),
        published_selector,
        published_attr,
    })
}

fn required_string(spec: &Value, key: &str) -> Result<String, AdapterError> {
    spec.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AdapterError::config(format!("缺少 {key}")))
}

fn optional_string(spec: &Value, key: &str) -> Option<String> {
    spec.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn must_selector(raw: &str) -> Result<Selector, AdapterError> {
    Selector::parse(raw).map_err(|error| AdapterError::config(error.to_string()))
}

fn optional_selector(raw: Option<&str>) -> Result<Option<Selector>, AdapterError> {
    raw.map(must_selector).transpose()
}

fn parse_optional(raw: &Option<String>) -> Result<(), AdapterError> {
    if let Some(raw) = raw {
        must_selector(raw)?;
    }
    Ok(())
}

fn follow_yes(input: &AdapterInput) -> Follow {
    Follow::Yes {
        accessor: input.source.accessor.clone(),
        render: input.source.render,
        priority: 0,
    }
}

fn number_ranks(items: &mut [Discovery]) {
    for (index, item) in items.iter_mut().enumerate() {
        item.hints.rank = Some((index + 1) as u32);
    }
}
