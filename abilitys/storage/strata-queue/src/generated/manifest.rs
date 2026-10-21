use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "strata_queue";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("enqueue", "写入任务", serde_json::json!({"properties":{"tasks":{"type":"array"}},"required":["tasks"],"type":"object"})), ability_support::MethodMeta::new("lease", "租约一批任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("extend", "延长租约", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("nack", "退回任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("lease_id", "按任务 id 租约", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("release_leases", "释放过期租约", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("active", "队列是否已有活跃任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("lease_ready", "租约就绪任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("lease_follows", "租约 follow 任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("leased_count", "已租约数量", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("apply", "应用变更集（含 ack）", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("accepted_cursor", "已接受游标", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("visible_cursor", "可见游标", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
