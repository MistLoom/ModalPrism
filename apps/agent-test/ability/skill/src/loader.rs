use std::path::{Path, PathBuf};

use crate::error::SkillError;
use crate::frontmatter::parse_skill_md;
use crate::skill::Skill;

/// 从目录树加载 Skill。
#[derive(Clone, Debug, Default)]
pub struct SkillLoader;

impl SkillLoader {
    pub fn new() -> Self {
        Self
    }

    /// 加载单个 Skill 目录（内含 `SKILL.md`）。
    pub fn load_dir(&self, dir: impl AsRef<Path>) -> Result<Skill, SkillError> {
        let dir = dir.as_ref();
        let path = dir.join("SKILL.md");
        if !path.is_file() {
            return Err(SkillError::InvalidPath(format!(
                "未找到 SKILL.md: {}",
                path.display()
            )));
        }
        let raw = std::fs::read_to_string(&path)?;
        let parsed = parse_skill_md(&raw)?;
        Ok(Skill {
            meta: parsed.meta,
            body: parsed.body,
            root: Some(dir.to_path_buf()),
        })
    }

    /// 从字符串解析（测试与内存装配）。
    pub fn parse_str(&self, raw: &str) -> Result<Skill, SkillError> {
        let parsed = parse_skill_md(raw)?;
        Ok(Skill::from_parts(parsed.meta, parsed.body))
    }

    /// 扫描根目录下一层子目录中的 Skill。
    pub fn discover(&self, root: impl AsRef<Path>) -> Result<Vec<Skill>, SkillError> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(SkillError::InvalidPath(format!(
                "不是目录: {}",
                root.display()
            )));
        }
        let mut skills = Vec::new();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let skill_md = dir.join("SKILL.md");
            if skill_md.is_file() {
                skills.push(self.load_dir(&dir)?);
            }
        }
        Ok(skills)
    }
}
