use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "config";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("load", "按路径加载配置（json/yaml/ini/conf/env）", serde_json::json!({"properties":{"format":{"enum":["auto","json","yaml","ini","conf","env"],"type":"string"},"path":{"type":"string"}},"required":["path"],"type":"object"})), ability_support::MethodMeta::new("env", "读取进程环境变量；可指定 prefix 过滤", serde_json::json!({"properties":{"content":{"type":"string"},"prefix":{"type":"string"}},"type":"object"}))],
        events: vec![],
    }
}
