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
        .depends_id("strata_queue")
        .depends_id("control_state")
        .on(
            "tick",
            "派出到期任务",
            serde_json::json!({"type":"object"}),
            handler_impl!(tick),
        )
        .on(
            "trigger",
            "把信源标成到期并派出",
            serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"}),
            handler_impl!(trigger),
        )
        .on(
            "next",
            "读取信源的下一次时间",
            serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"}),
            handler_impl!(next),
        )
        .on(
            "source_status",
            "读取信源健康状态",
            serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"}),
            handler_impl!(source_status),
        )
        .on(
            "health_snapshot",
            "生成健康快照变更操作",
            serde_json::json!({"type":"object"}),
            handler_impl!(health_snapshot),
        )
        .on(
            "health_commit",
            "提交健康快照游标",
            serde_json::json!({"type":"object"}),
            handler_impl!(health_commit),
        )
        .on(
            "shutdown",
            "释放租约并持久化调度状态",
            serde_json::json!({"type":"object"}),
            handler_impl!(shutdown),
        )
        .into_provider()
}
