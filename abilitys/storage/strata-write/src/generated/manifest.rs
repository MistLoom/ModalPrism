use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "strata_write";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("apply", "提交一个变更集", serde_json::json!({"properties":{"changeset":{"type":"object"}},"required":["changeset"],"type":"object"}))],
        events: vec![],
    }
}
