use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "todo_write";
pub const DECL_FRAMEWORK_VERSION: &str = "1";

pub fn manifest() -> AbilityManifest {
    AbilityManifest {
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: serde_json::json!(null),
        methods: vec![ability_support::MethodMeta::new("run", "Update the structured todo list for the current session.", serde_json::json!({"properties":{"merge":{"type":"boolean"},"todos":{"items":{"properties":{"content":{"type":"string"},"id":{"type":"string"},"status":{"enum":["pending","in_progress","completed","cancelled"],"type":"string"}},"required":["id","content","status"],"type":"object"},"type":"array"}},"required":["todos"],"type":"object"}))],
        events: vec![],
    }
}
