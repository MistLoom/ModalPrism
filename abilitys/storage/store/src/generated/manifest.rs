use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "store";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"data_root":{"type":"string"},"temp_root":{"type":"string"},"watch":{"type":"boolean"}},"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("get", "读取结构数据", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("put", "写入结构数据", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("merge", "深合并结构数据", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("delete", "删除", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("exists", "是否存在", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("list", "列举 key", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("purge", "按前缀删除", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("stat", "元信息", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("blob_get", "分块读二进制", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("blob_put", "写二进制", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("blob_write", "流式写二进制", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("acquire", "加锁", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("release", "解锁", serde_json::json!({"type":"object"}))],
        events: vec![ability_support::EventDecl { name: "changed".into(), dir: ability_support::EventDir::Emit, description: "文件系统变更通知".into(), payload_schema: serde_json::json!({"properties":{"op":{"type":"string"},"path":{"type":"string"}},"required":["path","op"],"type":"object"}) }],
    }
}
