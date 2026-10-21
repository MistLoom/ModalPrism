use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "runner";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("config").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("strata_queue").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("strata_write").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("strata_read").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("egress").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("paid_gateway").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("control_state").expect("dep"), alias: None, optional: false }],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"browserProgram":{"type":"string"},"configDir":{"type":"string"},"dataDir":{"type":"string"},"proxyUrl":{"type":"string"},"system":{"type":"object"}},"required":["dataDir","configDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("run_task", "执行一个任务", serde_json::json!({"properties":{"task":{"type":"object"}},"required":["task"],"type":"object"}))],
        events: vec![],
    }
}
