//! 生成 CLI 独立二进制入口。

use ability_support::{AbilityPackageMeta, CliInfo};

pub fn generate_bin(_meta: &AbilityPackageMeta, cli: &CliInfo) -> Result<String, String> {
    let id = &_meta.decl.id;
    let entry = &cli.entry;

    let mut arg_defs = String::new();
    let mut build_args = String::new();

    for arg in &cli.args {
        if arg.positional {
            arg_defs.push_str(&format!(
                r#"
    /// {name}
    {name}: Option<String>,
"#,
                name = arg.name,
            ));
            build_args.push_str(&format!(
                r#"
    if let Some(v) = &args.{name} {{
        obj["{name}"] = serde_json::json!(v);
    }}
"#,
                name = arg.name,
            ));
        } else if let Some(flag) = &arg.flag {
            let flag_clean = flag.trim_start_matches('-');
            arg_defs.push_str(&format!(
                r#"
    #[arg(long = "{flag_clean}")]
    {name}: Option<String>,
"#,
                flag_clean = flag_clean,
                name = arg.name,
            ));
            build_args.push_str(&format!(
                r#"
    if let Some(v) = args.{name} {{
        obj["{name}"] = serde_json::json!(v);
    }}
"#,
                name = arg.name,
            ));
        }
    }

    Ok(format!(
        r#"//! 独立 CLI：`{binary}`（由 cli/info.yml 生成）

use clap::Parser;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "{binary}")]
struct Args {{
    #[arg(long, short = 'c')]
    config: Option<String>,
{arg_defs}
    input: Option<String>,
}}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {{
    let args = Args::parse();
    let mut obj = serde_json::Map::new();
{build_args}
    if let Some(input) = &args.input {{
        if input.starts_with('{{') {{
            let v: serde_json::Value = serde_json::from_str(input)?;
            if let serde_json::Value::Object(m) = v {{
                for (k, v) in m {{
                    obj.insert(k, v);
                }}
            }}
        }} else if !obj.contains_key("path") && !obj.contains_key("text") {{
            obj.insert("path".into(), json!(input));
        }}
    }}
    eprintln!("CLI {id}.{entry} args={{:?}}", obj);
    let _ = args.config;
    Ok(())
}}
"#,
        binary = cli.binary,
        arg_defs = arg_defs,
        build_args = build_args,
    ))
}
