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
        .depends_id("config")
        .on(
            "flush",
            "发布已接受的变更",
            serde_json::json!({"type":"object"}),
            handler_impl!(flush),
        )
        .on(
            "maintain",
            "恢复存储并返回目录",
            serde_json::json!({"type":"object"}),
            handler_impl!(maintain),
        )
        .on(
            "recover",
            "恢复存储",
            serde_json::json!({"type":"object"}),
            handler_impl!(recover),
        )
        .on(
            "status",
            "返回控制库、Lance 和暂存区路径",
            serde_json::json!({"type":"object"}),
            handler_impl!(status),
        )
        .on(
            "hotlist_apply",
            "提交热榜差量变更集",
            serde_json::json!({"type":"object"}),
            handler_impl!(hotlist_apply),
        )
        .into_provider()
}
