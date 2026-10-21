//! 从页面及其脚本中抽出列表，返回 JSON 数组。脚本地址和字段形状写在 spec 里。

use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use strata_model::Locator;

use crate::envelope::{Page, from_page};
use crate::headers::is_http;
use crate::http::{Spec as HttpSpec, fetch_page, url_of};
use crate::ready::{config, ready};

pub struct BundleListAccessor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    #[serde(default = "default_scripts")]
    max_scripts: u32,
    #[serde(default = "default_bytes")]
    max_script_bytes: u64,
    extract: Vec<Extractor>,
}

fn default_scripts() -> u32 {
    8
}

fn default_bytes() -> u64 {
    1_048_576
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Extractor {
    QuotedFields {
        marker: String,
        #[serde(default, rename = "endMarker")]
        end_marker: Option<String>,
        fields: Vec<String>,
    },
    RouteRecord {
        #[serde(rename = "routePrefix")]
        route_prefix: String,
        #[serde(default)]
        lang: Option<String>,
        #[serde(default)]
        skip: Vec<String>,
    },
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    let spec: Spec = serde_json::from_value(spec.clone()).map_err(|_| config("spec 无效"))?;
    if spec.extract.is_empty()
        || !(1..=20).contains(&spec.max_scripts)
        || spec.max_script_bytes == 0
    {
        return Err(config("spec 无效"));
    }
    if spec.max_script_bytes > 2_000_000 {
        return Err(config("spec 无效"));
    }
    for item in &spec.extract {
        match item {
            Extractor::QuotedFields { marker, fields, .. }
                if marker.is_empty() || fields.is_empty() =>
            {
                return Err(config("spec 无效"));
            }
            Extractor::RouteRecord { route_prefix, .. } if route_prefix.is_empty() => {
                return Err(config("spec 无效"));
            }
            _ => {}
        }
    }
    Ok(spec)
}

fn capabilities() -> AccessorCapabilities {
    AccessorCapabilities {
        protocols: vec!["http".into(), "https".into()],
        conditional: false,
        render: false,
        layout: false,
        capture_network: CaptureNetwork::None,
        interact: false,
        paid: None,
        batchable: false,
        inbound: false,
    }
}

#[async_trait]
impl Accessor for BundleListAccessor {
    fn id(&self) -> &str {
        "bundle-list"
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
        let http = HttpSpec {
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            session: None,
        };
        let page = fetch_page(&request, ctx, &http, url.clone()).await?;
        if !(200..300).contains(&page.status) {
            return Ok(vec![from_page(ctx, &request.policy, started, page)?]);
        }
        let html = String::from_utf8_lossy(&page.body).into_owned();
        let base = if page.final_url.is_empty() {
            url.clone()
        } else {
            page.final_url.clone()
        };
        let mut items = extract_text(&html, &base, &spec.extract);
        if items.is_empty() {
            let mut scripts = script_srcs(&html, &base);
            let cap = spec.max_scripts as usize;
            if scripts.len() > cap {
                scripts = scripts.split_off(scripts.len() - cap);
            }
            scripts.reverse();
            let attempted = !scripts.is_empty();
            for script in scripts {
                ready(ctx, request.deadline)?;
                let fetched = fetch_page(&request, ctx, &http, script).await?;
                if !(200..300).contains(&fetched.status) {
                    continue;
                }
                if fetched.body.len() as u64 > spec.max_script_bytes {
                    continue;
                }
                let text = String::from_utf8_lossy(&fetched.body);
                items = extract_text(&text, &base, &spec.extract);
                if !items.is_empty() {
                    break;
                }
            }
            if items.is_empty() && attempted {
                return Err(accessors::AccessError::new(
                    strata_model::ErrorClass::Permanent,
                    "script-fetch",
                ));
            }
        }
        let body = json!({ "items": items }).to_string();
        let synthetic = Page {
            locator: Locator::Url { url: base },
            final_url: page.final_url,
            status: page.status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: Bytes::from(body),
            cursor: None,
            receipt_id: None,
            render_level: page.render_level,
            engine: None,
            extra: Vec::new(),
            document: true,
        };
        Ok(vec![from_page(ctx, &request.policy, started, synthetic)?])
    }
}

fn extract_text(text: &str, base: &str, extractors: &[Extractor]) -> Vec<Value> {
    for extractor in extractors {
        let items = match extractor {
            Extractor::QuotedFields {
                marker,
                end_marker,
                fields,
            } => quoted_items(text, base, marker, end_marker.as_deref(), fields),
            Extractor::RouteRecord {
                route_prefix,
                lang,
                skip,
            } => route_items(text, base, route_prefix, lang.as_deref(), skip),
        };
        if !items.is_empty() {
            return items;
        }
    }
    Vec::new()
}

fn quoted_items(
    text: &str,
    base: &str,
    marker: &str,
    end_marker: Option<&str>,
    fields: &[String],
) -> Vec<Value> {
    let Some(start) = text.find(marker) else {
        return Vec::new();
    };
    let after = start + marker.len();
    let end = end_marker
        .and_then(|marker| text[after..].find(marker).map(|index| after + index))
        .unwrap_or(text.len());
    let mut items = Vec::new();
    let mut seen = Vec::new();
    for object in brace_objects(&text[start..end]) {
        if !fields
            .iter()
            .any(|field| field_string(object, field).is_some())
        {
            continue;
        }
        let title = field_string(object, "title").unwrap_or_default();
        let link = field_string(object, "link").or_else(|| field_string(object, "url"));
        let Some(link) = link else {
            continue;
        };
        if title.trim().is_empty() {
            continue;
        }
        let Some(url) = join_url(base, &link) else {
            continue;
        };
        if seen.contains(&url) {
            continue;
        }
        seen.push(url.clone());
        let summary = field_string(object, "desc")
            .or_else(|| field_string(object, "description"))
            .or_else(|| field_string(object, "summary"))
            .unwrap_or_default();
        let published = field_string(object, "date")
            .or_else(|| field_string(object, "publishedAt"))
            .unwrap_or_default();
        items.push(record(&url, &title, &summary, &published));
    }
    items
}

fn route_items(
    text: &str,
    base: &str,
    prefix: &str,
    lang: Option<&str>,
    skip: &[String],
) -> Vec<Value> {
    let key = "routePath:\"";
    let mut from = 0;
    let mut items = Vec::new();
    let mut seen = Vec::new();
    while let Some(at) = text[from..].find(key) {
        let abs = from + at;
        let rest = &text[abs + key.len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        let route = &rest[..end];
        from = abs + key.len() + end + 1;
        if !route.starts_with(prefix) || skip.iter().any(|item| item == route) {
            continue;
        }
        let window_end = text.len().min(abs + 4000);
        let window = &text[abs..window_end];
        let before_front = window.split("frontmatter:").next().unwrap_or(window);
        if let Some(lang) = lang {
            if !before_front.contains(&format!("lang:\"{lang}\"")) {
                continue;
            }
        }
        let Some(front_at) = window.find("frontmatter:{") else {
            continue;
        };
        let Some(front) = brace_slice(&window[front_at + "frontmatter:".len()..]) else {
            continue;
        };
        let title = field_string(front, "title").unwrap_or_default();
        let summary = field_string(front, "description")
            .or_else(|| field_string(front, "desc"))
            .unwrap_or_default();
        let published = field_string(front, "date").unwrap_or_default();
        let Some(url) = join_url(base, route) else {
            continue;
        };
        if seen.contains(&url) {
            continue;
        }
        seen.push(url.clone());
        items.push(record(&url, &title, &summary, &published));
    }
    items
}

fn record(url: &str, title: &str, summary: &str, published: &str) -> Value {
    let mut map = Map::new();
    map.insert("url".into(), Value::String(url.to_string()));
    if !title.trim().is_empty() {
        map.insert("title".into(), Value::String(collapse(title)));
    }
    if !summary.trim().is_empty() && summary.trim() != title.trim() {
        map.insert("summary".into(), Value::String(collapse(summary)));
    }
    if !published.trim().is_empty() {
        map.insert(
            "publishedAt".into(),
            Value::String(published.trim().to_string()),
        );
    }
    Value::Object(map)
}

fn script_srcs(html: &str, base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = find_ignore_ascii(rest, "<script") {
        let tag = &rest[at..];
        let Some(end) = tag.find('>') else {
            break;
        };
        if let Some(src) = attr_value(&tag[..=end], "src") {
            if let Some(url) = join_url(base, &src) {
                if is_http(&url) && !out.contains(&url) {
                    out.push(url);
                }
            }
        }
        rest = &rest[at + end + 1..];
    }
    out
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let at = lower.find(&key)?;
    let after = tag[at + key.len()..].trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &after[quote.len_utf8()..];
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

fn join_url(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let url = url::Url::parse(base).ok()?.join(href).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    Some(url.to_string())
}

fn brace_objects(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            if let Some(end) = matching_brace(text, index) {
                out.push(&text[index..end]);
                index = end;
                continue;
            }
        }
        index += 1;
    }
    out
}

fn brace_slice(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = matching_brace(text, start)?;
    Some(&text[start..end])
}

fn matching_brace(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut index = start;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(mark) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == mark {
                quote = None;
            }
        } else if byte == b'"' || byte == b'\'' {
            quote = Some(byte);
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(index + 1);
            }
        }
        index += 1;
    }
    None
}

fn field_string(object: &str, name: &str) -> Option<String> {
    let bytes = object.as_bytes();
    let mut from = 0;
    while from < object.len() {
        let at = object[from..].find(name)?;
        let abs = from + at;
        let before = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
        let after = &object[abs + name.len()..];
        let trimmed = after.trim_start();
        if before && trimmed.starts_with(':') {
            let value = trimmed[1..].trim_start();
            if let Some(text) = read_js_string(value) {
                return Some(text);
            }
        }
        from = abs + name.len();
    }
    None
}

fn read_js_string(text: &str) -> Option<String> {
    let mut chars = text.chars();
    if chars.next() != Some('"') {
        return None;
    }
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(decoded) = char::from_u32(code) {
                            out.push(decoded);
                        }
                    }
                }
                Some(other) => out.push(other),
                None => break,
            }
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_ignore_ascii(hay: &str, needle: &str) -> Option<usize> {
    let hay_bytes = hay.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || hay_bytes.len() < needle.len() {
        return None;
    }
    let last = hay_bytes.len() - needle.len();
    for index in 0..=last {
        if hay_bytes[index..index + needle.len()].eq_ignore_ascii_case(needle) {
            return Some(index);
        }
    }
    None
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(capabilities);
