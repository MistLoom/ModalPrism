use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "planner";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("config").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("strata_queue").expect("dep"), alias: None, optional: false }, ability_support::Dependency { id: ability_support::AbilityId::new("control_state").expect("dep"), alias: None, optional: false }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("tick", "派出到期任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("trigger", "把信源标成到期并派出", serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"})), ability_support::MethodMeta::new("next", "读取信源的下一次时间", serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"})), ability_support::MethodMeta::new("source_status", "读取信源健康状态", serde_json::json!({"properties":{"source":{"type":"string"}},"required":["source"],"type":"object"})), ability_support::MethodMeta::new("health_snapshot", "生成健康快照变更操作", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("health_commit", "提交健康快照游标", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("shutdown", "释放租约并持久化调度状态", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
