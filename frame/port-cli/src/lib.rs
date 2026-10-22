//! CLI 端口：将命令行参数映射为统一调用信封。

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use frame_core::{
    DefaultCliController, Error, Gateway, LocalsScope, Port, ProtocolController, resolve_target,
};
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(name = "runtime", about = "LEAF 运行时")]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<String>,

    /// 启用管理面 HTTP API（默认关闭）
    #[arg(long, global = true)]
    pub management: bool,

    #[arg(long, global = true, default_value = "127.0.0.1:8791")]
    pub management_bind: SocketAddr,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Catalog {
        #[command(subcommand)]
        action: CatalogCmd,
    },
    /// 能力 CLI 批量编译
    Abilities {
        #[command(subcommand)]
        action: AbilitiesCmd,
    },
    /// 兼容旧子命令名
    Tools {
        #[command(subcommand)]
        action: CatalogCmd,
    },
    Invoke {
        /// 实例，或 `实例.方法`（单方法实例可只写实例）
        target: String,
        #[arg(long, default_value = "{}")]
        args: String,
        #[arg(long)]
        session: Option<String>,
    },
    Serve {
        #[command(subcommand)]
        target: ServeCmd,
    },
}

#[derive(Debug, Subcommand)]
pub enum AbilitiesCmd {
    Cli {
        #[command(subcommand)]
        action: CliBuildCmd,
    },
}

#[derive(Debug, Subcommand)]
pub enum CliBuildCmd {
    /// 编译所有声明 cli/ 的能力
    Build {
        #[arg(long)]
        ability: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CatalogCmd {
    List,
    Show { target: String },
}

#[derive(Debug, Subcommand)]
pub enum ServeCmd {
    Http {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
    },
    Mcp,
    Grpc {
        #[arg(long, default_value = "127.0.0.1:50051")]
        bind: String,
    },
}

pub struct CliPort;

impl CliPort {
    pub async fn run_catalog_list(gateway: &dyn Gateway) -> Result<(), Error> {
        let cat = DefaultCliController.catalog(gateway);
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for e in &cat {
            *counts.entry(e.instance.as_str()).or_insert(0) += 1;
        }
        for e in &cat {
            let name = e.discovery_name(*counts.get(e.instance.as_str()).unwrap_or(&1));
            println!("{}\tstateful={}\t{}", name, e.stateful, e.description);
        }
        Ok(())
    }

    pub async fn run_catalog_show(gateway: &dyn Gateway, target: &str) -> Result<(), Error> {
        let (inst, method) = resolve_target(target, &gateway.catalog())?;
        let meta = gateway
            .get_entry(&inst, &method)
            .ok_or_else(|| Error::not_found(target))?;
        let text = serde_json::to_string_pretty(&meta).map_err(|e| Error::failed(e.to_string()))?;
        println!("{text}");
        Ok(())
    }

    pub async fn run_invoke(
        gateway: &dyn Gateway,
        target: &str,
        args: &str,
        session: Option<&str>,
    ) -> Result<(), Error> {
        let (inst, method) = resolve_target(target, &gateway.catalog())?;
        let args: Value = serde_json::from_str(args)
            .map_err(|e| Error::config(format!("参数 JSON 无效: {e}")))?;
        let locals = match session {
            Some(s) => Some(
                s.parse::<LocalsScope>()
                    .map_err(|e| Error::config(format!("session 无效: {e}")))?,
            ),
            None => None,
        };
        let response = DefaultCliController
            .invoke(gateway, inst, method, args, None, locals)
            .await?;
        let text =
            serde_json::to_string_pretty(&response).map_err(|e| Error::failed(e.to_string()))?;
        println!("{text}");
        Ok(())
    }
}

#[async_trait]
impl Port for CliPort {
    fn name(&self) -> &'static str {
        "cli"
    }

    async fn serve(self: Box<Self>, _gateway: Arc<dyn Gateway>) -> Result<(), Error> {
        Err(Error::config("CLI 端口通过组合根同步调度，不支持 serve"))
    }
}
