#![allow(dead_code, non_camel_case_types, unused_imports)]
//! 热榜投影 Ability。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};
use serde_json::json;

mod logic;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("type_hotlist")
        .on(
            "project",
            "投影一期热榜",
            json!({"type": "object"}),
            |ctx, args| ability_support::box_fut(async move { logic::project(ctx, args).await }),
        )
        .on(
            "on_commit",
            "返回名次差与保留份数的变更集",
            json!({"type": "object"}),
            |ctx, args| ability_support::box_fut(async move { logic::on_commit(ctx, args).await }),
        )
        .into_provider()
}
