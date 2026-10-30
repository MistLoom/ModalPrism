use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "prompt";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("skill").expect("dep"), alias: None, optional: true }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("build", "装配提示词消息", serde_json::json!({"properties":{"assistant":{"type":"string"},"skill_path":{"type":"string"},"skill_paths":{"items":{"type":"string"},"type":"array"},"skill_root":{"type":"string"},"system":{"type":"string"},"user":{"type":"string"}},"type":"object"}))],
        events: vec![],
    }
}
