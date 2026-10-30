use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "store.persist";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("store").expect("dep"), alias: None, optional: false }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("get", "读取", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("put", "写入", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("merge", "合并", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("delete", "删除", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("exists", "是否存在", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("list", "列举", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("purge", "按前缀删除", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("stat", "元信息", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("blob_get", "分块读二进制", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("blob_put", "写二进制", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("blob_write", "流式写二进制", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("acquire", "加锁", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("release", "解锁", serde_json::json!({"type":"object"}))],
        events: vec![],
    }
}
