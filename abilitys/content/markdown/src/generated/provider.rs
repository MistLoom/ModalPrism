// 自动生成，勿手改
use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

use super::bridge::instance;
use super::manifest::{ABILITY_ID, DECL_FRAMEWORK_VERSION};

macro_rules! handler_impl {
    ($method:ident) => {
        move |ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value| {
            let handlers = instance();
            ability_support::box_fut(async move {
                handlers.$method(ctx, args).await
            })
        }
    };
}

pub fn provider() -> Arc<dyn AbilityProvider> {
    assert_eq!(
        DECL_FRAMEWORK_VERSION,
        ability_support::FRAMEWORK_VERSION,
        "ability.yml framework_version 与当前框架不匹配"
    );
    AbilitySpec::try_new(ABILITY_ID).expect("id")
        .depends_id("store.persist")
        .on(
            "read",
            "读取 Markdown（frontmatter + body + headings）",
            serde_json::json!({"required":["key"],"type":"object"}),
            handler_impl!(read),
        )
        .on(
            "write",
            "写入 Markdown",
            serde_json::json!({"required":["key"],"type":"object"}),
            handler_impl!(write),
        )
        .on(
            "patch_frontmatter",
            "合并 frontmatter",
            serde_json::json!({"required":["key","patch"],"type":"object"}),
            handler_impl!(patch_frontmatter),
        )
        .on(
            "append",
            "追加正文",
            serde_json::json!({"required":["key","text"],"type":"object"}),
            handler_impl!(append),
        )
        .on(
            "headings",
            "列出标题",
            serde_json::json!({"required":["key"],"type":"object"}),
            handler_impl!(headings),
        )
        .on(
            "parse",
            "解析 Markdown 文本（不落盘）",
            serde_json::json!({"required":["text"],"type":"object"}),
            handler_impl!(parse),
        )
        .into_provider()
}
