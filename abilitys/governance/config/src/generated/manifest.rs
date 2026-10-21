use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "config";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"configDir":{"type":"string"},"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["configDir","dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("load", "从目录重载配置", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("generation", "当前代次", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("applied", "已应用快照摘要", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
