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
            "读取拥有者的控制状态条目",
            serde_json::json!({"properties":{"key":{"description":"条目键，调度状态固定为 state","type":"string"},"owner":{"description":"拥有该状态的 Ability 逻辑名（如 planner、egress）","type":"string"}},"required":["owner","key"],"type":"object"}),
            handler_impl!(load),
        )
        .on(
            "store",
            "写入拥有者的控制状态条目",
            serde_json::json!({"properties":{"key":{"type":"string"},"owner":{"type":"string"},"value":{}},"required":["owner","key","value"],"type":"object"}),
            handler_impl!(store),
        )
        .into_provider()
}
