use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "demo.counter";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "会话内计数器", serde_json::json!({"properties":{"delta":{"default":1,"type":"integer"}},"type":"object"}))],
        events: vec![],
    }
}
