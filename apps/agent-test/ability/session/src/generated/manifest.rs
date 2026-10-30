use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "session";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("create", "创建会话", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("get", "读取会话", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("append_user", "追加用户消息", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("append_assistant", "追加助手消息（可选 tool_calls）", serde_json::json!({"properties":{"id":{"type":"string"},"text":{"type":"string"},"tool_calls":{"type":"array"}},"required":["id"],"type":"object"})), ability_support::MethodMeta::new("append_tool_results", "批量追加 tool 角色消息", serde_json::json!({"properties":{"id":{"type":"string"},"messages":{"type":"array"}},"required":["id","messages"],"type":"object"})), ability_support::MethodMeta::new("protocol_messages", "投影协议消息", serde_json::json!({"type":"object"})), ability_support::MethodMeta::new("list", "列举会话", serde_json::json!({"properties":{"limit":{"type":"integer"}},"type":"object"})), ability_support::MethodMeta::new("export", "导出会话快照", serde_json::json!({"properties":{"id":{"type":"string"}},"required":["id"],"type":"object"})), ability_support::MethodMeta::new("import", "导入会话快照", serde_json::json!({"properties":{"mode":{"enum":["copy","restore"],"type":"string"},"snapshot":{"type":"object"}},"required":["snapshot"],"type":"object"})), ability_support::MethodMeta::new("compact", "压缩会话上下文", serde_json::json!({"properties":{"id":{"type":"string"},"marker_text":{"type":"string"},"until_seq":{"type":"integer"}},"required":["id","until_seq"],"type":"object"})), ability_support::MethodMeta::new("fork", "分叉会话", serde_json::json!({"properties":{"id":{"type":"string"},"until_seq":{"type":"integer"}},"required":["id"],"type":"object"}))],
        events: vec![ability_support::EventDecl { name: "created".into(), dir: ability_support::EventDir::Emit, description: "会话创建完成".into(), payload_schema: serde_json::json!({"properties":{"id":{"type":"string"}},"required":["id"],"type":"object"}) }, ability_support::EventDecl { name: "items_appended".into(), dir: ability_support::EventDir::Emit, description: "会话追加消息".into(), payload_schema: serde_json::json!({"properties":{"id":{"type":"string"},"role":{"enum":["user","assistant","tool"],"type":"string"}},"required":["id","role"],"type":"object"}) }],
    }
}
