//! 从 HTML 收集 http(s) 链接。

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::Value;
use strata_model::Locator;

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, DiscoverOutput, Discovery, DiscoveryHints, Follow,
    InputSelector, OutputDecl,
};
use crate::textutil::{absolute_http, collapse_ws, locator_text, url_identity};
use crate::traits::SearchAdapter;

pub struct LinkDiscovery {
    meta: AdapterMeta,
}

impl Default for LinkDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkDiscovery {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/link-discovery".into(),
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

#[async_trait]
impl SearchAdapter for LinkDiscovery {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let html = String::from_utf8_lossy(&input.view.bytes);
        let document = Html::parse_document(&html);
        let selector =
            Selector::parse("a[href]").map_err(|error| AdapterError::config(error.to_string()))?;
        let base = locator_text(&input.locator);
        let mut items = Vec::new();
        let mut seen = Vec::new();
        for element in document.select(&selector) {
            let Some(href) = element.attr("href") else {
                continue;
            };
            let Some(absolute) = absolute_http(base, href) else {
                continue;
            };
            let Some(identity) = url_identity(&absolute) else {
                continue;
            };
            if seen.contains(&identity) {
                continue;
            }
            seen.push(identity.clone());
            let title = collapse_ws(&element.text().collect::<String>());
            items.push(Discovery {
                locator: Locator::Url { url: absolute },
                identity: vec![identity],
                hints: DiscoveryHints {
                    title: if title.is_empty() { None } else { Some(title) },
                    published_at: None,
                    author: None,
                    summary: None,
                    rank: Some((items.len() + 1) as u32),
                    extra: Default::default(),
                },
                span: None,
                follow: Follow::Yes {
                    accessor: input.source.accessor.clone(),
                    accessor_spec: serde_json::json!({}),
                    render: input.source.render,
                    priority: 0,
                    needs: Vec::new(),
                },
                materialize: Vec::new(),
            });
        }
        Ok(DiscoverOutput {
            items,
            feedback: None,
        })
    }
}
