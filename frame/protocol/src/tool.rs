use serde::{Deserialize, Serialize};
use serde_json::Value;

/// OpenAI `tools` 条目。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDef {
    Function { function: FunctionDef },
}

impl ToolDef {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self::Function {
            function: FunctionDef {
                name: name.into(),
                description: Some(description.into()),
                parameters,
                strict: None,
            },
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Function { function } => function.name.as_str(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema 对象。
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// 模型侧工具调用。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ToolCallKind,
    pub function: FunctionCall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    Function,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// 参数 JSON 字符串（协议原样）。
    pub arguments: String,
}

impl FunctionCall {
    /// 解析 arguments 为 JSON；保留原始错误语义。
    pub fn parse_args(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_str(&self.arguments)
    }
}

/// `tool_choice` 控制。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(ToolChoiceMode),
    Function(ToolChoiceFunction),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoiceMode {
    None,
    Auto,
    Required,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    #[serde(rename = "type")]
    pub kind: ToolCallKind,
    pub function: ToolChoiceName,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolChoiceName {
    pub name: String,
}

impl ToolChoice {
    pub fn auto() -> Self {
        Self::Mode(ToolChoiceMode::Auto)
    }

    pub fn none() -> Self {
        Self::Mode(ToolChoiceMode::None)
    }

    pub fn required() -> Self {
        Self::Mode(ToolChoiceMode::Required)
    }

    pub fn function(name: impl Into<String>) -> Self {
        Self::Function(ToolChoiceFunction {
            kind: ToolCallKind::Function,
            function: ToolChoiceName { name: name.into() },
        })
    }
}

/// 调度层写回的 tool 结果（尚未折叠为 Message）。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: String,
}

impl ToolResultMessage {
    pub fn into_message(self) -> crate::Message {
        crate::Message::tool_result(self.tool_call_id, self.content)
    }
}
