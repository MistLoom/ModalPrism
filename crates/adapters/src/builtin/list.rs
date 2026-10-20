//! HTML 列表与 JSON 列表。

use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use strata_model::{FacetKind, Locator, RenderLevel};

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, DiscoverOutput, Discovery, DiscoveryHints, DraftBody,
    FacetDraft, Feedback, Follow, InputSelector, OutputDecl, Quality,
};
use crate::textutil::{
    absolute_http, collapse_ws, decode_basic_entities, http_absolute, json_path, locator_text,
    parse_published, render_template, url_identity,
};
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
                consumes: vec![
                    document_selector("text/html"),
                    document_selector("text/markdown"),
                    document_selector("text/plain"),
                ],
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
                consumes: vec![
                    InputSelector {
                        kind: Some("raw".into()),
                        variant: None,
                        part_role: None,
                        media_type: Some("application/json".into()),
                    },
                    InputSelector {
                        kind: Some("raw".into()),
                        variant: None,
                        part_role: None,
                        media_type: Some("text/plain".into()),
                    },
                ],
                produces: vec![OutputDecl::Discovery],
                deterministic: true,
            },
        }
    }
}

struct HtmlSpec {
    mode: String,
    item: Option<String>,
    link: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    published_selector: Option<String>,
    published_attr: Option<String>,
    offset: Option<String>,
    base_url: Option<String>,
    min_title: usize,
}

#[async_trait]
impl SearchAdapter for HtmlList {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        let parsed = html_spec(spec)?;
        match parsed.mode.as_str() {
            "html" => {
                let item = parsed
                    .item
                    .as_deref()
                    .ok_or_else(|| AdapterError::config("缺少 item"))?;
                let link = parsed
                    .link
                    .as_deref()
                    .ok_or_else(|| AdapterError::config("缺少 link"))?;
                must_selector(item)?;
                must_selector(link)?;
            }
            "markdown" | "docusaurus_changelog" | "intercom_changelog" => {}
            _ => return Err(AdapterError::config("parseMode")),
        }
        parse_optional(&parsed.title)?;
        parse_optional(&parsed.summary)?;
        parse_optional(&parsed.published_selector)?;
        if let Some(offset) = &parsed.offset {
            if !offset_ok(offset) {
                return Err(AdapterError::config("publishedAtUtcOffset"));
            }
        }
        Ok(())
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let parsed = html_spec(spec)?;
        let base = parsed
            .base_url
            .clone()
            .unwrap_or_else(|| locator_text(&input.locator).to_string());
        let html = String::from_utf8_lossy(&input.view.bytes);
        let mut items = match parsed.mode.as_str() {
            "markdown" => markdown_items(&html, &base, &parsed, input),
            "docusaurus_changelog" => changelog_items(&html, &base, &parsed, input, false)?,
            "intercom_changelog" => changelog_items(&html, &base, &parsed, input, true)?,
            _ => html_items(&html, &base, &parsed, input)?,
        };
        number_ranks(&mut items);
        let render = if parsed.mode == "html" {
            Some(RenderLevel::L2)
        } else {
            None
        };
        Ok(DiscoverOutput {
            feedback: if items.is_empty() {
                Some(Feedback {
                    needs_render: render,
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
        match spec.get("items").and_then(Value::as_str) {
            Some(pointer) if pointer.starts_with('/') => {}
            Some(_) => return Err(AdapterError::config("items 必须是 JSON Pointer")),
            None if spec.get("itemsPath").is_some() => {}
            None => return Err(AdapterError::config("缺少 items")),
        }
        let url = optional_string(spec, "url");
        let template = optional_string(spec, "urlTemplate");
        if url.is_none() && template.is_none() {
            return Err(AdapterError::config("缺少 url"));
        }
        if let Some(unit) = optional_string(spec, "publishedAtUnit") {
            if !matches!(unit.as_str(), "epoch_s" | "epoch_ms" | "yyyymmdd") {
                return Err(AdapterError::config("publishedAtUnit"));
            }
        }
        if let Some(offset) = optional_string(spec, "publishedAtUtcOffset") {
            if !offset_ok(&offset) {
                return Err(AdapterError::config("publishedAtUtcOffset"));
            }
        }
        Ok(())
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let value: Value = serde_json::from_slice(&input.view.bytes).unwrap_or(Value::Null);
        let mut items = Vec::new();
        if let Some(entries) = json_items(&value, spec) {
            for entry in entries {
                if let Some(discovery) = json_discovery(entry, input, spec) {
                    items.push(discovery);
                }
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

fn document_selector(media_type: &str) -> InputSelector {
    InputSelector {
        kind: Some("raw".into()),
        variant: None,
        part_role: Some("document".into()),
        media_type: Some(media_type.into()),
    }
}

fn html_items(
    html: &str,
    base: &str,
    spec: &HtmlSpec,
    input: &AdapterInput,
) -> Result<Vec<Discovery>, AdapterError> {
    let item_sel = must_selector(spec.item.as_deref().unwrap_or(""))?;
    let link_sel = must_selector(spec.link.as_deref().unwrap_or(""))?;
    let title_sel = optional_selector(spec.title.as_deref())?;
    let summary_sel = optional_selector(spec.summary.as_deref())?;
    let published_sel = optional_selector(spec.published_selector.as_deref())?;
    let document = Html::parse_document(html);
    let mut items = Vec::new();
    let mut seen = Vec::new();
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
        if same_document(&absolute, base) {
            continue;
        }
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
        let published = published_at(element, published_sel.as_ref(), spec);
        if let Some(index) = seen.iter().position(|item| item == &identity) {
            if let Some(title) = title.clone() {
                replace_shorter_title(&mut items[index], title);
            }
            continue;
        }
        seen.push(identity.clone());
        items.push(discovery(
            input,
            ItemFields {
                url: absolute,
                identity: vec![identity],
                title,
                published,
                author: None,
                summary,
                summary_is_body: false,
            },
        ));
    }
    Ok(items)
}

fn markdown_items(text: &str, base: &str, spec: &HtmlSpec, input: &AdapterInput) -> Vec<Discovery> {
    let mut items = Vec::new();
    let mut seen = Vec::new();
    for (label, href) in markdown_links(text) {
        let title = collapse_ws(&label.replace(['*', '_', '`', '#'], ""));
        if title.chars().count() < spec.min_title {
            continue;
        }
        let Some(absolute) = absolute_http(base, &href) else {
            continue;
        };
        if same_document(&absolute, base) {
            continue;
        }
        let Some(identity) = url_identity(&absolute) else {
            continue;
        };
        if let Some(index) = seen.iter().position(|item| item == &identity) {
            replace_shorter_title(&mut items[index], title);
            continue;
        }
        seen.push(identity.clone());
        items.push(discovery(
            input,
            ItemFields {
                url: absolute,
                identity: vec![identity],
                title: Some(title),
                published: None,
                author: None,
                summary: None,
                summary_is_body: false,
            },
        ));
    }
    items
}

fn changelog_items(
    html: &str,
    base: &str,
    spec: &HtmlSpec,
    input: &AdapterInput,
    intercom: bool,
) -> Result<Vec<Discovery>, AdapterError> {
    let document = Html::parse_document(html);
    if intercom {
        return intercom_items(&document, base, spec, input);
    }
    let selector =
        must_selector("article h2[id], article h3[id], .markdown h2[id], .markdown h3[id]")?;
    let mut items = Vec::new();
    let mut section_date = None;
    for head in document.select(&selector) {
        let Some(id) = head.attr("id") else {
            continue;
        };
        let title = collapse_ws(
            &head
                .text()
                .collect::<String>()
                .replace(['#', '\u{200b}'], ""),
        );
        if let Some(date) = date_heading(&title, spec.offset.as_deref()) {
            section_date = Some(date);
            continue;
        }
        if head.value().name() == "h2" {
            section_date = None;
        }
        let Some(absolute) = with_fragment(base, id) else {
            continue;
        };
        let Some(identity) = url_identity(&absolute) else {
            continue;
        };
        items.push(discovery(
            input,
            ItemFields {
                url: absolute,
                identity: vec![identity],
                title: Some(title),
                published: section_date,
                author: None,
                summary: None,
                summary_is_body: false,
            },
        ));
    }
    Ok(items)
}

fn intercom_items(
    document: &Html,
    base: &str,
    spec: &HtmlSpec,
    input: &AdapterInput,
) -> Result<Vec<Discovery>, AdapterError> {
    let selector = must_selector("article h2[id], article h3[id]")?;
    let mut items = Vec::new();
    for heading in document.select(&selector) {
        let Some(id) = heading.attr("id") else {
            continue;
        };
        let heading_text = collapse_ws(&heading.text().collect::<String>());
        let offset = spec.offset.clone().unwrap_or_else(|| "+00:00".into());
        let Some(published) = parse_published(&heading_text, Some(&offset), None) else {
            continue;
        };
        let (title, body) = intercom_body(heading);
        if body.trim().is_empty() {
            continue;
        }
        let Some(absolute) = with_fragment(base, id) else {
            continue;
        };
        let Some(identity) = url_identity(&absolute) else {
            continue;
        };
        let title = if title.is_empty() {
            heading_text
        } else {
            title
        };
        items.push(discovery(
            input,
            ItemFields {
                url: absolute,
                identity: vec![identity],
                title: Some(title),
                published: Some(published),
                author: None,
                summary: Some(body.clone()),
                summary_is_body: true,
            },
        ));
    }
    Ok(items)
}

fn intercom_body(block: ElementRef<'_>) -> (String, String) {
    let mut titles = Vec::new();
    let mut parts = Vec::new();
    let mut next = block.next_sibling();
    while let Some(node) = next {
        if let Some(element) = ElementRef::wrap(node) {
            if element.value().name().eq_ignore_ascii_case("h2")
                || element.value().name().eq_ignore_ascii_case("h3")
                || element
                    .select(&Selector::parse("h2, h3").expect("h2"))
                    .next()
                    .is_some()
            {
                break;
            }
            let text = collapse_ws(&element.text().collect::<String>());
            if !text.is_empty() {
                parts.push(text.clone());
            }
            if let Some(paragraph) = element
                .select(&Selector::parse("p").expect("p"))
                .next()
                .or_else(|| {
                    if element.value().name() == "p" {
                        Some(element)
                    } else {
                        None
                    }
                })
            {
                let paragraph_text = collapse_ws(&paragraph.text().collect::<String>());
                let bold: Vec<_> = paragraph.children().filter_map(ElementRef::wrap).collect();
                let only_bold = !bold.is_empty()
                    && bold
                        .iter()
                        .all(|child| matches!(child.value().name(), "b" | "strong"));
                if only_bold && !paragraph_text.is_empty() {
                    titles.push(paragraph_text);
                }
            }
        }
        next = node.next_sibling();
    }
    (titles.join(" · "), parts.join("\n"))
}

fn published_at(
    element: ElementRef<'_>,
    selector: Option<&Selector>,
    spec: &HtmlSpec,
) -> Option<strata_model::Timestamp> {
    let selector = selector?;
    let node = element.select(selector).next()?;
    if let Some(attr) = spec.published_attr.as_deref() {
        return node
            .attr(attr)
            .and_then(|raw| parse_published(raw, spec.offset.as_deref(), None));
    }
    for key in ["datetime", "content", "title"] {
        if let Some(raw) = node.attr(key) {
            if let Some(parsed) = parse_published(raw, spec.offset.as_deref(), None) {
                return Some(parsed);
            }
        }
    }
    parse_published(
        &node.text().collect::<String>(),
        spec.offset.as_deref(),
        None,
    )
}

fn json_items<'a>(value: &'a Value, spec: &Value) -> Option<&'a Vec<Value>> {
    if let Some(pointer) = spec.get("items").and_then(Value::as_str) {
        return value.pointer(pointer).and_then(Value::as_array);
    }
    let path = spec.get("itemsPath").and_then(Value::as_str).unwrap_or("");
    json_path(value, path)?.as_array()
}

fn json_discovery(entry: &Value, input: &AdapterInput, spec: &Value) -> Option<Discovery> {
    let url = if let Some(template) = optional_string(spec, "urlTemplate") {
        http_absolute(&render_template(&template, entry)?)?
    } else {
        let key = optional_string(spec, "url")?;
        let raw = scalar_at(entry, &key)?;
        absolute_http(locator_text(&input.locator), &raw)?
    };
    let mut identity = vec![url_identity(&url)?];
    if let Some(path) = optional_string(spec, "externalIdPath") {
        if let Some(raw) = scalar_at(entry, &path) {
            if !raw.chars().any(char::is_whitespace) {
                let key = format!("ext:{raw}");
                if key != identity[0] {
                    identity.push(key);
                }
            }
        }
    }
    let decode = spec
        .get("decodeEntities")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let title = first_text(entry, &path_list(spec, "title", "titlePaths")).map(|text| {
        if decode {
            decode_basic_entities(&text)
        } else {
            text
        }
    });
    let summary = first_text(entry, &path_list(spec, "summary", "summaryPaths")).map(|text| {
        if decode {
            decode_basic_entities(&text)
        } else {
            text
        }
    });
    let author = first_text(entry, &path_list(spec, "author", "authorPaths"));
    let published = published_value(entry, spec);
    let summary_is_body = spec
        .get("summaryIsBody")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(discovery(
        input,
        ItemFields {
            url,
            identity,
            title,
            published,
            author,
            summary,
            summary_is_body,
        },
    ))
}

fn published_value(entry: &Value, spec: &Value) -> Option<strata_model::Timestamp> {
    let path = optional_string(spec, "publishedAtPath")
        .or_else(|| optional_string(spec, "publishedAt"))?;
    let raw = scalar_at(entry, &path)?;
    let unit = optional_string(spec, "publishedAtUnit");
    let offset = optional_string(spec, "publishedAtUtcOffset");
    parse_published(&raw, offset.as_deref(), unit.as_deref())
}

struct ItemFields {
    url: String,
    identity: Vec<String>,
    title: Option<String>,
    published: Option<strata_model::Timestamp>,
    author: Option<String>,
    summary: Option<String>,
    summary_is_body: bool,
}

fn discovery(input: &AdapterInput, fields: ItemFields) -> Discovery {
    let ItemFields {
        url,
        identity,
        title,
        published,
        author,
        summary,
        summary_is_body,
    } = fields;
    let (materialize, follow) = if summary_is_body && summary.is_some() {
        (
            vec![FacetDraft {
                kind: FacetKind::Text,
                variant: None,
                body: DraftBody::Text(summary.clone().unwrap_or_default()),
                media_type: Some("text/plain".into()),
                derived_from: Vec::new(),
            }],
            Follow::No,
        )
    } else {
        (Vec::new(), follow_yes(input))
    };
    Discovery {
        locator: Locator::Url { url },
        identity,
        hints: DiscoveryHints {
            title,
            published_at: published,
            author,
            summary,
            rank: None,
            extra: Default::default(),
        },
        span: None,
        follow,
        materialize,
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
    let min_title = match spec.get("minTitle") {
        None => 1,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| AdapterError::config("minTitle"))? as usize,
    };
    Ok(HtmlSpec {
        mode: optional_string(spec, "parseMode").unwrap_or_else(|| "html".into()),
        item: optional_string(spec, "item"),
        link: optional_string(spec, "link"),
        title: optional_string(spec, "title"),
        summary: optional_string(spec, "summary"),
        published_selector,
        published_attr,
        offset: optional_string(spec, "publishedAtUtcOffset"),
        base_url: optional_string(spec, "baseUrl"),
        min_title,
    })
}

fn path_list(spec: &Value, singular: &str, plural: &str) -> Vec<String> {
    if let Some(items) = spec.get(plural).and_then(Value::as_array) {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    match spec.get(singular) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(text)) if !text.is_empty() => vec![text.clone()],
        _ => Vec::new(),
    }
}

fn first_text(entry: &Value, paths: &[String]) -> Option<String> {
    paths.iter().find_map(|path| scalar_at(entry, path))
}

fn scalar_at(entry: &Value, path: &str) -> Option<String> {
    let value = if path.contains('.') {
        json_path(entry, path)?
    } else {
        entry.get(path).or_else(|| json_path(entry, path))?
    };
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn markdown_links(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let after = &rest[start + 1..];
        let Some(label_end) = after.find(']') else {
            break;
        };
        if !after[label_end..].starts_with("](") {
            rest = &rest[start + 1..];
            continue;
        }
        let url_at = label_end + 2;
        let Some(url_end) = after[url_at..].find(')') else {
            break;
        };
        let label = after[..label_end].to_string();
        let raw_url = &after[url_at..url_at + url_end];
        let href = raw_url
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .to_string();
        if !label.trim().is_empty() && !href.is_empty() {
            out.push((label, href));
        }
        rest = &after[url_at + url_end + 1..];
    }
    out
}

fn date_heading(title: &str, offset: Option<&str>) -> Option<strata_model::Timestamp> {
    let mut text = title.trim().trim_end_matches('#').trim();
    if let Some(index) = text.find([':', '：']) {
        let prefix = &text[..index];
        if prefix.chars().count() <= 12 && !prefix.chars().any(|ch| ch.is_ascii_digit()) {
            text = text[index + 1..].trim();
        }
    }
    if text.len() > 32 || text.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    parse_published(text, offset, None)
}

fn with_fragment(base: &str, id: &str) -> Option<String> {
    let mut url = url::Url::parse(base).ok()?;
    url.set_fragment(Some(id));
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    Some(url.to_string())
}

fn same_document(url: &str, base: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    let Ok(base) = url::Url::parse(base) else {
        return false;
    };
    url.host() == base.host()
        && url.path().trim_end_matches('/') == base.path().trim_end_matches('/')
        && url.query() == base.query()
}

fn replace_shorter_title(item: &mut Discovery, title: String) {
    let current = item.hints.title.as_deref().unwrap_or("");
    if title.chars().count() > current.chars().count() {
        item.hints.title = Some(title);
    }
}

fn offset_ok(offset: &str) -> bool {
    let bytes = offset.as_bytes();
    bytes.len() == 6
        && (bytes[0] == b'+' || bytes[0] == b'-')
        && bytes[3] == b':'
        && bytes[1..3].iter().all(u8::is_ascii_digit)
        && bytes[4..6].iter().all(u8::is_ascii_digit)
}

fn required_string(spec: &Value, key: &str) -> Result<String, AdapterError> {
    optional_string(spec, key).ok_or_else(|| AdapterError::config(format!("缺少 {key}")))
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
