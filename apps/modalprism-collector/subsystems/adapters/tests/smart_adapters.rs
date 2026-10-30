//! 智能适配器：结构推断、兜底条目与多份区域拆分。

use std::sync::Arc;

use adapters::{
    AdapterError, AdapterInput, ContentAdapter, Detection, DraftBody, Prism, ReadonlyView, RenderMode,
    SearchAdapter, SmartContent, SmartDiscover, SourceView,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{Value, json};
use strata_model::{Locator, PartRole, SourceId};

struct FakePrism;

#[async_trait]
impl Prism for FakePrism {
    async fn invoke(&self, method: &str, args: Value) -> Result<Value, AdapterError> {
        match method {
            "scan" => Ok(json!({
                "types": [
                    {"id": "struct.json", "count": 1},
                    {"id": "text.md", "count": 2}
                ],
                "regions": [
                    {"type_id": "struct.json", "start": 0, "end": 10, "confidence": 0.95},
                    {"type_id": "text.md", "start": 11, "end": 20, "confidence": 0.9},
                    {"type_id": "text.md", "start": 21, "end": 30, "confidence": 0.9}
                ],
                "truncated": false
            })),
            "materialize" => {
                let selectors = args
                    .get("selectors")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let type_id = selectors
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let index = args.get("index").and_then(Value::as_u64).unwrap_or(0);
                let text = match type_id {
                    "struct.json" => "{\"a\": 1}".to_string(),
                    other => format!("# {other} #{index}"),
                };
                Ok(json!({ "type_id": type_id, "text": text }))
            }
            other => Err(AdapterError::permanent(format!("未知方法 {other}"))),
        }
    }
}

fn input(text: &str, media: &str, detection: Option<Detection>) -> AdapterInput {
    AdapterInput {
        part_id: None,
        view: ReadonlyView {
            bytes: Bytes::from(text.to_string().into_bytes()),
            media_type: media.into(),
            charset: Some("utf-8".into()),
            role: Some(PartRole::Document),
        },
        detection,
        source: SourceView {
            id: SourceId::new("demo").expect("源标识"),
            accessor: "accessor/http".into(),
            render: RenderMode::Auto,
        },
        resource: None,
        locator: Locator::Url {
            url: "https://example.com/data".into(),
        },
        facet: None,
    }
}

#[tokio::test]
async fn discover_infers_items_from_json_container() {
    let adapter = SmartDiscover::new(None);
    let body = json!({
        "data": [
            {"title": "A", "url": "https://example.com/a", "publishedAt": "2026-01-02T03:04:05Z"},
            {"title": "B", "url": "https://example.com/b"},
            {"title": "C", "url": "https://example.com/c"}
        ]
    })
    .to_string();
    let output = adapter
        .discover(&input(&body, "application/json", None), &Value::Null)
        .await
        .expect("发现");
    assert_eq!(output.items.len(), 3, "{:?}", output.items);
    assert!(output.items[0].hints.title.as_deref() == Some("A"));
    assert!(output.items[0].hints.published_at.is_some(), "发布时间应被解析");
    assert!(output.feedback.is_none(), "结构清晰的文档不应降级");
}

#[tokio::test]
async fn discover_never_returns_zero_items() {
    let adapter = SmartDiscover::new(None);
    // 空白文档没有任何可抽取内容：仍必须留下一条，并标成「只是原样收下」。
    let output = adapter
        .discover(&input("   \n\t ", "text/plain", None), &Value::Null)
        .await
        .expect("发现");
    assert!(!output.items.is_empty(), "识别不到也必须留下一条");
    assert_eq!(
        output.feedback.map(|item| item.quality),
        Some(adapters::Quality::Thin)
    );
}

#[tokio::test]
async fn discover_splits_plain_text_into_blocks() {
    let adapter = SmartDiscover::new(None);
    let body = "first paragraph with enough words\n\nsecond paragraph also long enough";
    let output = adapter
        .discover(&input(body, "text/plain", None), &Value::Null)
        .await
        .expect("发现");
    assert_eq!(output.items.len(), 2, "{:?}", output.items);
}

#[tokio::test]
async fn content_splits_every_region_into_its_own_facet() {
    let adapter = SmartContent::new(Some(Arc::new(FakePrism)));
    let detection = Detection {
        media_type: "text/plain".into(),
        charset: Some("utf-8".into()),
        confidence: 0.9,
        traits: vec!["prism".into(), "struct.json".into(), "text.md".into()],
    };
    let output = adapter
        .adapt(&input("{\"a\":1}\n# one\n# two\n", "text/plain", Some(detection)), &Value::Null)
        .await
        .expect("拆分");
    let variants: Vec<String> = output
        .facets
        .iter()
        .filter_map(|facet| facet.variant.clone())
        .collect();
    assert_eq!(variants, vec!["r0", "r1", "r2", "prism"], "{variants:?}");
    let index = output
        .facets
        .iter()
        .find(|facet| facet.variant.as_deref() == Some("prism"))
        .expect("区域索引");
    let DraftBody::Struct(value) = &index.body else {
        panic!("索引应为结构化面");
    };
    let regions = value["regions"].as_array().expect("regions");
    assert_eq!(regions.len(), 3, "同类型多份必须各自成条");
    assert_eq!(regions[1]["typeId"], json!("text.md"));
    assert_eq!(regions[1]["variant"], json!("r1"));
}
