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
        .on(
            "reserve",
            "占位方法，不扣费",
            serde_json::json!({"type":"object"}),
            handler_impl!(reserve),
        )
        .on(
            "complete",
            "确认回执",
            serde_json::json!({"properties":{"receiptId":{"type":"string"}},"required":["receiptId"],"type":"object"}),
            handler_impl!(complete),
        )
        .on(
            "status",
            "读取回执状态",
            serde_json::json!({"properties":{"receiptId":{"type":"string"}},"required":["receiptId"],"type":"object"}),
            handler_impl!(status),
        )
        .on(
            "call",
            "经付费网关执行请求",
            serde_json::json!({"properties":{"body":{},"endpoint":{"type":"string"},"headers":{"items":{"items":{"type":"string"},"maxItems":2,"minItems":2,"type":"array"},"type":"array"},"params":{"type":"object"},"policy":{"type":"object"}},"required":["endpoint","headers","body","params"],"type":"object"}),
            handler_impl!(call),
        )
        .into_provider()
}
