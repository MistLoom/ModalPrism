use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "config_inspect";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"configDir":{"type":"string"},"dataDir":{"type":"string"}},"required":["configDir","dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("validate", "重新校验配置目录", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("diff", "比较当前快照与目录", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("explain", "解释模块字段来源", serde_json::json!({"properties":{"module":{"type":"string"}},"required":["module"],"type":"object"}))],
        events: vec![],
    }
}
