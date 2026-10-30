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
        .on(
            "load",
            "加载单个 Skill 目录",
            serde_json::json!({"properties":{"path":{"type":"string"}},"required":["path"],"type":"object"}),
            handler_impl!(load),
        )
        .on(
            "list",
            "扫描根目录下的 Skill",
            serde_json::json!({"properties":{"root":{"type":"string"}},"required":["root"],"type":"object"}),
            handler_impl!(list),
        )
        .into_provider()
}
