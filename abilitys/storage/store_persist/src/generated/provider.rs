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
        .depends_id("store")
        .on(
            "get",
            "读取",
            serde_json::json!({"type":"object"}),
            handler_impl!(get),
        )
        .on(
            "put",
            "写入",
            serde_json::json!({"type":"object"}),
            handler_impl!(put),
        )
        .on(
            "merge",
            "合并",
            serde_json::json!({"type":"object"}),
            handler_impl!(merge),
        )
        .on(
            "delete",
            "删除",
            serde_json::json!({"type":"object"}),
            handler_impl!(delete),
        )
        .on(
            "exists",
            "是否存在",
            serde_json::json!({"type":"object"}),
            handler_impl!(exists),
        )
        .on(
            "list",
            "列举",
            serde_json::json!({"type":"object"}),
            handler_impl!(list),
        )
        .on(
            "purge",
            "按前缀删除",
            serde_json::json!({"type":"object"}),
            handler_impl!(purge),
        )
        .on(
            "stat",
            "元信息",
            serde_json::json!({"type":"object"}),
            handler_impl!(stat),
        )
        .on(
            "blob_get",
            "分块读二进制",
            serde_json::json!({"type":"object"}),
            handler_impl!(blob_get),
        )
        .on(
            "blob_put",
            "写二进制",
            serde_json::json!({"type":"object"}),
            handler_impl!(blob_put),
        )
        .on(
            "blob_write",
            "流式写二进制",
            serde_json::json!({"type":"object"}),
            handler_impl!(blob_write),
        )
        .on(
            "acquire",
            "加锁",
            serde_json::json!({"type":"object"}),
            handler_impl!(acquire),
        )
        .on(
            "release",
            "解锁",
            serde_json::json!({"type":"object"}),
            handler_impl!(release),
        )
        .into_provider()
}
