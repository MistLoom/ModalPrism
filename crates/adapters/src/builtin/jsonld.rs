//! 收集页面上的 JSON-LD。

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::Value;
use strata_model::FacetKind;

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, ContentOutput, DraftBody, FacetDraft, InputSelector,
    OutputDecl,
};
use crate::traits::ContentAdapter;

pub struct JsonLd {
    meta: AdapterMeta,
}

impl Default for JsonLd {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonLd {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/jsonld-struct".into(),
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
                    variant: Some("jsonld".into()),
                }],
                deterministic: true,
            },
        }
    }
}

#[async_trait]
impl ContentAdapter for JsonLd {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    async fn adapt(
        &self,
        input: &AdapterInput,
        _spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        let html = String::from_utf8_lossy(&input.view.bytes);
        let document = Html::parse_document(&html);
        let selector = Selector::parse("script[type='application/ld+json']")
            .map_err(|error| AdapterError::config(error.to_string()))?;
        let mut items = Vec::new();
        for element in document.select(&selector) {
            let text = element.text().collect::<String>();
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(array) = value.as_array() {
                items.extend(array.iter().cloned());
            } else {
                items.push(value);
            }
        }
        if items.is_empty() {
            return Ok(ContentOutput {
                facets: Vec::new(),
                links: Vec::new(),
                feedback: None,
            });
        }
        Ok(ContentOutput {
            facets: vec![FacetDraft {
                kind: FacetKind::Struct,
                variant: Some("jsonld".into()),
                body: DraftBody::Struct(serde_json::json!({ "items": items })),
                media_type: Some("application/ld+json".into()),
                derived_from: Vec::new(),
            }],
            links: Vec::new(),
            feedback: None,
        })
    }
}
