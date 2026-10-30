//! `detector/prism`：TypeId → traits 的投影与降级行为。

use std::sync::Arc;

use adapters::{AdapterError, Declared, Detector, Prism, PrismDetector, ReadonlyView};
use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{Value, json};
use strata_model::PartRole;

struct FakePrism {
    available: bool,
}

#[async_trait]
impl Prism for FakePrism {
    async fn invoke(&self, method: &str, _args: Value) -> Result<Value, AdapterError> {
        match method {
            "scan" => Ok(json!({
                "types": [
                    {"id": "struct.json", "count": 1},
                    {"id": "text.md", "count": 2}
                ],
                "regions": [
                    {"type_id": "struct.json", "start": 0, "end": 12, "confidence": 0.97},
                    {"type_id": "text.md", "start": 13, "end": 30, "confidence": 0.8},
                    {"type_id": "text.md", "start": 31, "end": 50, "confidence": 0.7}
                ],
                "truncated": false
            })),
            other => Err(AdapterError::permanent(format!("未知方法 {other}"))),
        }
    }

    fn available(&self) -> bool {
        self.available
    }
}

fn view(text: &str) -> ReadonlyView {
    ReadonlyView {
        bytes: Bytes::from(text.to_string().into_bytes()),
        media_type: "text/plain".into(),
        charset: Some("utf-8".into()),
        role: Some(PartRole::Document),
    }
}

fn declared() -> Declared {
    Declared {
        media_type: "text/plain".into(),
        charset: Some("utf-8".into()),
        role: PartRole::Document,
    }
}

#[tokio::test]
async fn detect_projects_type_ids_into_traits() {
    let detector = PrismDetector::new(Arc::new(FakePrism { available: true }));
    let detection = detector
        .detect(&view("{\"a\": 1}\n# md\n"), &declared())
        .await
        .expect("识别");
    for expected in ["struct.json", "text.md", "prism"] {
        assert!(
            detection.traits.iter().any(|item| item == expected),
            "缺少 trait {expected}: {:?}",
            detection.traits
        );
    }
    // 弱判定（text/plain）时按识别结果细化媒体类型。
    assert_eq!(detection.media_type, "application/json");
}

#[tokio::test]
async fn regions_keep_every_occurrence() {
    let detector = PrismDetector::new(Arc::new(FakePrism { available: true }));
    let hits = detector.regions(&view("a b c"), &[]).await;
    assert_eq!(hits.len(), 3, "{hits:?}");
    let md: Vec<_> = hits.iter().filter(|hit| hit.r#type == "text.md").collect();
    assert_eq!(md.len(), 2, "同类型多份必须保留");
    assert!(md[0].start < md[1].start, "区域按出现位置有序");
}

#[tokio::test]
async fn unavailable_engine_falls_back_to_basic_detection() {
    let detector = PrismDetector::new(Arc::new(FakePrism { available: false }));
    let detection = detector
        .detect(&view("<html><body>x</body></html>"), &declared())
        .await
        .expect("回退");
    assert!(
        !detection.traits.iter().any(|item| item == "prism"),
        "引擎不可用时不得打上 prism 标记: {:?}",
        detection.traits
    );
    assert_eq!(detection.media_type, "text/html");
}
