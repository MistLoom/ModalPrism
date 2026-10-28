//! 独立 CLI：`todo_write`（由 cli/info.yml 生成）

use clap::Parser;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "todo_write")]
struct Args {
    #[arg(long, short = 'c')]
    config: Option<String>,

    #[arg(long = "merge")]
    merge: Option<String>,

    input: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut obj = serde_json::Map::new();

    if let Some(v) = args.merge {
        obj["merge"] = serde_json::json!(v);
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
    eprintln!("CLI todo_write.run args={:?}", obj);
    let _ = args.config;
    Ok(())
}
