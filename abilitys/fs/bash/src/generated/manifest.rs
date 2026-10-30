use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "bash";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "Run a shell command.", serde_json::json!({"properties":{"block_until_ms":{"type":"integer"},"command":{"type":"string"},"working_directory":{"type":"string"}},"required":["command"],"type":"object"}))],
        events: vec![],
    }
}
