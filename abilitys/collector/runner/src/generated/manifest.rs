use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "runner";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("planner").expect("dep"), alias: None, optional: false }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run_task", "执行一个任务", serde_json::json!({"properties":{"task":{"type":"object"}},"required":["task"],"type":"object"}))],
        events: vec![],
    }
}
