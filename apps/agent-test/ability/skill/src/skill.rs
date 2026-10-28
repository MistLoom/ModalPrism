use serde::{Deserialize, Serialize};

use crate::error::SkillError;

/// Skill 元数据（对应 SKILL.md YAML frontmatter 子集）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub disable_model_invocation: bool,
}

impl SkillMeta {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, SkillError> {
        let name = name.into();
        if name.is_empty() {
            return Err(SkillError::InvalidMeta("name 不能为空".into()));
        }
        Ok(Self {
            name,
            description: description.into(),
            disable_model_invocation: false,
        })
    }
}

/// 已加载的 Skill。
#[derive(Clone, Debug)]
pub struct Skill {
    pub meta: SkillMeta,
    /// Markdown 正文（不含 frontmatter）。
    pub body: String,
    /// 来源目录（若从文件系统加载）。
    pub root: Option<std::path::PathBuf>,
}

impl Skill {
    pub fn from_parts(meta: SkillMeta, body: impl Into<String>) -> Self {
        Self {
            meta,
            body: body.into(),
            root: None,
        }
    }

    /// 生成可注入提示词的材料化文本。
    pub fn render_prompt_section(&self) -> String {
        format!(
            "## Skill: {}\n\n{}\n\n{}",
            self.meta.name,
            self.meta.description,
            self.body.trim()
        )
    }

    /// 简短目录条目（供模型发现，不含全文）。
    pub fn render_catalog_entry(&self) -> String {
        format!("- {}: {}", self.meta.name, self.meta.description)
    }
}
