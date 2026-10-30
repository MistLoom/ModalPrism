//! 基于 RawPrism 的检测器。
//!
//! 基础检测器只看到 MIME（html/json/xml）；这里把 RawPrism 识别出的 TypeId
//! （`struct.json`、`text.md`、`markup.html`、`entity.url` 等）投影成 `traits`，
//! 让 `route.rs` 的 `when.traits` / `when.detected` 真正可用。
//!
//! 识别失败一律退回基础检测：识别引擎是增强项，不能让采集因它不可用而中断。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::builtin::BasicDetector;
use crate::error::AdapterError;
use crate::model::{Declared, Detection, ReadonlyView, RegionHit};
use crate::prism::{Prism, ScanOutcome, prism_input};
use crate::traits::Detector;

/// 扫描资源上限：区域数与时间预算，避免超长文档拖住一次采集。
fn scan_limits() -> Value {
    json!({ "max_regions": 512, "time_budget_ms": 2_000 })
}

/// TypeId → 媒体类型。用于基础检测只给出弱判定时的细化。
fn media_of(type_id: &str) -> Option<&'static str> {
    match type_id {
        "struct.json" | "struct.ndjson" => Some("application/json"),
        "struct.yaml" => Some("text/yaml"),
        "struct.toml" => Some("application/toml"),
        "struct.csv" => Some("text/csv"),
        "struct.xml" => Some("application/xml"),
        "markup.html" => Some("text/html"),
        "text.md" => Some("text/markdown"),
        _ => None,
    }
}

pub struct PrismDetector {
    prism: Arc<dyn Prism>,
    /// 同一份输入的扫描结果只算一次：`detect` 与 `regions` 共用。
    cache: Mutex<Option<(u64, Arc<ScanOutcome>)>>,
}

impl PrismDetector {
    pub fn new(prism: Arc<dyn Prism>) -> Self {
        Self {
            prism,
            cache: Mutex::new(None),
        }
    }

    fn fingerprint(bytes: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        bytes.len().hash(&mut hasher);
        let head = &bytes[..bytes.len().min(4_096)];
        head.hash(&mut hasher);
        hasher.finish()
    }

    /// 扫描并缓存。缓存键只看输入指纹，同一份内容只扫一次。
    async fn scan(&self, view: &ReadonlyView) -> Option<Arc<ScanOutcome>> {
        if !self.prism.available() {
            return None;
        }
        let key = Self::fingerprint(&view.bytes);
        let cached = {
            let guard = self.cache.lock().ok()?;
            guard
                .as_ref()
                .filter(|(cached, _)| *cached == key)
                .map(|(_, outcome)| Arc::clone(outcome))
        };
        if let Some(outcome) = cached {
            return Some(outcome);
        }
        let text = prism_input(&view.bytes);
        if text.trim().is_empty() {
            return None;
        }
        let outcome = Arc::new(
            self.prism
                .scan_with(&text, &[], Some(&scan_limits()))
                .await
                .ok()?,
        );
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some((key, Arc::clone(&outcome)));
        }
        Some(outcome)
    }
}

fn refine_media(basic: &str, confidence: f64, outcome: &ScanOutcome) -> String {
    // 基础检测强判定（魔数 / 结构首字符）时以它为准：RawPrism 的区域判断不能把
    // 一个 html 文档整体改写成 markdown。
    let strong =
        confidence >= 0.9 && !matches!(basic, "text/plain" | "application/octet-stream" | "");
    if strong {
        return basic.to_string();
    }
    for id in outcome.type_ids() {
        if let Some(media) = media_of(&id) {
            return media.to_string();
        }
    }
    basic.to_string()
}

#[async_trait]
impl Detector for PrismDetector {
    fn id(&self) -> &str {
        "detector/prism"
    }

    fn version(&self) -> &str {
        "1"
    }

    async fn detect(
        &self,
        view: &ReadonlyView,
        declared: &Declared,
    ) -> Result<Detection, AdapterError> {
        let basic = BasicDetector.detect_view(view, declared);
        let Some(outcome) = self.scan(view).await else {
            return Ok(basic);
        };
        let mut traits = basic.traits.clone();
        for id in outcome.type_ids() {
            if !traits.contains(&id) {
                traits.push(id);
            }
        }
        if !traits.iter().any(|item| item == "prism") {
            traits.push("prism".into());
        }
        traits.sort();
        traits.dedup();
        let media_type = refine_media(&basic.media_type, basic.confidence, &outcome);
        let seen = outcome
            .regions
            .iter()
            .map(|region| region.confidence)
            .fold(basic.confidence, f64::max);
        Ok(Detection {
            media_type,
            charset: basic.charset,
            confidence: seen,
            traits,
        })
    }

    /// `wanted` 为空或含 `prism` 时返回全部区域；否则按类型前缀过滤。
    async fn regions(&self, view: &ReadonlyView, wanted: &[String]) -> Vec<RegionHit> {
        let all = wanted.is_empty() || wanted.iter().any(|item| item == "prism");
        let Some(outcome) = self.scan(view).await else {
            return Vec::new();
        };
        outcome
            .regions
            .iter()
            .filter(|region| {
                all || wanted.iter().any(|item| {
                    region.type_id == *item || region.type_id.starts_with(&format!("{item}."))
                })
            })
            .map(|region| RegionHit {
                r#type: region.type_id.clone(),
                part_id: String::new(),
                start: region.start,
                end: region.end,
                confidence: region.confidence,
            })
            .collect()
    }
}
