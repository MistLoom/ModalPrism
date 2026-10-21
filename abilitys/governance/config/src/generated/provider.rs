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
        .stateful()
        .on(
            "load",
            "从目录重载配置",
            serde_json::json!({"type":"object"}),
            handler_impl!(load),
        )
        .on(
            "generation",
            "当前代次",
            serde_json::json!({"type":"object"}),
            handler_impl!(generation),
        )
        .on(
            "layout",
            "数据与配置目录",
            serde_json::json!({"type":"object"}),
            handler_impl!(layout),
        )
        .on(
            "applied",
            "已应用快照摘要",
            serde_json::json!({"type":"object"}),
            handler_impl!(applied),
        )
        .into_provider()
}
