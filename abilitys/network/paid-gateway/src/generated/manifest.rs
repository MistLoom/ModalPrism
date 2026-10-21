use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "paid_gateway";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("config").expect("dep"), alias: None, optional: false }],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("reserve", "占位方法，不扣费", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("complete", "确认回执", serde_json::json!({"properties":{"receiptId":{"type":"string"}},"required":["receiptId"],"type":"object"})), ability_support::MethodMeta::new("status", "读取回执状态", serde_json::json!({"properties":{"receiptId":{"type":"string"}},"required":["receiptId"],"type":"object"})), ability_support::MethodMeta::new("call", "经付费网关执行请求", serde_json::json!({"properties":{"body":{},"endpoint":{"type":"string"},"headers":{"items":{"items":{"type":"string"},"maxItems":2,"minItems":2,"type":"array"},"type":"array"},"params":{"type":"object"},"policy":{"type":"object"}},"required":["endpoint","headers","body","params"],"type":"object"}))],
        events: vec![],
    }
}
