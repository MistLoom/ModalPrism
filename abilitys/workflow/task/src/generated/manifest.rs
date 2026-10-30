use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "task";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("store.persist").expect("dep"), alias: None, optional: false }],
        stateful: true,
        config_schema: serde_json::json!({"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("project_set", "设置工程参数", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("submit", "提交任务 DAG", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("status", "查询任务或工程状态", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("next", "认领下一个就绪任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("get", "获取可认领或全部任务", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("update", "更新任务状态", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
