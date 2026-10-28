//! Ability `skill`：SKILL.md。调用方只用 `SkillExt`。

mod catalog;
mod error;
mod frontmatter;
mod generated;
mod loader;
mod logic;
mod skill;

use ability_support::client;

pub use generated::provider::provider;

/// 调用方 `use ai_skill::SkillExt` 后 `ctx.skill().load(args)`。
#[client(id = "skill", ext = skill)]
pub trait Skill {
    async fn load(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn list(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
