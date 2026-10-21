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
        .depends_id("config")
        .depends_id("planner")
        .depends_optional("strata_write")
        .on(
            "evaluate",
            "把健康写入存储",
            serde_json::json!({"type":"object"}),
            handler_impl!(evaluate),
        )
        .on(
            "status",
            "读取信源健康",
            serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"}),
            handler_impl!(status),
        )
        .into_provider()
}
