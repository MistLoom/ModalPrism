//! 工作流与模型的声明形状。
//!
//! 解析与校验放在一起：界面新建时走 [`WorkflowSpec::parse`] / [`ModelSpec::parse`]，
//! 引擎运行时读到的也是同一份结构，避免出现「界面能存、运行时读不懂」的第三态。

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerOn {
    #[default]
    Commit,
    Manual,
}

impl TriggerOn {
    pub fn label(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Trigger {
    #[serde(default)]
    pub on: TriggerOn,
    /// 只处理这些信源；空表示全部。
    #[serde(default)]
    pub sources: Vec<String>,
    /// 只处理这些资源类型，默认 article。
    #[serde(default = "default_kinds")]
    pub kinds: Vec<String>,
}

fn default_kinds() -> Vec<String> {
    vec!["article".into()]
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            on: TriggerOn::Commit,
            sources: Vec::new(),
            kinds: default_kinds(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Llm,
    Template,
    Http,
}

impl StepKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Template => "template",
            Self::Http => "http",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub kind: StepKind,
    /// `llm` 步骤的提示词模板。
    #[serde(default)]
    pub prompt: Option<String>,
    /// `template` 步骤的模板。
    #[serde(default)]
    pub template: Option<String>,
    /// `http` 步骤的请求。
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
    /// 覆盖工作流默认模型。
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FacetOut {
    /// `struct`、`md` 或 `text`。
    pub kind: String,
    #[serde(default)]
    pub variant: Option<String>,
    /// 正文模板，可引用文章字段与前面步骤的输出。
    pub body: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Outputs {
    #[serde(default)]
    pub facet: Option<FacetOut>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowSpec {
    /// 声明文件里的标识，写入时由存储层补齐。
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub trigger: Trigger,
    /// 默认模型的文件名（不含扩展名）。
    pub model: String,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub outputs: Outputs,
    #[serde(default)]
    pub notify: Vec<Notify>,
}

fn default_enabled() -> bool {
    true
}

impl WorkflowSpec {
    pub fn parse(file: &str, value: &Value) -> Result<Self> {
        let spec: Self = serde_json::from_value(value.clone())
            .map_err(|error| Error::invalid(format!("{file}: 工作流声明无效: {error}")))?;
        spec.validate(file)?;
        Ok(spec)
    }

    pub fn validate(&self, file: &str) -> Result<()> {
        if self.model.trim().is_empty() {
            return Err(Error::invalid(format!("{file}: 缺少 model")));
        }
        if self.steps.is_empty() {
            return Err(Error::invalid(format!("{file}: 至少要有一个步骤")));
        }
        let mut seen = std::collections::BTreeSet::new();
        for step in &self.steps {
            if step.id.trim().is_empty() {
                return Err(Error::invalid(format!("{file}: 步骤缺少 id")));
            }
            if !seen.insert(step.id.as_str()) {
                return Err(Error::invalid(format!("{file}: 步骤 id 重复: {}", step.id)));
            }
            match step.kind {
                StepKind::Llm => {
                    if step.prompt.as_deref().is_none_or(str::is_empty) {
                        return Err(Error::invalid(format!(
                            "{file}: 步骤 {} 缺少 prompt",
                            step.id
                        )));
                    }
                }
                StepKind::Template => {
                    if step.template.as_deref().is_none_or(str::is_empty) {
                        return Err(Error::invalid(format!(
                            "{file}: 步骤 {} 缺少 template",
                            step.id
                        )));
                    }
                }
                StepKind::Http => {
                    let url = step.url.as_deref().unwrap_or_default();
                    if !(url.starts_with("http://") || url.starts_with("https://")) {
                        return Err(Error::invalid(format!(
                            "{file}: 步骤 {} 的 url 必须是 HTTP 地址",
                            step.id
                        )));
                    }
                }
            }
        }
        if let Some(facet) = &self.outputs.facet {
            if !matches!(facet.kind.as_str(), "struct" | "md" | "text") {
                return Err(Error::invalid(format!(
                    "{file}: outputs.facet.kind 只能是 struct、md 或 text"
                )));
            }
            if facet.body.trim().is_empty() {
                return Err(Error::invalid(format!(
                    "{file}: outputs.facet.body 不能为空"
                )));
            }
        }
        for notify in &self.notify {
            if !(notify.url.starts_with("http://") || notify.url.starts_with("https://")) {
                return Err(Error::invalid(format!("{file}: 通知地址必须是 HTTP 地址")));
            }
        }
        Ok(())
    }

    /// 列表用的摘要。声明正文留给详情接口。
    pub fn summary(&self, slug: &str, file: &str) -> Value {
        json!({
            "slug": slug,
            "file": file,
            "name": self.name.clone().unwrap_or_else(|| slug.to_string()),
            "description": self.description,
            "enabled": self.enabled,
            "trigger": {
                "on": self.trigger.on.label(),
                "sources": self.trigger.sources,
                "kinds": self.trigger.kinds,
            },
            "model": self.model,
            "steps": self.steps.iter().map(|step| json!({
                "id": step.id,
                "kind": step.kind.label(),
            })).collect::<Vec<_>>(),
            "notify": self.notify.iter().map(|item| json!({
                "kind": item.kind.label(),
                "on": item.on.iter().map(|on| match on {
                    NotifyOn::Success => "success",
                    NotifyOn::Failure => "failure",
                }).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "outputs": self.outputs.facet.as_ref().map(|facet| json!({
                "kind": facet.kind,
                "variant": facet.variant,
            })),
        })
    }

    /// 文档形态，供编辑抽屉回填。密钥不在其中。
    pub fn document(&self) -> Value {
        prune(serde_json::to_value(self).unwrap_or(Value::Null))
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
