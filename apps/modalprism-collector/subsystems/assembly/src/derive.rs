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
}

impl Registry {
    pub fn new(services: &Arc<Services>) -> Self {
        Self {
            media_download: MediaDownload {
                services: Arc::clone(services),
            },
        }
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
        Err(format!("产出方未注册: {module}"))
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
}
