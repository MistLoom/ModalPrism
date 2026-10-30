use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "write_file";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "Write contents to a file.", serde_json::json!({"properties":{"contents":{"type":"string"},"path":{"type":"string"}},"required":["path","contents"],"type":"object"}))],
        events: vec![],
    }
}
