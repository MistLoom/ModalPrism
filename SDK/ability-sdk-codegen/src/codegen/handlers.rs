//! 生成 handlers_trait.rs。

use ability_support::AbilityPackageMeta;
use serde_json::Value;

pub fn generate(meta: &AbilityPackageMeta) -> Result<String, String> {
    let mut methods = String::new();
    let mut args_structs = String::new();

    for m in &meta.decl.methods {
        let fn_name = method_fn_name(&m.id);
        let struct_name = method_args_name(&m.id);
        let fields = args_struct_fields(&m.input)?;

        args_structs.push_str(&format!(
            r#"
#[derive(Debug, serde::Deserialize)]
pub struct {struct_name} {{
{fields}
}}
"#,
            struct_name = struct_name,
            fields = fields,
        ));

        methods.push_str(&format!(
            r#"
    async fn {fn_name}(&self, ctx: &dyn ability_support::AbilityCtx, args: serde_json::Value) -> ability_support::Result<serde_json::Value>;
"#,
            fn_name = fn_name,
        ));
    }

    Ok(format!(
        r#"// 自动生成，勿手改
use async_trait::async_trait;

{args_structs}

#[async_trait]
pub trait Handlers: Send + Sync {{
{methods}
}}
"#,
        args_structs = args_structs,
        methods = methods,
    ))
}

pub fn method_fn_name(id: &str) -> String {
    id.replace('.', "_").replace('-', "_")
}

pub fn method_args_name(id: &str) -> String {
    let base = method_fn_name(id);
    let mut chars = base.chars();
    match chars.next() {
        None => "Args".into(),
        Some(c) => c.to_uppercase().chain(chars).collect::<String>() + "Args",
    }
}

fn args_struct_fields(input: &Value) -> Result<String, String> {
    let required: Vec<&str> = input
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut lines = Vec::new();
    if let Some(props) = input.get("properties").and_then(|v| v.as_object()) {
        for (key, schema) in props {
            let rust_ty = schema_to_rust_type(schema)?;
            let is_req = required.iter().any(|r| r == key);
            if is_req {
                lines.push(format!("    pub {key}: {rust_ty},"));
            } else {
                lines.push("#[serde(default)]".to_string());
                lines.push(format!("    pub {key}: {rust_ty},"));
            }
        }
    }
    Ok(lines.join("\n"))
}

fn schema_to_rust_type(schema: &Value) -> Result<String, String> {
    let ty = schema.get("type").and_then(|v| v.as_str()).unwrap_or("string");
    match ty {
        "string" => Ok("String".into()),
        "integer" => Ok("i64".into()),
        "number" => Ok("f64".into()),
        "boolean" => Ok("bool".into()),
        "array" => {
            let item = schema
                .get("items")
                .map(schema_to_rust_type)
                .transpose()?
                .unwrap_or_else(|| "String".to_string());
            Ok(format!("Vec<{item}>"))
        }
        "object" => Ok("serde_json::Value".into()),
        other => Err(format!("暂不支持的 input 字段类型: {other}")),
    }
}
