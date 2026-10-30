use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "dispatch";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("defs", "catalog 生成 LLM tool 定义", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("run", "执行模型 tool_calls", serde_json::json!({"properties":{"calls":{"type":"array"},"continue_on_error":{"type":"boolean"}},"required":["calls"],"type":"object"}))],
        events: vec![],
    }
}
