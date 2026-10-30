use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "read_file";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "Read a file from the local filesystem.", serde_json::json!({"properties":{"limit":{"type":"integer"},"offset":{"type":"integer"},"path":{"type":"string"}},"required":["path"],"type":"object"}))],
        events: vec![],
    }
}
