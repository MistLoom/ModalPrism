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
        .on(
            "run",
            "Update the structured todo list for the current session.",
            serde_json::json!({"properties":{"merge":{"type":"boolean"},"todos":{"items":{"properties":{"content":{"type":"string"},"id":{"type":"string"},"status":{"enum":["pending","in_progress","completed","cancelled"],"type":"string"}},"required":["id","content","status"],"type":"object"},"type":"array"}},"required":["todos"],"type":"object"}),
            handler_impl!(run),
        )
        .into_provider()
}
