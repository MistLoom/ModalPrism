use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "app.shout";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("algo.echo").expect("dep"), alias: None, optional: false }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "经 algo.echo 后转大写", serde_json::json!({"properties":{"text":{"type":"string"}},"required":["text"],"type":"object"}))],
        events: vec![],
    }
}
