//! `modalprism-agent` 的命令行。工具面走 MCP 的标准输入输出。

use std::env;

use modalprism_agent::{boot, parse_args, serve};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), types::Error> {
    let command = parse_args(env::args().skip(1))?;
    let running = boot(&command.data, &command.config).await?;
    serve(running.gateway()).await
}
