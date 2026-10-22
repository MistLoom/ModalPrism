//! 集成测试用最小 Ability 装配。

use std::sync::Arc;

use ability_support::{
    AbilityManifest, AbilityProvider, EventDecl, FnAbility, MethodMeta, StaticProvider,
};
use serde_json::json;

pub fn manifest(id: &str, methods: &[&str]) -> AbilityManifest {
    manifest_with_events(id, methods, &[])
}

pub fn manifest_with_events(
    id: &str,
    methods: &[&str],
    events: &[EventDecl],
) -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(id).expect("ability id"),
        depends: vec![],
        stateful: false,
        config_schema: json!({}),
        methods: methods
            .iter()
            .map(|m| MethodMeta::new(*m, "", json!({ "type": "object" })))
            .collect(),
        events: events.to_vec(),
    }
}

/// 回显 `run`：返回入参 JSON。
pub fn echo_provider(id: &str) -> Arc<dyn AbilityProvider> {
    let m = manifest(id, &["run"]);
    Arc::new(StaticProvider::new(Arc::new(FnAbility::new(m, |a| async move {
        Ok(a)
    }))))
}
