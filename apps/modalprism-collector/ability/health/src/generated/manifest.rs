use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "health";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("config").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("planner").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("strata_write").expect("dep"), alias: None, optional: true }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("evaluate", "把健康写入存储", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("status", "读取信源健康", serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"}))],
        events: vec![],
    }
}
