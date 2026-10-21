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
        .depends_id("strata_queue")
        .depends_id("strata_write")
        .depends_id("strata_read")
        .depends_id("egress")
        .depends_id("paid_gateway")
        .depends_id("control_state")
        .depends_id("accessor_http")
        .depends_id("accessor_rss")
        .depends_id("accessor_api")
        .depends_id("accessor_paid_api")
        .depends_id("accessor_search")
        .depends_id("accessor_push")
        .depends_id("accessor_file")
        .depends_id("accessor_browser")
        .depends_id("accessor_bundle_list")
        .on(
            "run_task",
            "执行一个任务",
            serde_json::json!({"properties":{"task":{"type":"object"}},"required":["task"],"type":"object"}),
            handler_impl!(run_task),
        )
        .into_provider()
}
