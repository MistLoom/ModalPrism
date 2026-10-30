use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "chat";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!({"properties":{"api_key":{"type":"string"},"base_url":{"type":"string"},"model":{"type":"string"},"timeout_ms":{"type":"integer"}},"required":["base_url","model"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("completion", "非流式聊天补全（OpenAI 兼容）", serde_json::json!({"properties":{"messages":{"type":"array"},"temperature":{"type":"number"},"tool_choice":{"type":"string"},"tools":{"type":"array"}},"required":["messages"],"type":"object"}))],
        events: vec![],
    }
}
