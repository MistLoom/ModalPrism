use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "strata_queue";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("enqueue", "写入任务", serde_json::json!({"properties":{"tasks":{"type":"array"}},"required":["tasks"],"type":"object"})), ability_support::MethodMeta::new("lease", "租约一批任务", serde_json::json!({"properties":{"count":{"type":"integer"},"leaseMs":{"type":"integer"},"queue":{"type":"string"}},"required":["queue","count","leaseMs"],"type":"object"})), ability_support::MethodMeta::new("extend", "延长租约", serde_json::json!({"properties":{"leaseMs":{"type":"integer"},"taskId":{"type":"string"}},"required":["taskId","leaseMs"],"type":"object"})), ability_support::MethodMeta::new("nack", "退回任务", serde_json::json!({"properties":{"error":{"type":"string"},"retryAt":{"type":"string"},"taskId":{"type":"string"}},"required":["taskId","retryAt","error"],"type":"object"}))],
        events: vec![],
    }
}
