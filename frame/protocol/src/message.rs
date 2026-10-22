use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 消息角色（OpenAI Chat Completions 兼容）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    Developer,
}

/// 多模态内容片段；纯文本场景可直接用 [`Message::text`]。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    #[serde(other)]
    Unknown,
}

/// 对话消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// 文本或分段内容；tool 结果亦写入此字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// assistant 发起的工具调用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<crate::ToolCall>>,
    /// tool 角色消息关联的调用 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// 消息内容：兼容纯字符串与分段数组。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    ContentPart::Unknown => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::text(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(Role::Assistant, content)
    }

    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(MessageContent::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<crate::ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: None,
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(MessageContent::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    pub fn content_text(&self) -> Option<String> {
        self.content.as_ref().map(|c| c.as_text())
    }
}

/// 补齐 assistant `tool_calls` 之后缺失的 tool 消息，满足 OpenAI 兼容 API 约束。
///
/// 当 dispatch 失败或回合中断时，session 可能留下未配对的 tool_calls，上游会返回 400。
pub fn repair_tool_call_chain(messages: Vec<Message>) -> Vec<Message> {
    use std::collections::HashSet;

    let mut out = Vec::with_capacity(messages.len());
    let mut i = 0;
    while i < messages.len() {
        let msg = &messages[i];
        if msg.role == Role::Assistant {
            if let Some(calls) = msg.tool_calls.as_ref().filter(|c| !c.is_empty()) {
                out.push(msg.clone());
                i += 1;
                let mut got = HashSet::new();
                while i < messages.len() && messages[i].role == Role::Tool {
                    if let Some(id) = &messages[i].tool_call_id {
                        got.insert(id.as_str());
                    }
                    out.push(messages[i].clone());
                    i += 1;
                }
                for call in calls {
                    if !got.contains(call.id.as_str()) {
                        out.push(Message::tool_result(
                            call.id.clone(),
                            json_content(&serde_json::json!({
                                "error": "tool result missing; session repaired"
                            })),
                        ));
                    }
                }
                continue;
            }
        }
        out.push(msg.clone());
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FunctionCall, ToolCall, ToolCallKind};

    fn sample_call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: ToolCallKind::Function,
            function: FunctionCall {
                name: "read_file".into(),
                arguments: "{}".into(),
            },
        }
    }

    #[test]
    fn repair_inserts_missing_tool_messages() {
        let messages = vec![
            Message::user("hi"),
            Message {
                role: Role::Assistant,
                content: None,
                name: None,
                tool_calls: Some(vec![sample_call("c1"), sample_call("c2")]),
                tool_call_id: None,
            },
            Message::tool_result("c1", "ok"),
        ];
        let fixed = repair_tool_call_chain(messages);
        assert_eq!(fixed.len(), 4);
        assert_eq!(fixed[2].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(fixed[3].tool_call_id.as_deref(), Some("c2"));
    }
}

/// 将任意 JSON 序列化为 tool 结果文本。
pub fn json_content(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}
