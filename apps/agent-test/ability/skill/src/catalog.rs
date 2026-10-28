use std::collections::HashMap;
use std::path::Path;

use crate::error::SkillError;
use crate::loader::SkillLoader;
use crate::skill::Skill;

/// Skill 目录：按 name 索引，供编排层按需选取。
#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    skills: HashMap<String, Skill>,
}

impl SkillCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, skill: Skill) {
        self.skills.insert(skill.meta.name.clone(), skill);
    }

    pub fn extend(&mut self, skills: impl IntoIterator<Item = Skill>) {
        for skill in skills {
            self.insert(skill);
        }
    }

    pub fn load_root(&mut self, root: impl AsRef<Path>) -> Result<usize, SkillError> {
        let loaded = SkillLoader::new().discover(root)?;
        let n = loaded.len();
        self.extend(loaded);
        Ok(n)
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn require(&self, name: &str) -> Result<&Skill, SkillError> {
        self.get(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))
    }

    pub fn list(&self) -> Vec<&Skill> {
        let mut list: Vec<_> = self.skills.values().collect();
        list.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
        list
    }

    /// 可被模型自动调用的 Skill（`disable_model_invocation == false`）。
    pub fn invocable(&self) -> Vec<&Skill> {
        self.list()
            .into_iter()
            .filter(|s| !s.meta.disable_model_invocation)
            .collect()
    }

    /// 简单关键词匹配：description / name 包含任一 needle（大小写不敏感）。
    pub fn match_keywords(&self, needles: &[&str]) -> Vec<&Skill> {
        if needles.is_empty() {
            return self.invocable();
        }
        self.invocable()
            .into_iter()
            .filter(|s| {
                let hay = format!("{} {}", s.meta.name, s.meta.description).to_lowercase();
                needles.iter().any(|n| hay.contains(&n.to_lowercase()))
            })
            .collect()
    }

    pub fn render_catalog(&self) -> String {
        self.list()
            .iter()
            .map(|s| s.render_catalog_entry())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}
