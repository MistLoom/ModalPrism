//! 只读进程：与采集进程共用 Ability 包，经 MCP 暴露只读表面。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{InstanceId, MethodId};
use async_trait::async_trait;
use modalprism_assembly::{
    McpReadGateway, READ_AGENT_MCP_INSTANCES, install_read_agent, read_agent_expose,
};
use collector_runtime::refuse_remote;
use config::LoadOptions;
use frame_core::{CatalogEntry, Gateway, InvokeRequest, InvokeResponse, Port};
use frame_port_mcp::McpPort;
use serde_json::json;

pub struct Command {
    pub data: PathBuf,
    pub config: PathBuf,
}

pub struct Running {
    host: Arc<AbilityHost>,
    gateway: Arc<dyn Gateway>,
}

impl Running {
    pub fn host(&self) -> &AbilityHost {
        &self.host
    }

    pub fn gateway(&self) -> Arc<dyn Gateway> {
        Arc::clone(&self.gateway)
    }
}

pub fn parse_args<I, S>(args: I) -> Result<Command, types::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut data = PathBuf::from("var");
    let mut config = PathBuf::from("config");
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let flag = flag.as_ref();
        let Some(value) = args.next() else {
            return Err(types::Error::new(format!("缺少参数值: {flag}")));
        };
        match flag {
            "--data" => data = PathBuf::from(value.as_ref()),
            "--config" => config = PathBuf::from(value.as_ref()),
            _ => return Err(types::Error::new(format!("未知参数: {flag}"))),
        }
    }
    Ok(Command { data, config })
}

pub async fn boot(data: &Path, config_dir: &Path) -> Result<Running, types::Error> {
    let load_options = LoadOptions::from_process();
    let snapshot = config::validate(config_dir, &load_options)?;
    refuse_remote(&snapshot, data).map_err(|e| types::Error::new(e.to_string()))?;

    let data_dir = collector_runtime::normalize(data);
    let bind = json!({
        "dataDir": data_dir,
        "configDir": config_dir,
        "system": snapshot.system,
    });
    let strata_config = json!({
        "dataDir": data_dir,
        "system": snapshot.system,
    });

    let host = install_read_agent(bind, strata_config)
        .await
        .map_err(|e| types::Error::new(e.to_string()))?;
    let gateway = McpReadGateway::new(
        Arc::clone(&host),
        read_agent_expose(),
        READ_AGENT_MCP_INSTANCES,
    );
    Ok(Running { host, gateway })
}

pub async fn serve(gateway: Arc<dyn Gateway>) -> Result<(), types::Error> {
    Box::new(McpPort::new("modalprism-agent", "0.1.0"))
        .serve(mcp_gateway(gateway))
        .await
        .map_err(|error| types::Error::new(error.to_string()))
}

pub fn mcp_gateway(inner: Arc<dyn Gateway>) -> Arc<dyn Gateway> {
    Arc::new(MethodTools { inner })
}

struct MethodTools {
    inner: Arc<dyn Gateway>,
}

#[async_trait]
impl Gateway for MethodTools {
    async fn invoke(&self, mut request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        let (instance, method) = split_tool(request.instance.as_str())?;
        request.instance = instance;
        request.method = method;
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter_map(|mut entry| {
                mark_read_only(&mut entry);
                let name = format!("{}.{}", entry.instance.as_str(), entry.method.as_str());
                entry.instance = InstanceId::new(&name).ok()?;
                Some(entry)
            })
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        let (real_instance, real_method) = split_tool(instance.as_str()).ok()?;
        if real_method.as_str() != method.as_str() {
            return None;
        }
        let mut entry = self.inner.get_entry(&real_instance, &real_method)?;
        mark_read_only(&mut entry);
        entry.instance = instance.clone();
        Some(entry)
    }
}

fn mark_read_only(entry: &mut CatalogEntry) {
    if method_is_read_only(entry.method.as_str()) {
        entry.stateful = false;
    }
}

fn method_is_read_only(method: &str) -> bool {
    matches!(
        method,
        "kinds"
            | "has"
            | "get"
            | "slice"
            | "resolve"
            | "history"
            | "lineage"
            | "related"
            | "query"
            | "changes"
            | "status"
            | "validate"
            | "diff"
            | "explain"
            | "project"
            | "on_commit"
    )
}

fn split_tool(name: &str) -> ability_support::Result<(InstanceId, MethodId)> {
    let Some((instance, method)) = name.rsplit_once('.') else {
        return Err(ability_support::Error::not_found(name));
    };
    if instance.is_empty() || method.is_empty() {
        return Err(ability_support::Error::not_found(name));
    }
    let instance =
        InstanceId::new(instance).map_err(|_| ability_support::Error::not_found(name))?;
    let method = MethodId::new(method).map_err(|_| ability_support::Error::not_found(name))?;
    Ok((instance, method))
}
