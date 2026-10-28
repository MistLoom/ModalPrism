use ai_protocol::{Message, Role};

use crate::section::{PromptSection, SectionKind};

/// 流式装配提示词，最终产出 OpenAI 兼容消息列表。
#[derive(Clone, Debug, Default)]
pub struct PromptBuilder {
    system_sections: Vec<PromptSection>,
    messages: Vec<Message>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加 system 段落（构建时合并为一条 system 消息）。
    pub fn system(&mut self, body: impl Into<String>) -> &mut Self {
        self.system_sections
            .push(PromptSection::new(SectionKind::System, body));
        self
    }

    pub fn system_section(&mut self, section: PromptSection) -> &mut Self {
        self.system_sections.push(section);
        self
    }

    pub fn context(&mut self, title: impl Into<String>, body: impl Into<String>) -> &mut Self {
        self.system_sections
            .push(PromptSection::new(SectionKind::Context, body).with_title(title));
        self
    }

    pub fn instruction(&mut self, body: impl Into<String>) -> &mut Self {
        self.system_sections
            .push(PromptSection::new(SectionKind::Instruction, body));
        self
    }

    /// 注入已材料化的 Skill 正文（由 Ability `skill` 经 client 取得）。
    pub fn skill(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        body: impl Into<String>,
    ) -> &mut Self {
        let name = name.into();
        let rendered = format!(
            "## Skill: {}\n\n{}\n\n{}",
            name,
            description.into(),
            body.into().trim()
        );
        self.system_sections.push(
            PromptSection::new(SectionKind::Skill, rendered).with_title(format!("Skill:{name}")),
        );
        self
    }

    /// 仅注入 Skill 目录（名称 + 描述），不含正文。
    pub fn skill_catalog(&mut self, catalog_text: impl Into<String>) -> &mut Self {
        self.system_sections.push(
            PromptSection::new(SectionKind::Skill, catalog_text).with_title("Available Skills"),
        );
        self
    }

    pub fn user(&mut self, content: impl Into<String>) -> &mut Self {
        self.messages.push(Message::user(content));
        self
    }

    pub fn assistant(&mut self, content: impl Into<String>) -> &mut Self {
        self.messages.push(Message::assistant(content));
        self
    }

    pub fn tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
    ) -> &mut Self {
        self.messages
            .push(Message::tool_result(tool_call_id, content));
        self
    }

    pub fn message(&mut self, message: Message) -> &mut Self {
        self.messages.push(message);
        self
    }

    pub fn messages(&mut self, messages: impl IntoIterator<Item = Message>) -> &mut Self {
        self.messages.extend(messages);
        self
    }

    /// 合并 system 段落 + 后续消息。
    pub fn build(&self) -> Vec<Message> {
        let mut out = Vec::new();
        if let Some(system) = self.render_system() {
            out.push(Message::system(system));
        }
        out.extend(self.messages.clone());
        out
    }

    /// 仅渲染合并后的 system 文本。
    pub fn render_system(&self) -> Option<String> {
        if self.system_sections.is_empty() {
            return None;
        }
        let text = self
            .system_sections
            .iter()
            .map(PromptSection::render)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.is_empty() { None } else { Some(text) }
    }

    /// 调试用纯文本展开（非协议格式）。
    pub fn render_debug(&self) -> String {
        let mut parts = Vec::new();
        if let Some(system) = self.render_system() {
            parts.push(format!("[system]\n{system}"));
        }
        for msg in &self.messages {
            let role = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::Developer => "developer",
            };
            let content = msg.content_text().unwrap_or_default();
            parts.push(format!("[{role}]\n{content}"));
        }
        parts.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_system_and_user() {
        let mut b = PromptBuilder::new();
        b.system("你是助手").context("facts", "a=1").user("hello");
        let msgs = b.build();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert!(msgs[0].content_text().unwrap().contains("你是助手"));
        assert!(msgs[0].content_text().unwrap().contains("a=1"));
        assert_eq!(msgs[1].role, Role::User);
    }

    #[test]
    fn injects_skill() {
        let mut b = PromptBuilder::new();
        b.skill("demo", "desc", "do it carefully").user("go");
        let system = b.render_system().unwrap();
        assert!(system.contains("demo"));
        assert!(system.contains("do it carefully"));
    }
}
