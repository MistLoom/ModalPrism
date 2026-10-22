//! 生成 manifest.rs。

use ability_support::AbilityPackageMeta;

pub fn generate(meta: &AbilityPackageMeta) -> Result<String, String> {
    let decl = &meta.decl;
    let id = &decl.id;
    let stateful = decl.stateful;
    let config_schema = serde_json::to_string(&decl.config_schema).map_err(|e| e.to_string())?;

    let depends: Vec<String> = decl
        .depends
        .iter()
        .cloned()
        .map(|d| {
            let dep = d.into_dependency()?;
            let alias = match &dep.alias {
                Some(a) => format!(r#"alias: Some("{a}".into())"#),
                None => "alias: None".into(),
            };
            let opt = if dep.optional {
                "optional: true"
            } else {
                "optional: false"
            };
            Ok(format!(
                r#"ability_support::Dependency {{ id: ability_support::AbilityId::new("{id}").expect("dep"), {alias}, {opt} }}"#,
                id = dep.id,
                alias = alias,
                opt = opt,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let methods: Vec<String> = decl
        .methods
        .iter()
        .map(|m| {
            let input = serde_json::to_string(&m.input).map_err(|e| e.to_string())?;
            let output: Option<String> = m.output.as_ref().map(|o| {
                let s = serde_json::to_string(o).map_err(|e| e.to_string())?;
                Ok::<String, String>(format!(".with_output(serde_json::json!({s}))"))
            }).transpose()?;
            let output = output.unwrap_or_default();
            let desc = escape_str(&m.description);
            Ok(format!(
                r#"ability_support::MethodMeta::new("{mid}", "{desc}", serde_json::json!({input})){output}"#,
                mid = m.id,
                desc = desc,
                input = input,
                output = output,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let events: Vec<String> = decl
        .events
        .iter()
        .map(|e| {
            let payload =
                serde_json::to_string(&e.payload_schema).map_err(|e| e.to_string())?;
            let dir = match e.dir {
                ability_support::EventDir::Emit => "Emit",
                ability_support::EventDir::Listen => "Listen",
            };
            Ok(format!(
                r#"ability_support::EventDecl {{ name: "{name}".into(), dir: ability_support::EventDir::{dir}, description: "{desc}".into(), payload_schema: serde_json::json!({payload}) }}"#,
                name = e.name,
                dir = dir,
                desc = escape_str(&e.description),
                payload = payload,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(format!(
        r#"use ability_support::AbilityManifest;

pub const ABILITY_ID: &str = "{id}";
pub const DECL_FRAMEWORK_VERSION: &str = "{fw}";

pub fn manifest() -> AbilityManifest {{
    AbilityManifest {{
        id: ability_support::AbilityId::new(ABILITY_ID).expect("id"),
        depends: vec![{depends}],
        stateful: {stateful},
        config_schema: serde_json::json!({config_schema}),
        methods: vec![{methods}],
        events: vec![{events}],
    }}
}}
"#,
        id = id,
        fw = ability_support::FRAMEWORK_VERSION,
        depends = depends.join(", "),
        stateful = stateful,
        config_schema = config_schema,
        methods = methods.join(", "),
        events = events.join(", "),
    ))
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
