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
            "enqueue",
            "写入任务",
            serde_json::json!({"properties":{"tasks":{"type":"array"}},"required":["tasks"],"type":"object"}),
            handler_impl!(enqueue),
        )
        .on(
            "lease",
            "租约一批任务",
            serde_json::json!({"type":"object"}),
            handler_impl!(lease),
        )
        .on(
            "extend",
            "延长租约",
            serde_json::json!({"type":"object"}),
            handler_impl!(extend),
        )
        .on(
            "nack",
            "退回任务",
            serde_json::json!({"type":"object"}),
            handler_impl!(nack),
        )
        .on(
            "lease_id",
            "按任务 id 租约",
            serde_json::json!({"type":"object"}),
            handler_impl!(lease_id),
        )
        .on(
            "release_leases",
            "释放过期租约",
            serde_json::json!({"type":"object"}),
            handler_impl!(release_leases),
        )
        .on(
            "active",
            "队列是否已有活跃任务",
            serde_json::json!({"type":"object"}),
            handler_impl!(active),
        )
        .on(
            "lease_ready",
            "租约就绪任务",
            serde_json::json!({"type":"object"}),
            handler_impl!(lease_ready),
        )
        .on(
            "lease_follows",
            "租约 follow 任务",
            serde_json::json!({"type":"object"}),
            handler_impl!(lease_follows),
        )
        .on(
            "leased_count",
            "已租约数量",
            serde_json::json!({"type":"object"}),
            handler_impl!(leased_count),
        )
        .on(
            "apply",
            "应用变更集（含 ack）",
            serde_json::json!({"type":"object"}),
            handler_impl!(apply),
        )
        .on(
            "accepted_cursor",
            "已接受游标",
            serde_json::json!({"type":"object"}),
            handler_impl!(accepted_cursor),
        )
        .on(
            "visible_cursor",
            "可见游标",
            serde_json::json!({"type":"object"}),
            handler_impl!(visible_cursor),
        )
        .into_provider()
}
