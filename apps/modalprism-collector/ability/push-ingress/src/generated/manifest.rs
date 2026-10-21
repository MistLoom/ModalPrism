use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "push_ingress";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("config").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("egress").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("paid_gateway").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("strata_write").expect("dep"), alias: None, optional: false }],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"configDir":{"type":"string"},"dataDir":{"type":"string"},"pushListen":{"type":"string"},"system":{"type":"object"}},"required":["dataDir","configDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("status", "监听状态", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("start_listener", "绑定 Push 监听（原 Lifecycle::Started）", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("stop_listener", "停止 Push 监听", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
