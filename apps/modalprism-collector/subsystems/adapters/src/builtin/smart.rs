//! 智能适配器：用识别区域完成条目发现与多类型内容拆分。
//!
//! 它们不是人工声明的替代品，而是「没有 spec 时的默认行为」：
//! 结构化响应按字段推断条目，网页按链接密度抽取，识别不到时仍然产出一条指向
//! 文档本身的条目并把质量标成 `Thin`——采集不会因为「没认出来」而空转。

use std::sync::Arc;

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::{Map, Value, json};
use strata_model::{FacetKind, Locator};

use crate::builtin::{PageFields, Readability};
use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, ContentOutput, DiscoverOutput, Discovery,
    DiscoveryHints, DraftBody, FacetDraft, Feedback, Follow, InputSelector, OutputDecl, Probe,
    Quality,
};
use crate::prism::{Prism, prism_input};
use crate::textutil::{
    absolute_http, collapse_ws, locator_text, parse_published, url_identity,
};
use crate::traits::{ContentAdapter, SearchAdapter};

const ALLOWED: &[&str] = &[
    "mode",
    "maxItems",
    "minConfidence",
    "maxRegions",
    "follow",
    "selectors",
];

/// 智能适配器的装配参数。全部可选，缺省即可工作。
struct Options {
    /// `auto` 允许启发式与回退；`strict` 只接受识别出的结构区域。
    mode: String,
    max_items: usize,
    min_confidence: f64,
    max_regions: usize,
    follow: bool,
    selectors: Vec<String>,
}

impl Options {
    fn parse(spec: &Value) -> Result<Self, AdapterError> {
        // Null 与缺省等价：智能适配器必须能空参数直接工作。
        let empty = Map::new();
        let map = match spec {
            Value::Null => &empty,
            Value::Object(map) => map,
            _ => return Err(AdapterError::config("spec 必须是对象")),
        };
        for key in map.keys() {
            if !ALLOWED.contains(&key.as_str()) {
                return Err(AdapterError::config(format!("未知参数: {key}")));
            }
        }
        let mode = match map.get("mode").and_then(Value::as_str) {
            None | Some("auto") => "auto".to_string(),
            Some("strict") => "strict".to_string(),
            Some(_) => return Err(AdapterError::config("mode 只接受 auto 或 strict")),
        };
        let selectors = map
            .get("selectors")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            mode,
            max_items: cap(map.get("maxItems"), 200),
            min_confidence: map
                .get("minConfidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            max_regions: cap(map.get("maxRegions"), 64),
            follow: map.get("follow").and_then(Value::as_bool).unwrap_or(false),
            selectors,
        })
    }

    fn strict(&self) -> bool {
        self.mode == "strict"
    }
}

fn cap(value: Option<&Value>, fallback: usize) -> usize {
    value
        .and_then(Value::as_u64)
        .map(|item| item as usize)
        .filter(|item| *item > 0)
        .unwrap_or(fallback)
}

fn scan_limits(max_regions: usize) -> Value {
    json!({ "max_regions": max_regions.max(16), "time_budget_ms": 2_000 })
}

pub struct SmartDiscover {
    meta: AdapterMeta,
    prism: Option<Arc<dyn Prism>>,
}

impl Default for SmartDiscover {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SmartDiscover {
    pub fn new(prism: Option<Arc<dyn Prism>>) -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/smart-discover".into(),
                version: "1".into(),
                role: AdapterRole::Search,
                // 空 consumes 表示接受任何文档：它就是「没声明时」的默认适配器。
                consumes: Vec::new(),
                produces: vec![OutputDecl::Discovery],
                deterministic: true,
            },
            prism,
        }
    }

    fn context(&self, input: &AdapterInput) -> String {
        locator_text(&input.locator).to_string()
    }

    /// 从一份文档推断条目。识别器可用时优先按结构区域切分，否则按媒体类型启发。
    async fn candidates(
        &self,
        input: &AdapterInput,
        options: &Options,
        base: &str,
    ) -> Vec<Discovery> {
        let text = prism_input(&input.view.bytes);
        let media = input
            .detection
            .as_ref()
            .map(|item| item.media_type.as_str())
            .unwrap_or(input.view.media_type.as_str());
        let structured = media.contains("json")
            || media.contains("csv")
            || media.contains("yaml")
            || text.trim_start().starts_with('{')
            || text.trim_start().starts_with('[');
        // 媒体类型看不出来时（如 text/plain）让识别器判定是否含可枚举结构。
        let structured = if structured {
            true
        } else if let Some(prism) = &self.prism {
            prism
                .scan_with(&text, &options.selectors, Some(&scan_limits(options.max_regions)))
                .await
                .map(|outcome| {
                    outcome
                        .type_ids()
                        .iter()
                        .any(|id| id.starts_with("struct."))
                })
                .unwrap_or(false)
        } else {
            false
        };

        let mut out = Vec::new();
        if structured {
            let values = if media.contains("csv") {
                csv_items(&text, options.max_items)
            } else {
                structured_items(&text)
            };
            for (rank, value) in values.iter().take(options.max_items).enumerate() {
                if let Some(item) = discovery_of_value(value, base, rank, options) {
                    out.push(item);
                }
            }
        }
        if out.is_empty() && !options.strict() && media.contains("html") {
            for (url, title) in html_links(&text, base, options.max_items) {
                if let Some(item) = discovery_of_link(&url, &title, options) {
                    out.push(item);
                }
            }
        }
        if out.is_empty() && !options.strict() && !structured && !media.contains("html") {
            for (rank, block) in text_blocks(&text).iter().take(options.max_items).enumerate() {
                out.push(discovery_of_text(block, &input.locator, rank, options));
            }
        }
        out
    }
}

#[async_trait]
impl SearchAdapter for SmartDiscover {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        Options::parse(spec).map(|_| ())
    }

    async fn probe(&self, input: &AdapterInput, spec: &Value) -> Option<Probe> {
        let options = Options::parse(spec).ok()?;
        let traits = input.detection.as_ref().map(|item| &item.traits);
        let structured = traits.is_some_and(|items| {
            items.iter().any(|item| {
                item == "struct.json"
                    || item == "struct.ndjson"
                    || item == "struct.csv"
                    || item == "struct.yaml"
            })
        });
        let confidence = if structured {
            0.9
        } else if options.strict() {
            0.2
        } else {
            0.5
        };
        Some(Probe {
            accept: confidence >= options.min_confidence,
            confidence: confidence as f32,
        })
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let options = Options::parse(spec)?;
        let base = self.context(input);
        let mut items = self.candidates(input, &options, &base).await;
        if items.is_empty() && !options.strict() {
            // 兜底：文档本身就是一条内容。质量标 Thin，界面能看出「只是原样收下」。
            items.push(discovery_of_document(&input.locator, &base, &options));
            return Ok(DiscoverOutput {
                items,
                feedback: Some(Feedback {
                    needs_render: None,
                    quality: Quality::Thin,
                    missing: vec!["structure".into()],
                }),
            });
        }
        if items.is_empty() {
            return Ok(DiscoverOutput {
                items,
                feedback: Some(Feedback {
                    needs_render: None,
                    quality: Quality::Empty,
                    missing: vec!["structure".into()],
                }),
            });
        }
        Ok(DiscoverOutput {
            items,
            feedback: None,
        })
    }
}

pub struct SmartContent {
    meta: AdapterMeta,
    prism: Option<Arc<dyn Prism>>,
    readability: Readability,
    page: PageFields,
}

impl Default for SmartContent {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SmartContent {
    pub fn new(prism: Option<Arc<dyn Prism>>) -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/smart-content".into(),
                version: "1".into(),
                role: AdapterRole::Content,
                consumes: Vec::new(),
                produces: vec![OutputDecl::Facet {
                    kind: "text".into(),
                    variant: None,
                }],
                deterministic: true,
            },
            prism,
            readability: Readability::new(),
            page: PageFields::new(),
        }
    }
}

#[async_trait]
impl ContentAdapter for SmartContent {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        Options::parse(spec).map(|_| ())
    }

    async fn probe(&self, input: &AdapterInput, spec: &Value) -> Option<Probe> {
        let options = Options::parse(spec).ok()?;
        Some(Probe {
            accept: true,
            confidence: if input.detection.is_some() { 0.8 } else { 0.4 },
        })
        .filter(|_| !options.strict() || input.detection.is_some())
    }

    async fn adapt(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<ContentOutput, AdapterError> {
        let options = Options::parse(spec)?;
        let mut facets = Vec::new();
        if let Some(prism) = &self.prism {
            facets = split_regions(prism.as_ref(), input, &options).await;
        }
        if facets.is_empty() {
            // 回退：正文抽取 + 页面字段。两者都失败时仍返回空集，由上层记为能力受限。
            let mut output = self.readability.adapt(input, &Value::Null).await?;
            output
                .facets
                .extend(self.page.adapt(input, &Value::Null).await?.facets);
            facets = output.facets;
        }
        let feedback = facets.is_empty().then(|| Feedback {
            needs_render: None,
            quality: Quality::Empty,
            missing: vec!["content".into()],
        });
        Ok(ContentOutput {
            facets,
            links: Vec::new(),
            feedback,
        })
    }
}

/// 按识别区域把一份文档拆成多份 facet，并写一份区域索引。
///
/// 同类型多份用 `r0..rN` 区分；索引里记录 `typeId`、偏移与对应 facet，
/// 内容库据此列出「这份内容包含哪几类、每类几份」。
async fn split_regions(
    prism: &dyn Prism,
    input: &AdapterInput,
    options: &Options,
) -> Vec<FacetDraft> {
    let text = prism_input(&input.view.bytes);
    if text.trim().is_empty() {
        return Vec::new();
    }
    let Ok(outcome) = prism
        .scan_with(&text, &options.selectors, Some(&scan_limits(options.max_regions)))
        .await
    else {
        return Vec::new();
    };
    let mut facets = Vec::new();
    let mut index = Vec::new();
    let mut type_ids = outcome.type_ids();
    type_ids.sort();
    let mut seq = 0usize;
    for type_id in type_ids {
        let hits = outcome.regions_of(&type_id);
        let selectors = vec![type_id.clone()];
        for (position, hit) in hits.iter().enumerate() {
            if seq >= options.max_regions {
                break;
            }
            if hit.confidence < options.min_confidence {
                continue;
            }
            let Ok(region) = prism
                .materialize(&text, &selectors, position, false)
                .await
            else {
                continue;
            };
            let Some(body) = region.get("text").and_then(Value::as_str) else {
                continue;
            };
            if body.trim().is_empty() {
                continue;
            }
            let (kind, media) = kind_of(&type_id);
            let variant = format!("r{seq}");
            facets.push(FacetDraft {
                kind: kind.clone(),
                variant: Some(variant.clone()),
                body: DraftBody::Text(body.to_string()),
                media_type: media.map(str::to_string),
                derived_from: Vec::new(),
            });
            index.push(json!({
                "typeId": type_id,
                "start": hit.start,
                "end": hit.end,
                "confidence": hit.confidence,
                "kind": kind.as_str(),
                "variant": variant,
            }));
            seq += 1;
        }
    }
    if !index.is_empty() {
        facets.push(FacetDraft {
            kind: FacetKind::Regions,
            variant: Some("prism".into()),
            body: DraftBody::Struct(json!({ "regions": index })),
            media_type: Some("application/json".into()),
            derived_from: Vec::new(),
        });
    }
    facets
}

/// TypeId → facet 种类与媒体类型。
fn kind_of(type_id: &str) -> (FacetKind, Option<&'static str>) {
    match type_id {
        "struct.json" => (FacetKind::Json, Some("application/json")),
        "struct.ndjson" => (FacetKind::Json, Some("application/x-ndjson")),
        "struct.yaml" => (FacetKind::Yaml, Some("text/yaml")),
        "struct.toml" => (FacetKind::Yaml, Some("application/toml")),
        "struct.csv" => (FacetKind::Other("csv".into()), Some("text/csv")),
        "struct.xml" | "markup.xml" => (FacetKind::Xml, Some("application/xml")),
        "markup.html" => (FacetKind::Html, Some("text/html")),
        "text.md" => (FacetKind::Md, Some("text/markdown")),
        "text.plain" => (FacetKind::Text, Some("text/plain")),
        _ => (FacetKind::Text, Some("text/plain")),
    }
}

// ---- 条目推断 ----

fn structured_items(text: &str) -> Vec<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return container_items(&value);
    }
    let mut items = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || (!line.starts_with('{') && !line.starts_with('[')) {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            items.push(value);
        }
    }
    items
}

fn container_items(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::Object(map) => {
            for key in [
                "data", "items", "results", "list", "records", "entries", "rows", "listData",
            ] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return items.clone();
                }
            }
            if let Some(found) = map.values().find_map(|item| match item {
                Value::Array(items) if items.iter().any(Value::is_object) => Some(items.clone()),
                _ => None,
            }) {
                return found;
            }
            vec![value.clone()]
        }
        _ => Vec::new(),
    }
}

fn csv_items(text: &str, limit: usize) -> Vec<Value> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let keys: Vec<String> = split_csv(header);
    if keys.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::new();
    for line in lines.take(limit) {
        let cells = split_csv(line);
        if cells.len() != keys.len() {
            continue;
        }
        let mut map = Map::new();
        for (key, cell) in keys.iter().zip(cells) {
            map.insert(key.clone(), Value::String(cell));
        }
        items.push(Value::Object(map));
    }
    items
}

fn split_csv(line: &str) -> Vec<String> {
    line.split(',')
        .map(|cell| cell.trim().trim_matches('"').to_string())
        .collect()
}

fn discovery_of_value(
    value: &Value,
    base: &str,
    rank: usize,
    options: &Options,
) -> Option<Discovery> {
    let object = value.as_object()?;
    let url = ["url", "link", "href", "permalink", "detailUrl", "sourceUrl"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .find_map(|raw| absolute_http(base, raw));
    let identity = match &url {
        Some(url) => url_identity(url),
        None => None,
    };
    // 没有 URL 的条目按内容指纹去重：同一份数据重复采集不会翻倍。
    let identity = identity.or_else(|| {
        let hash = strata_model::content_hash(value.to_string().as_bytes());
        Some(format!("ext:item:{hash}"))
    })?;
    Some(Discovery {
        locator: url
            .clone()
            .map(|url| Locator::Url { url })
            .unwrap_or_else(|| Locator::Query {
                query: base.to_string(),
            }),
        identity: vec![identity],
        hints: hints_of(object, rank),
        span: None,
        follow: follow_of(options),
        materialize: vec![FacetDraft {
            kind: FacetKind::Struct,
            variant: Some("item".into()),
            body: DraftBody::Struct(value.clone()),
            media_type: Some("application/json".into()),
            derived_from: Vec::new(),
        }],
    })
}

fn discovery_of_link(url: &str, title: &str, options: &Options) -> Option<Discovery> {
    Some(Discovery {
        locator: Locator::Url {
            url: url.to_string(),
        },
        identity: vec![url_identity(url)?],
        hints: DiscoveryHints {
            title: Some(title.to_string()),
            ..DiscoveryHints::default()
        },
        span: None,
        follow: follow_of(options),
        materialize: Vec::new(),
    })
}

fn discovery_of_text(block: &str, locator: &Locator, rank: usize, options: &Options) -> Discovery {
    let title = block
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| collapse_ws(line.trim()))
        .unwrap_or_else(|| format!("第 {} 段", rank + 1));
    let identity = format!(
        "ext:item:{}",
        strata_model::content_hash(block.as_bytes())
    );
    Discovery {
        locator: locator.clone(),
        identity: vec![identity],
        hints: DiscoveryHints {
            title: Some(title.chars().take(120).collect()),
            summary: Some(block.chars().take(280).collect()),
            rank: Some(rank as u32),
            ..DiscoveryHints::default()
        },
        span: None,
        follow: follow_of(options),
        materialize: Vec::new(),
    }
}

fn discovery_of_document(locator: &Locator, base: &str, options: &Options) -> Discovery {
    let identity = url_identity(base)
        .unwrap_or_else(|| format!("ext:item:{}", strata_model::content_hash(base.as_bytes())));
    Discovery {
        locator: locator.clone(),
        identity: vec![identity],
        hints: DiscoveryHints {
            title: Some(host_of(base).unwrap_or_else(|| base.chars().take(80).collect())),
            ..DiscoveryHints::default()
        },
        span: None,
        follow: follow_of(options),
        materialize: Vec::new(),
    }
}

fn host_of(base: &str) -> Option<String> {
    url::Url::parse(base)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

fn hints_of(object: &Map<String, Value>, rank: usize) -> DiscoveryHints {
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .filter_map(|key| object.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .find(|value| !value.is_empty())
            .map(str::to_string)
    };
    let raw_title = pick(&["title", "name", "headline", "text", "label", "subject"]);
    let published = pick(&[
        "publishedAt",
        "published_at",
        "pubDate",
        "date",
        "createdAt",
        "timestamp",
        "time",
    ]);
    let mut extra = Map::new();
    for key in ["id", "category", "tag", "type", "author", "score"] {
        if let Some(value) = object.get(key) {
            extra.insert(key.to_string(), value.clone());
        }
    }
    DiscoveryHints {
        title: raw_title.map(|title| collapse_ws(&title).chars().take(200).collect()),
        published_at: published.as_deref().and_then(|raw| parse_published(raw, None, None)),
        author: pick(&["author", "authorName", "byline", "creator"]),
        summary: pick(&["summary", "description", "desc", "abstract", "excerpt"])
            .map(|text| text.chars().take(400).collect()),
        rank: Some(rank as u32),
        extra,
    }
}

fn follow_of(options: &Options) -> Follow {
    if options.follow {
        Follow::Yes {
            accessor: "accessor/http".into(),
            accessor_spec: Value::Object(Map::new()),
            render: crate::model::RenderMode::Auto,
            priority: 0,
            needs: vec!["body".into()],
        }
    } else {
        Follow::No
    }
}

fn html_links(text: &str, base: &str, limit: usize) -> Vec<(String, String)> {
    let document = Html::parse_document(text);
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for element in document.select(&selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Some(url) = absolute_http(base, href) else {
            continue;
        };
        let title = collapse_ws(&element.text().collect::<Vec<_>>().join(" "));
        if title.chars().count() < 4 {
            continue;
        }
        if out.iter().any(|(existing, _)| existing == &url) {
            continue;
        }
        out.push((url, title));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn text_blocks(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|block| block.chars().count() >= 8)
        .map(str::to_string)
        .collect()
}

/// 让 `InputSelector` 在未来需要按类型收窄时可用；当前智能适配器接受任意输入。
#[allow(dead_code)]
fn any_document() -> InputSelector {
    InputSelector {
        kind: Some("raw".into()),
        variant: None,
        part_role: Some("document".into()),
        media_type: None,
    }
}
