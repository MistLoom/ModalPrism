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
            "check",
            "检查 URL 是否被策略阻断",
            serde_json::json!({"properties":{"url":{"type":"string"}},"required":["url"],"type":"object"}),
            handler_impl!(check),
        )
        .on(
            "status",
            "当前代理地址",
            serde_json::json!({"type":"object"}),
            handler_impl!(status),
        )
        .on(
            "governance_domains",
            "出站熔断域列表",
            serde_json::json!({"type":"object"}),
            handler_impl!(governance_domains),
        )
        .on(
            "reset_circuit",
            "重置指定主机的熔断器",
            serde_json::json!({"properties":{"confirm":{"type":"boolean"},"host":{"type":"string"}},"required":["host"],"type":"object"}),
            handler_impl!(reset_circuit),
        )
        .on(
            "http_request",
            "经出站网关执行 HTTP 请求（进程内访问器专用）",
            serde_json::json!({"properties":{"body":{},"headers":{"items":{"items":{"type":"string"},"maxItems":2,"minItems":2,"type":"array"},"type":"array"},"method":{"type":"string"},"modulePolicy":{"type":"object"},"url":{"type":"string"}},"required":["method","url","headers","body"],"type":"object"}),
            handler_impl!(http_request),
        )
        .into_provider()
}
