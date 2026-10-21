use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "strata_read";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: true,
        config_schema: serde_json::json!({"properties":{"dataDir":{"type":"string"},"system":{"type":"object"}},"required":["dataDir"],"type":"object"}),
        methods: vec![ability_support::MethodMeta::new("kinds", "列出资源上的 facet 种类", serde_json::json!({"properties":{"includeVariants":{"type":"boolean"},"rid":{"type":"string"}},"required":["rid"],"type":"object"})), ability_support::MethodMeta::new("has", "判断资源是否有某种 facet", serde_json::json!({"properties":{"kind":{"type":"string"},"rid":{"type":"string"},"variant":{"type":"string"}},"required":["rid","kind"],"type":"object"})), ability_support::MethodMeta::new("get", "读取 facet 的版本与元数据", serde_json::json!({"properties":{"facet":{"type":"string"},"lazy":{"type":"boolean"}},"required":["facet"],"type":"object"})), ability_support::MethodMeta::new("slice", "读取文本区间", serde_json::json!({"properties":{"span":{"type":"string"}},"required":["span"],"type":"object"})), ability_support::MethodMeta::new("resolve", "按身份键解析资源", serde_json::json!({"properties":{"identity":{"type":"string"}},"required":["identity"],"type":"object"})), ability_support::MethodMeta::new("history", "列出 facet 版本", serde_json::json!({"properties":{"facet":{"type":"string"}},"required":["facet"],"type":"object"})), ability_support::MethodMeta::new("lineage", "读取 facet 血缘", serde_json::json!({"properties":{"depth":{"type":"integer"},"direction":{"type":"string"},"facet":{"type":"string"}},"required":["facet"],"type":"object"})), ability_support::MethodMeta::new("related", "读取资源关系", serde_json::json!({"properties":{"direction":{"type":"string"},"rel":{"type":"string"},"rid":{"type":"string"}},"required":["rid"],"type":"object"})), ability_support::MethodMeta::new("query", "按查询对象读取", serde_json::json!({"properties":{"query":{"type":"object"}},"required":["query"],"type":"object"})), ability_support::MethodMeta::new("changes", "读取已可见变更", serde_json::json!({"properties":{"after":{"type":"integer"},"limit":{"type":"integer"}},"required":["after"],"type":"object"}))],
        events: vec![],
    }
}
