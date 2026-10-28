use std::fmt;

/// 提示词逻辑段落（组装前的中间表示）。
#[derive(Clone, Debug)]
pub struct PromptSection {
    pub kind: SectionKind,
    pub title: Option<String>,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    System,
    Context,
    Skill,
    Instruction,
    Custom,
}

impl PromptSection {
    pub fn new(kind: SectionKind, body: impl Into<String>) -> Self {
        Self {
            kind,
            title: None,
            body: body.into(),
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn render(&self) -> String {
        match &self.title {
            Some(title) => format!("### {title}\n\n{}", self.body.trim()),
            None => self.body.trim().to_string(),
        }
    }
}

impl fmt::Display for PromptSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}
