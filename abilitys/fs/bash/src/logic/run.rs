//! 业务逻辑：run

use std::time::Duration;

use ability_support::{AbilityCtx, Error, Result, decode_text_bytes};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Deserialize)]
struct Args {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    argv: Option<Vec<String>>,
    working_directory: Option<String>,
    #[serde(default)]
    block_until_ms: Option<u64>,
}

pub async fn run(_ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let a: Args = serde_json::from_value(args).map_err(|e| Error::invalid_args(e.to_string()))?;
    let ms = a.block_until_ms.unwrap_or(30_000);
    let command_label = a
        .command
        .clone()
        .or_else(|| a.argv.as_ref().map(|v| v.join(" ")))
        .unwrap_or_default();
    let mut cmd = if let Some(argv) = &a.argv {
        if argv.is_empty() {
            return Err(Error::invalid_args("argv 不能为空"));
        }
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        c
    } else {
        let command = a
            .command
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::invalid_args("需要 command 或 argv"))?;
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            // 优先 UTF-8 代码页，减少 type/Get-Content 混用时的编码歧义
            c.args(["/C", &format!("chcp 65001>nul & {}", command)]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", &command]);
            c
        }
    };
    if let Some(dir) = &a.working_directory {
        cmd.current_dir(dir);
    }
    cmd.kill_on_drop(true);
    let fut = async {
        cmd.output()
            .await
            .map_err(|e| Error::failed(format!("bash 启动失败: {e}")))
    };
    let output = match timeout(Duration::from_millis(ms), fut).await {
        Ok(r) => r?,
        Err(_) => {
            return Err(Error::failed(format!(
                "bash 超时 ({ms} ms): {}",
                command_label.as_str()
            )));
        }
    };
    Ok(json!({
        "command": command_label,
        "exit_code": output.status.code(),
        "stdout": decode_text_bytes(&output.stdout),
        "stderr": decode_text_bytes(&output.stderr),
    }))
}
