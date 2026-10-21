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
        .depends_id("egress")
        .depends_id("paid_gateway")
        .depends_id("strata_write")
        .on(
            "status",
            "监听状态",
            serde_json::json!({"type":"object"}),
            handler_impl!(status),
        )
        .on(
            "start_listener",
            "绑定 Push 监听（原 Lifecycle::Started）",
            serde_json::json!({"type":"object"}),
            handler_impl!(start_listener),
        )
        .on(
            "stop_listener",
            "停止 Push 监听",
            serde_json::json!({"type":"object"}),
            handler_impl!(stop_listener),
        )
        .into_provider()
}
