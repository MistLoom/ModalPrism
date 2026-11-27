//! HTML 表格：表头映射到声明的字段名，每一行一条发现。
//!
//! 表格行本身没有稳定 URL，因此身份取整行投影的指纹；声明了 `link` 列时改用该列
//! 的链接，使分页与表头改版不影响已有条目。

use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};
use serde_json::{Map, Value};

use crate::error::AdapterError;
use crate::model::{
    AdapterInput, AdapterMeta, AdapterRole, DiscoverOutput, Discovery, DiscoveryHints, Feedback,
    Follow, InputSelector, OutputDecl, Quality,
};
use crate::textutil::{absolute_http, collapse_ws, locator_text, parse_published, url_identity};
use crate::traits::SearchAdapter;

const MAX_COLUMNS: usize = 32;
const MAX_VALUE_CHARS: usize = 512;
const DEFAULT_TABLE: &str = "table";

pub struct TableRecords {
    meta: AdapterMeta,
}

impl Default for TableRecords {
    fn default() -> Self {
        Self::new()
    }
}

impl TableRecords {
    pub fn new() -> Self {
        Self {
            meta: AdapterMeta {
                id: "adapter/table-records".into(),
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

struct Column {
    field: String,
    header: String,
}

struct TableSpec {
    table: String,
    columns: Vec<Column>,
    link: Option<String>,
    title: Option<String>,
    published: Option<String>,
}

#[async_trait]
impl SearchAdapter for TableRecords {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn validate(&self, spec: &Value) -> Result<(), AdapterError> {
        table_spec(spec).map(|_| ())
    }

    async fn discover(
        &self,
        input: &AdapterInput,
        spec: &Value,
    ) -> Result<DiscoverOutput, AdapterError> {
        let parsed = table_spec(spec)?;
        let base = locator_text(&input.locator).to_string();
        let html = String::from_utf8_lossy(&input.view.bytes);
        let document = Html::parse_document(&html);
        let table_sel =
            Selector::parse(&parsed.table).map_err(|_| AdapterError::config("table"))?;
        let mut items = Vec::new();
        for table in document.select(&table_sel) {
            items.extend(rows_of(table, &parsed, &base));
        }
        for (index, item) in items.iter_mut().enumerate() {
            item.hints.rank = Some((index + 1) as u32);
        }
        Ok(DiscoverOutput {
            feedback: if items.is_empty() {
                Some(Feedback {
                    needs_render: Some(strata_model::RenderLevel::L2),
                    quality: Quality::Empty,
                    missing: vec!["row".into()],
                })
            } else {
                None
            },
            items,
        })
    }
}

fn table_spec(spec: &Value) -> Result<TableSpec, AdapterError> {
    let table = spec
        .get("table")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(DEFAULT_TABLE)
        .to_string();
    if Selector::parse(&table).is_err() {
        return Err(AdapterError::config("table 不是合法选择器"));
    }
    let map = spec
        .get("columns")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::config("columns 必须是对象"))?;
    if map.is_empty() {
        return Err(AdapterError::config("columns 不能为空"));
    }
    if map.len() > MAX_COLUMNS {
        return Err(AdapterError::config("columns 字段数量超限"));
    }
    let mut columns = Vec::new();
    let mut headers = Vec::new();
    for (field, header) in map {
        if field.is_empty()
            || !field
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(AdapterError::config(format!("字段名无效: {field}")));
        }
        let Some(header) = header
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return Err(AdapterError::config(format!(
                "columns.{field} 必须是表头文本"
            )));
        };
        // 同一表头映射到两个字段时，取值无从区分，必须在前置校验里拒绝。
        if headers.iter().any(|item| item == header) {
            return Err(AdapterError::config(format!("表头重复: {header}")));
        }
        headers.push(header.to_string());
        columns.push(Column {
            field: field.clone(),
            header: header.to_string(),
        });
    }
    let link = named_column(spec, "link", &columns)?;
    let title = named_column(spec, "title", &columns)?;
    let published = named_column(spec, "publishedAt", &columns)?;
    Ok(TableSpec {
        table,
        columns,
        link,
        title,
        published,
    })
}

/// `link` / `title` / `publishedAt` 取的是 `columns` 的字段名，不是表头文本。
fn named_column(
    spec: &Value,
    key: &str,
    columns: &[Column],
) -> Result<Option<String>, AdapterError> {
    let Some(name) = spec
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return Ok(None);
    };
    if !columns.iter().any(|column| column.field == name) {
        return Err(AdapterError::config(format!(
            "{key} 必须是 columns 里的字段名"
        )));
    }
    Ok(Some(name.to_string()))
}

fn rows_of(table: ElementRef<'_>, spec: &TableSpec, base: &str) -> Vec<Discovery> {
    let Some(headers) = header_cells(table) else {
        return Vec::new();
    };
    let mut index = Map::new();
    for column in &spec.columns {
        if let Some(pos) = headers.iter().position(|text| text == &column.header) {
            index.insert(column.field.clone(), Value::from(pos));
        }
    }
    if index.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::new();
    let mut seen = Vec::new();
    for row in data_rows(table) {
        let cells = row_cells(row);
        let mut extra = Map::new();
        for column in &spec.columns {
            let Some(pos) = index.get(&column.field).and_then(Value::as_u64) else {
                continue;
            };
            let Some(cell) = cells.get(pos as usize) else {
                continue;
            };
            let text = collapse_ws(&cell.text().collect::<String>());
            let value = if spec.link.as_deref() == Some(column.field.as_str()) {
                cell_link(*cell, base).unwrap_or(text)
            } else {
                text
            };
            if value.is_empty() {
                continue;
            }
            let truncated: String = value.chars().take(MAX_VALUE_CHARS).collect();
            extra.insert(column.field.clone(), Value::String(truncated));
        }
        if extra.is_empty() {
            continue;
        }
        let link = spec
            .link
            .as_deref()
            .and_then(|field| extra.get(field))
            .and_then(Value::as_str)
            .and_then(http_url);
        let (locator, identity) = match link {
            Some(url) => {
                let Some(key) = url_identity(&url) else {
                    continue;
                };
                (url, vec![key])
            }
            None => {
                // 行没有链接时，身份取整行投影：只含标题会让不同表格的同名行互相覆盖。
                let fingerprint = row_fingerprint(&extra);
                let locator = absolute_http(base, &format!("#r-{fingerprint}")).unwrap_or_default();
                (locator, vec![format!("ext:row:{fingerprint}")])
            }
        };
        if seen.iter().any(|item: &String| item == &identity[0]) {
            continue;
        }
        seen.push(identity[0].clone());
        let title = spec
            .title
            .as_deref()
            .and_then(|field| extra.get(field))
            .and_then(Value::as_str)
            .map(str::to_string);
        let published = spec.published.as_deref().and_then(|field| {
            extra
                .get(field)
                .and_then(Value::as_str)
                .and_then(|raw| parse_published(raw, None, None))
        });
        items.push(Discovery {
            locator: strata_model::Locator::Url { url: locator },
            identity,
            hints: DiscoveryHints {
                title,
                published_at: published,
                rank: None,
                extra,
                ..DiscoveryHints::default()
            },
            span: None,
            follow: Follow::No,
            materialize: Vec::new(),
        });
    }
    items
}

/// 整行投影的稳定指纹：键排序后逐个写入，避免列顺序变化改变身份。
fn row_fingerprint(extra: &Map<String, Value>) -> String {
    let mut keys: Vec<&String> = extra.keys().collect();
    keys.sort();
    let mut raw = String::new();
    for key in keys {
        raw.push_str(key);
        raw.push('=');
        if let Some(text) = extra[key].as_str() {
            raw.push_str(text);
        }
        raw.push('\n');
    }
    strata_model::content_hash(raw.as_bytes())
}

fn header_cells(table: ElementRef<'_>) -> Option<Vec<String>> {
    let thead = Selector::parse("thead tr").ok()?;
    let tr = Selector::parse("tr").ok()?;
    let th = Selector::parse("th").ok()?;
    let row = table.select(&thead).next().or_else(|| {
        table
            .select(&tr)
            .find(|row| row.select(&th).next().is_some())
    })?;
    let cells = row_cells(row);
    if cells.is_empty() {
        return None;
    }
    Some(
        cells
            .iter()
            .map(|cell| collapse_ws(&cell.text().collect::<String>()))
            .collect(),
    )
}

fn data_rows(table: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    let thead = Selector::parse("thead tr").expect("thead");
    let tbody = Selector::parse("tbody tr").expect("tbody");
    let tr = Selector::parse("tr").expect("tr");
    let th = Selector::parse("th").expect("th");
    if table.select(&thead).next().is_some() {
        let body: Vec<_> = table.select(&tbody).collect();
        if !body.is_empty() {
            return body;
        }
    }
    let rows: Vec<_> = table.select(&tr).collect();
    if rows
        .first()
        .is_some_and(|row| row.select(&th).next().is_some())
    {
        return rows.into_iter().skip(1).collect();
    }
    rows
}

fn row_cells(row: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    row.children()
        .filter_map(ElementRef::wrap)
        .filter(|cell| matches!(cell.value().name(), "td" | "th"))
        .collect()
}

fn cell_link(cell: ElementRef<'_>, base: &str) -> Option<String> {
    let selector = Selector::parse("a[href]").ok()?;
    let href = cell
        .select(&selector)
        .next()
        .and_then(|anchor| anchor.attr("href"))?;
    absolute_http(base, href)
}

fn http_url(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() == "http" || url.scheme() == "https" {
        Some(url.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AdapterInput, ReadonlyView, RenderMode, SourceView};
    use bytes::Bytes;
    use serde_json::json;
    use strata_model::SourceId;

    fn input(html: &str) -> AdapterInput {
        AdapterInput {
            part_id: None,
            view: ReadonlyView {
                bytes: Bytes::copy_from_slice(html.as_bytes()),
                media_type: "text/html".into(),
                charset: None,
                role: None,
            },
            detection: None,
            source: SourceView {
                id: SourceId::new("source/pricing").unwrap(),
                accessor: "http".into(),
                render: RenderMode::Never,
            },
            resource: None,
            locator: strata_model::Locator::Url {
                url: "https://example.com/pricing".into(),
            },
            facet: None,
        }
    }

    #[tokio::test]
    async fn maps_header_names_to_fields() {
        let adapter = TableRecords::new();
        let spec = json!({
            "table": "table",
            "columns": {"subject_official_id": "Model", "value": "Input"},
            "title": "subject_official_id"
        });
        adapter.validate(&spec).unwrap();
        let html = "<table><tr><th>Model</th><th>Input</th></tr>\
                    <tr><td>gpt-4</td><td>$2.50</td></tr></table>";
        let out = adapter.discover(&input(html), &spec).await.unwrap();
        assert_eq!(out.items.len(), 1);
        assert!(out.items[0].identity[0].starts_with("ext:row:"));
        assert_eq!(out.items[0].hints.title.as_deref(), Some("gpt-4"));
        assert_eq!(out.items[0].hints.extra["value"], json!("$2.50"));
        assert_eq!(
            out.items[0].hints.extra["subject_official_id"],
            json!("gpt-4")
        );
    }

    #[tokio::test]
    async fn rows_without_a_link_differ_by_whole_row() {
        let adapter = TableRecords::new();
        let spec = json!({
            "columns": {"name": "Name", "amount": "Value"},
            "title": "name"
        });
        // 同名行取值不同：只有整行指纹才能把二者分开。
        let html = "<table><tr><th>Name</th><th>Value</th></tr>\
                    <tr><td>pro</td><td>1</td></tr><tr><td>pro</td><td>2</td></tr></table>";
        let out = adapter.discover(&input(html), &spec).await.unwrap();
        assert_eq!(out.items.len(), 2);
        assert_ne!(out.items[0].identity[0], out.items[1].identity[0]);
        assert_eq!(
            out.items[0].locator,
            strata_model::Locator::Url {
                url: format!(
                    "https://example.com/pricing#r-{}",
                    out.items[0].identity[0].trim_start_matches("ext:row:")
                )
            }
        );
    }

    #[tokio::test]
    async fn link_and_published_columns_shape_the_discovery() {
        let adapter = TableRecords::new();
        let spec = json!({
            "table": "table#records",
            "columns": {
                "name": "Name",
                "observed_at": "When",
                "source_url": "Link"
            },
            "link": "source_url",
            "title": "name",
            "publishedAt": "observed_at"
        });
        adapter.validate(&spec).unwrap();
        let html =
            "<table id=\"records\"><thead><tr><th>Name</th><th>When</th><th>Link</th></tr></thead>\
                    <tbody><tr><td>Alpha</td><td>2026-01-02T03:04:05Z</td>\
                    <td><a href=\"/a\">src</a></td></tr></tbody></table>";
        let out = adapter.discover(&input(html), &spec).await.unwrap();
        assert_eq!(out.items.len(), 1);
        assert_eq!(
            out.items[0].identity,
            vec!["url:https://example.com/a".to_string()]
        );
        assert_eq!(
            out.items[0].hints.extra["source_url"],
            json!("https://example.com/a")
        );
        assert_eq!(
            out.items[0].hints.published_at.map(|at| at.to_string()),
            Some("2026-01-02T03:04:05Z".to_string())
        );
    }

    #[test]
    fn validation_rejects_unknown_column_references() {
        let adapter = TableRecords::new();
        let spec = json!({
            "columns": {"name": "Name"},
            "link": "missing"
        });
        assert!(adapter.validate(&spec).is_err());
    }
}
