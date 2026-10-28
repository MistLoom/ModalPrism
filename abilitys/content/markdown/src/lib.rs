//! Ability `markdown`：frontmatter / 正文 / 标题。落盘经 persist。

mod generated;
mod logic;

use ability_support::client;

pub use generated::provider::provider;

#[client(id = "markdown", ext = markdown)]
pub trait Markdown {
    async fn read(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn write(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn patch_frontmatter(
        &self,
        args: serde_json::Value,
    ) -> ability_support::Result<serde_json::Value>;
    async fn append(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
    async fn headings(&self, args: serde_json::Value)
    -> ability_support::Result<serde_json::Value>;
    async fn parse(&self, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
}
