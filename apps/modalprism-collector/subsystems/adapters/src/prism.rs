//! 结构化识别（RawPrism）的宿主侧契约。
//!
//! 适配器不直接依赖框架 Host：识别引擎以 [`Prism`] 的形式注入，未装载时由
//! `None` 表示，全链路按基础检测降级，绝不因识别不可用而阻断采集。

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::error::AdapterError;

/// 一段被识别出来的同类型区域。`type_id` 即 RawPrism 的 TypeId（如 `struct.json`）。
#[derive(Clone, Debug, PartialEq)]
pub struct ScanRegion {
    pub type_id: String,
    pub start: u64,
    pub end: u64,
    pub confidence: f64,
}

/// 类型计数。同一类型在一份输入里可能出现多次，`count` 保留份数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeCount {
    pub id: String,
    pub count: u64,
}

/// 一次扫描的结果视图。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanOutcome {
    pub types: Vec<TypeCount>,
    pub regions: Vec<ScanRegion>,
    pub truncated: bool,
}

impl ScanOutcome {
    /// 从 RawPrism 的 `scan` 返回体解析；字段缺失一律按空处理。
    pub fn from_value(value: &Value) -> Self {
        let types = value
            .get("types")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item.get("id").and_then(Value::as_str)?.to_string();
                        let count = item.get("count").and_then(Value::as_u64).unwrap_or(1);
                        Some(TypeCount { id, count })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let regions = value
            .get("regions")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(ScanRegion {
                            type_id: item.get("type_id").and_then(Value::as_str)?.to_string(),
                            start: item.get("start").and_then(Value::as_u64).unwrap_or(0),
                            end: item.get("end").and_then(Value::as_u64).unwrap_or(0),
                            confidence: item
                                .get("confidence")
                                .and_then(Value::as_f64)
                                .unwrap_or(1.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            types,
            regions,
            truncated: value
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// 是否识别出某个类型（前缀匹配，`text.*` 命中 `text.md`）。
    pub fn has(&self, type_id: &str) -> bool {
        self.type_ids()
            .iter()
            .any(|id| id == type_id || id.starts_with(&format!("{type_id}.")))
    }

    /// 去重后的类型标识，按首次出现顺序。
    pub fn type_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for item in &self.types {
            if !out.contains(&item.id) {
                out.push(item.id.clone());
            }
        }
        for region in &self.regions {
            if !out.contains(&region.type_id) {
                out.push(region.type_id.clone());
            }
        }
        out
    }

    /// 同类区域按开始位置排序后的下标（用于“同类型多份”的稳定编号）。
    pub fn regions_of(&self, type_id: &str) -> Vec<ScanRegion> {
        let mut hits: Vec<ScanRegion> = self
            .regions
            .iter()
            .filter(|region| region.type_id == type_id)
            .cloned()
            .collect();
        hits.sort_by_key(|region| (region.start, region.end));
        hits
    }
}

/// 识别输入的字节上限。超过时只识别前一段，`materialize` 侧必须使用同一截断规则，
/// 否则区域偏移会与原文错位。
pub const PRISM_INPUT_LIMIT: usize = 4 * 1024 * 1024;

/// 把原始字节收成识别输入：按上限截断且回退到 UTF-8 字符边界。
pub fn prism_input(bytes: &[u8]) -> String {
    let mut end = bytes.len().min(PRISM_INPUT_LIMIT);
    while end > 0 && end < bytes.len() && !is_char_boundary(bytes, end) {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn is_char_boundary(bytes: &[u8], index: usize) -> bool {
    // 续字节形如 10xxxxxx；边界即该位置不是续字节。
    index >= bytes.len() || bytes[index] & 0xC0 != 0x80
}

/// 识别引擎。宿主侧实现转发到 RawPrism Guest，测试侧可用内存实现。
#[async_trait]
pub trait Prism: Send + Sync {
    /// 调用引擎方法（`scan` / `materialize` / `query_object` / `custom_detect`）。
    async fn invoke(&self, method: &str, args: Value) -> Result<Value, AdapterError>;

    /// 引擎是否可用。默认可用；占位实现返回 false 以便降级。
    fn available(&self) -> bool {
        true
    }

    /// 扫描文本，返回类型计数与区域。
    async fn scan(&self, text: &str, selectors: &[String]) -> Result<ScanOutcome, AdapterError> {
        self.scan_with(text, selectors, None).await
    }

    /// 带资源上限的扫描。上限映射 RawPrism 的 `ScanConfig`，缺省由引擎决定。
    async fn scan_with(
        &self,
        text: &str,
        selectors: &[String],
        config: Option<&Value>,
    ) -> Result<ScanOutcome, AdapterError> {
        let mut args = json!({ "text": text });
        if !selectors.is_empty() {
            args["selectors"] = json!(selectors);
        }
        if let Some(config) = config {
            args["config"] = config.clone();
        }
        Ok(ScanOutcome::from_value(&self.invoke("scan", args).await?))
    }

    /// 惰性物化：取第 `index` 个命中区域的原文或对象值。
    async fn materialize(
        &self,
        text: &str,
        selectors: &[String],
        index: usize,
        with_value: bool,
    ) -> Result<Value, AdapterError> {
        let args = json!({
            "text": text,
            "selectors": selectors,
            "index": index as u64,
            "with_value": with_value,
        });
        self.invoke("materialize", args).await
    }
}
