use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "skill";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("load", "加载单个 Skill 目录", serde_json::json!({"properties":{"path":{"type":"string"}},"required":["path"],"type":"object"})), ability_support::MethodMeta::new("list", "扫描根目录下的 Skill", serde_json::json!({"properties":{"root":{"type":"string"}},"required":["root"],"type":"object"}))],
        events: vec![],
    }
}
