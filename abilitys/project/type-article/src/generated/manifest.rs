use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "type_article";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("project", "投影一篇文章", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("on_commit", "文章不产出变更集", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
