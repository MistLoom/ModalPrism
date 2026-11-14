//! 派生任务的执行侧接线：产出方注册表与内置的 media-download。
//!
//! 声明式派生（`strata.gateway.derivations`）由数据层在发布时入队；这里补上执行端——
//! 租约任务、按 `producer.module` 分派给注册表里的产出方。未注册的产出方报错退回，
//! 由执行框架带 60 秒退避，不会形成重试风暴。接入新产出方只需要再实现一个
//! [`DerivationProducer`] 并在这里登记模块名。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use strata_client::{DerivationInput, DerivationProducer, DerivationRun};

use crate::Services;

/// 产出方注册表：按 `producer.module` 前缀匹配。
pub struct Registry {
    media_download: MediaDownload,
    /// 嵌入产出计数，供 vectorize.ready 事件聚合。
    embedding_hits: std::sync::atomic::AtomicUsize,
}

impl Registry {
    pub fn new(services: &Arc<Services>) -> Self {
        Self {
            media_download: MediaDownload {
                services: Arc::clone(services),
            },
            embedding_hits: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// 本次派生轮里成功产出的嵌入数。
    pub fn embedding_hits(&self) -> usize {
        self.embedding_hits
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait]
impl DerivationProducer for Registry {
    async fn produce(
        &self,
        input: DerivationInput<'_>,
    ) -> Result<Option<strata_model::Body>, String> {
        let module = input.task.producer.module.as_str();
        if module.starts_with("media-download") {
            return self.media_download.produce(input).await;
        }
        if module.starts_with("derive-embedding") {
            let produced = embedding(input)?;
            self.embedding_hits
                .fetch_add(usize::from(produced.is_some()), std::sync::atomic::Ordering::Relaxed);
            return Ok(produced);
        }
        Err(format!("产出方未注册: {module}"))
    }
}

/// 向量写入阶段的嵌入产出方：小而确定的特征哈希嵌入，纯 Rust、跨平台、
/// 不依赖外部服务或 OS 专用 API，足够支撑增量写入、跳过与清理。
/// 输入必须是文本 facet；非文本/空文本没有可嵌入内容，返回 `None`（任务确认跳过），
/// 不会把空值冒充成向量。
const EMBEDDING_DIMS: usize = 64;

fn embedding(input: DerivationInput<'_>) -> Result<Option<strata_model::Body>, String> {
    let text = input
        .input
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "非文本或空输入，无法嵌入".to_string())?;
    Ok(Some(strata_model::Body::Struct {
        value: serde_json::json!({
            "model": strata_client::EMBEDDING_VARIANT,
            "vector": hash_embedding(text),
        }),
    }))
}

/// 特征哈希：对每个空白分词做 FNV-1a 到 [0, DIMS) 的槽位，按哈希奇偶决定正负号，
/// 累加后 L2 归一化。确定性：同文本同向量（同内容哈希 → 同向量，增量跳过才成立）。
fn hash_embedding(text: &str) -> Vec<f32> {
    let mut slots = [0f32; EMBEDDING_DIMS];
    for token in text.split_whitespace() {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in token.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let index = (hash % EMBEDDING_DIMS as u64) as usize;
        let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
        slots[index] += sign;
    }
    let norm = slots.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        slots.iter().map(|v| v / norm).collect()
    } else {
        slots.to_vec()
    }
}

/// 内置产出方 media-download：读输入 facet 里的地址，经出站治理下载字节，
/// 暂存后写成二进制 facet。受 `media.mode`、`media.kinds`、`media.max_size` 约束。
struct MediaDownload {
    services: Arc<Services>,
}

fn binary_kind(kind: &str) -> bool {
    matches!(kind, "image" | "audio" | "video" | "file")
}

/// 配置里的尺寸写法（`512KiB` 一类）换成字节数；解析失败按不限制处理。
fn parse_size(text: &str) -> Option<u64> {
    let digits = text
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    let value: u64 = digits.parse().ok()?;
    let unit = text[digits.len()..].trim();
    let factor: u64 = match unit {
        "" | "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    value.checked_mul(factor)
}

#[async_trait]
impl DerivationProducer for MediaDownload {
    async fn produce(
        &self,
        input: DerivationInput<'_>,
    ) -> Result<Option<strata_model::Body>, String> {
        // 只有二进制产出种类才写暂存部件，其余交给真正的模型产出方。
        if !binary_kind(&input.task.produces) {
            return Ok(None);
        }
        let Some(url) = input
            .input
            .value
            .as_ref()
            .and_then(|value| {
                value
                    .get("url")
                    .or_else(|| value.pointer("/locator/url"))
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_owned)
        else {
            return Ok(None);
        };
        // media.* 是信源作用域，不随系统策略覆盖；派生不依附具体信源，故直接读系统策略里的声明值。
        let snapshot = self.services.config().applied();
        let system = &snapshot.system;
        if system
            .pointer("/policy/media/mode")
            .and_then(serde_json::Value::as_str)
            != Some("download")
        {
            return Ok(None);
        }
        let kinds: Vec<String> = system
            .pointer("/policy/media/kinds")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if !kinds.iter().any(|item| item == &input.task.produces) {
            return Ok(None);
        }
        let limit = system
            .pointer("/policy/media/max_size")
            .and_then(serde_json::Value::as_str)
            .and_then(parse_size);
        if let Some(limit) = limit
            && input.input.size > limit
        {
            // 输入已声明的尺寸超限：确认跳过，不再下载。
            return Ok(None);
        }
        let egress_policy = crate::transport::policy_of(system);
        let request = accessors::HttpRequest {
            method: "GET".into(),
            url,
            headers: Vec::new(),
            body: Bytes::new(),
        };
        let response = self
            .services
            .egress_gateway()
            .request_governed(&egress_policy, request)
            .await
            .map_err(|error| error.message)?;
        if response.status < 200 || response.status >= 300 {
            return Err(format!("媒体下载返回 {}", response.status));
        }
        if let Some(limit) = limit
            && (response.body.len() as u64) > limit
        {
            return Ok(None);
        }
        let part = self
            .services
            .strata()
            .stage(&response.body)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Some(strata_model::Body::Staged { part }))
    }
}

impl Services {
    /// 执行一轮派生：没有声明时是空操作，接电但不产生负载。
    pub async fn run_derivations(self: &Arc<Self>) -> Result<DerivationRun, crate::Error> {
        let options = strata_client::Options::from_system(&self.config().applied().system)
            .map_err(|error| crate::Error::new(error.to_string()))?;
        if options.derivations.is_empty() {
            return Ok(DerivationRun::default());
        }
        let registry = Registry::new(self);
        let run = self
            .strata()
            .run_derivations(
                &registry,
                strata_model::QueueName::Render,
                16,
                Duration::from_secs(120),
            )
            .await
            .map_err(|error| crate::Error::new(error.to_string()))?;
        // 向量写入阶段：本轮确有嵌入成功写入才发 vectorize.ready；
        // 失败/跳过/取消不计入，未触发该阶段时计数恒为 0。
        let vectors = registry.embedding_hits();
        if vectors > 0 {
            self.events.vectorize_ready(vectors);
        }
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_sizes() {
        assert_eq!(parse_size("512KiB"), Some(512 * 1024));
        assert_eq!(parse_size("8MiB"), Some(8 * 1024 * 1024));
        assert_eq!(parse_size("100B"), Some(100));
        assert_eq!(parse_size("3"), Some(3));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("TiB"), None);
    }

    #[test]
    fn only_binary_kinds_are_staged() {
        assert!(binary_kind("image"));
        assert!(binary_kind("file"));
        assert!(!binary_kind("struct"));
        assert!(!binary_kind("md"));
    }

    /// 嵌入确定性：同文本同向量（同内容哈希 → 同向量，增量跳过才成立）；
    /// 不同文本方向不同；向量 L2 归一化；维度固定。
    #[test]
    fn hash_embedding_is_deterministic_and_normalized() {
        let first = hash_embedding("向量写入阶段 验收正文");
        let again = hash_embedding("向量写入阶段 验收正文");
        assert_eq!(first, again, "同文本必须同向量");
        assert_eq!(first.len(), EMBEDDING_DIMS);
        let other = hash_embedding("另一段完全不同的文字");
        assert_ne!(first, other);
        let norm: f32 = first.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "L2 归一化后模长为 1，实际 {norm}");
    }
}
