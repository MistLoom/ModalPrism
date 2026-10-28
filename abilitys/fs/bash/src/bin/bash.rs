//! 独立 CLI：`bash`（由 cli/info.yml 生成）

use clap::Parser;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "bash")]
struct Args {
    #[arg(long, short = 'c')]
    config: Option<String>,

    /// command
    command: Option<String>,

    #[arg(long = "working-directory")]
    working_directory: Option<String>,

    #[arg(long = "block-until-ms")]
    block_until_ms: Option<String>,

    input: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut obj = serde_json::Map::new();

    if let Some(v) = &args.command {
        obj["command"] = serde_json::json!(v);
    }

    if let Some(v) = args.working_directory {
        obj["working_directory"] = serde_json::json!(v);
    }

    if let Some(v) = args.block_until_ms {
        obj["block_until_ms"] = serde_json::json!(v);
    }

    if let Some(input) = &args.input {
        if input.starts_with('{') {
            let v: serde_json::Value = serde_json::from_str(input)?;
            if let serde_json::Value::Object(m) = v {
                for (k, v) in m {
                    obj.insert(k, v);
                }
            }
        } else if !obj.contains_key("path") && !obj.contains_key("text") {
            obj.insert("path".into(), json!(input));
        }
    }
    eprintln!("CLI bash.run args={:?}", obj);
    let _ = args.config;
    Ok(())
}
