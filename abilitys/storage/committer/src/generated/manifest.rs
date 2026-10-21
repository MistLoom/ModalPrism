use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "committer";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("flush", "发布已接受的变更", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("maintain", "恢复存储并返回目录", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("recover", "恢复存储", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("status", "返回控制库、Lance 和暂存区路径", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
