use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "egress";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"proxyListen":{"type":"string"},"proxyUrl":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("check", "检查 URL 是否被策略阻断", serde_json::json!({"properties":{"url":{"type":"string"}},"required":["url"],"type":"object"})), ability_support::MethodMeta::new("status", "当前代理地址", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("governance_domains", "出站熔断域列表", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("reset_circuit", "重置指定主机的熔断器", serde_json::json!({"properties":{"confirm":{"type":"boolean"},"host":{"type":"string"}},"required":["host"],"type":"object"}))],
        events: vec![],
    }
}
