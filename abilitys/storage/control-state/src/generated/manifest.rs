use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "control_state";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("load", "读取拥有者的控制状态条目", serde_json::json!({"properties":{"key":{"description":"条目键，调度状态固定为 state","type":"string"},"owner":{"description":"拥有该状态的 Ability 逻辑名（如 planner、egress）","type":"string"}},"required":["owner","key"],"type":"object"})).with_output(serde_json::json!({"properties":{"value":{}},"type":"object"})), ability_support::MethodMeta::new("store", "写入拥有者的控制状态条目", serde_json::json!({"properties":{"key":{"type":"string"},"owner":{"type":"string"},"value":{}},"required":["owner","key","value"],"type":"object"})).with_output(serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
