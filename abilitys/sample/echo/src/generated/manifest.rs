use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "algo.echo";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "回显输入文本", serde_json::json!({"properties":{"text":{"type":"string"}},"required":["text"],"type":"object"}))],
        events: vec![],
    }
}
