//! 只读进程。目录只含读取、健康状态、配置检查和两个抽象类型。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use async_trait::async_trait;
use frame_core::{AccessKind, CatalogEntry, Gateway, InvokeRequest, InvokeResponse, Port};
use frame_port_mcp::McpPort;
use types::{ReadCtx, ReadRoot};

pub struct Command {
    pub data: PathBuf,
    pub config: PathBuf,
}

pub struct Running {
    root: Arc<ReadRoot>,
    host: Arc<AbilityHost>,
    gateway: Arc<dyn Gateway>,
}

impl Running {
    pub fn root(&self) -> &ReadRoot {
        &self.root
    }

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
    let root = Arc::new(ReadRoot::open(data, config_dir).await?);
    let host = link(Arc::clone(&root)).await?;
    let gateway: Arc<dyn Gateway> = Arc::new(ReadGateway {
        inner: Arc::clone(&host),
    });
    Ok(Running {
        root,
        host,
        gateway,
    })
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
    if surface::is_read_only(
        surface::AGENT,
        entry.instance.as_str(),
        entry.method.as_str(),
    ) {
        entry.stateful = false;
    }
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

async fn link(root: Arc<ReadRoot>) -> Result<Arc<AbilityHost>, types::Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    let ctx: Arc<dyn ReadCtx> = Arc::clone(&root) as Arc<dyn ReadCtx>;
    let providers: Vec<(&str, Arc<dyn ability_support::AbilityProvider>)> = vec![
        ("strata_read", strata_read::provider(Arc::clone(&ctx))),
        ("health", health::provider(Arc::clone(&ctx))),
        ("config_inspect", config_inspect::provider(Arc::clone(&ctx))),
        ("type_article", type_article::provider(Arc::clone(&ctx))),
        ("type_hotlist", type_hotlist::provider(Arc::clone(&ctx))),
    ];
    for (id, provider) in providers {
        host.install_str(id, provider)
            .await
            .map_err(|error| types::Error::new(error.to_string()))?;
        host.set_protocol_support(
            id,
            vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
        );
        let source = SourceId::new(id).map_err(|error| types::Error::new(error.to_string()))?;
        let instance = InstanceId::new(id).map_err(|error| types::Error::new(error.to_string()))?;
        host.load(LoadSpec::new(source, instance))
            .await
            .map_err(|error| types::Error::new(error.to_string()))?;
    }
    Ok(host)
}

struct ReadGateway {
    inner: Arc<AbilityHost>,
}

#[async_trait]
impl Gateway for ReadGateway {
    async fn invoke(&self, request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        if !surface::installed(surface::AGENT, request.instance.as_str()) {
            return Err(ability_support::Error::unavailable("实例未开放"));
        }
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter(|entry| surface::installed(surface::AGENT, entry.instance.as_str()))
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !surface::installed(surface::AGENT, instance.as_str()) {
            return None;
        }
        self.inner.get_entry(instance, method)
    }
}
