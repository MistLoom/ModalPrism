//! 模型与通知目标的声明形状。
//!
//! 模型声明（名称、调用方式、端点、环境变量名）与通知目标（webhook / 飞书）
//! 由内容投递直接使用，解析与校验放在一起，避免出现「界面能存、运行时读不懂」的第三态。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};

/// 模型的调用方式。`api` 与 `local` 都走 OpenAI 兼容的 chat completions，
/// 区别只在默认值与密钥来源；`cli` 交给本机的命令行模型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Api,
    Local,
    Cli,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Local => "local",
            Self::Cli => "cli",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSpec {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub mode: Mode,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// 密钥所在的环境变量名。声明里不写密钥本身。
    #[serde(default)]
    pub key_env: Option<String>,
    /// 直接登记的密钥值：仅驻留内存用于组装请求头，不序列化、不写入任何声明文件。
    #[serde(default, skip_serializing)]
    pub key_value: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// `cli` 模式的命令与参数，提示词写入标准输入。
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

impl ModelSpec {
    pub fn parse(file: &str, value: &Value) -> Result<Self> {
        let spec: Self = serde_json::from_value(value.clone())
            .map_err(|error| Error::invalid(format!("{file}: 模型声明无效: {error}")))?;
        spec.validate(file)?;
        Ok(spec)
    }

    pub fn validate(&self, file: &str) -> Result<()> {
        match self.mode {
            Mode::Api | Mode::Local => {
                let endpoint = self
                    .endpoint
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| Error::invalid(format!("{file}: 缺少 endpoint")))?;
                if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
                    return Err(Error::invalid(format!("{file}: endpoint 必须是 HTTP 地址")));
                }
                if self.model.as_deref().is_none_or(str::is_empty) {
                    return Err(Error::invalid(format!("{file}: 缺少 model")));
                }
            }
            Mode::Cli => {
                if self
                    .command
                    .first()
                    .is_none_or(|item| item.trim().is_empty())
                {
                    return Err(Error::invalid(format!("{file}: cli 模式缺少 command")));
                }
            }
        }
        if let Some(timeout) = &self.timeout
            && config::duration(timeout).is_none()
        {
            return Err(Error::invalid(format!("{file}: timeout 形如 30s、2m")));
        }
        if let Some(temperature) = self.temperature
            && !(0.0..=2.0).contains(&temperature)
        {
            return Err(Error::invalid(format!("{file}: temperature 超出 0~2")));
        }
        Ok(())
    }

    /// 界面上的简要描述，不含密钥。
    pub fn summary(&self, slug: &str) -> Value {
        json!({
            "slug": slug,
            "name": self.name.clone().unwrap_or_else(|| slug.to_string()),
            "mode": self.mode.label(),
            "model": self.model,
            "endpoint": self.endpoint,
            "keyEnv": self.key_env,
            "command": self.command,
        })
    }

    /// 文档形态，供编辑抽屉回填。只有环境变量名，没有密钥。
    pub fn document(&self) -> Value {
        prune(serde_json::to_value(self).unwrap_or(Value::Null))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyKind {
    Webhook,
    Feishu,
}

impl NotifyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Feishu => "feishu",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyOn {
    Success,
    Failure,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notify {
    pub kind: NotifyKind,
    pub url: String,
    /// 默认成功与失败都通知。
    #[serde(default)]
    pub on: Vec<NotifyOn>,
    #[serde(default)]
    pub title: Option<String>,
}

impl Notify {
    pub fn wants(&self, success: bool) -> bool {
        if self.on.is_empty() {
            return true;
        }
        self.on.iter().any(|item| {
            matches!(
                (item, success),
                (NotifyOn::Success, true) | (NotifyOn::Failure, false)
            )
        })
    }
}

/// 去掉 `null` 字段，让回填到编辑器的声明与手写风格一致。
fn prune(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter_map(|(key, item)| {
                    let item = prune(item);
                    (!item.is_null()).then_some((key, item))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(prune).collect()),
        other => other,
    }
}
