use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "type_hotlist";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("config").expect("dep"), alias: None, optional: false }],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("project", "投影一期热榜", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("on_commit", "返回名次差与保留份数的变更集", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
