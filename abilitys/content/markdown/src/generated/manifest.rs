use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "markdown";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![ability_support::Dependency { id: ability_support::AbilityId::new("store.persist").expect("dep"), alias: None, optional: false }],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("read", "读取 Markdown（frontmatter + body + headings）", serde_json::json!({"required":["key"],"type":"object"})), ability_support::MethodMeta::new("write", "写入 Markdown", serde_json::json!({"required":["key"],"type":"object"})), ability_support::MethodMeta::new("patch_frontmatter", "合并 frontmatter", serde_json::json!({"required":["key","patch"],"type":"object"})), ability_support::MethodMeta::new("append", "追加正文", serde_json::json!({"required":["key","text"],"type":"object"})), ability_support::MethodMeta::new("headings", "列出标题", serde_json::json!({"required":["key"],"type":"object"})), ability_support::MethodMeta::new("parse", "解析 Markdown 文本（不落盘）", serde_json::json!({"required":["text"],"type":"object"}))],
        events: vec![],
    }
}
