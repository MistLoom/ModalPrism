//! 只读进程。目录只含读取、健康状态、配置检查和两个抽象类型。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{AbilityProvider, AbilitySpec, InstanceId, LoadSpec, MethodId, SourceId};
use async_trait::async_trait;
use frame_core::{AccessKind, CatalogEntry, Gateway, InvokeRequest, InvokeResponse, Port};
use frame_port_mcp::McpPort;
use serde_json::{Value, json};
use types::ReadRoot;

const OPEN: &[&str] = &[
    "strata_read",
    "health",
    "config_inspect",
    "type_article",
    "type_hotlist",
];

const READ_METHODS: &[(&str, &str)] = &[
    ("kinds", "列出资源上的 facet 种类"),
    ("has", "判断资源是否有某种 facet"),
    ("get", "读取 facet 的版本与元数据"),
    ("slice", "读取文本区间"),
    ("resolve", "按身份键解析资源"),
    ("history", "列出 facet 版本"),
    ("lineage", "读取 facet 血缘"),
    ("related", "读取资源关系"),
    ("query", "按查询对象读取"),
    ("changes", "读取已可见变更"),
];

const INSPECT_METHODS: &[(&str, &str)] = &[
    ("validate", "重新校验配置目录"),
    ("diff", "比较当前快照与目录"),
    ("explain", "解释模块字段来源"),
];

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
        entry.instance = instance.clone();
        Some(entry)
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
    let providers: Vec<(&str, Arc<dyn AbilityProvider>)> = vec![
        (
            "strata_read",
            read_provider(Arc::clone(&root), "strata_read", READ_METHODS, true),
        ),
        (
            "health",
            read_provider(
                Arc::clone(&root),
                "health",
                &[("status", "读取信源健康")],
                true,
            ),
        ),
        (
            "config_inspect",
            read_provider(Arc::clone(&root), "config_inspect", INSPECT_METHODS, false),
        ),
        ("type_article", type_article::provider(Arc::clone(&root))),
        ("type_hotlist", type_hotlist::provider(Arc::clone(&root))),
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

fn read_provider(
    root: Arc<ReadRoot>,
    id: &'static str,
    methods: &[(&'static str, &'static str)],
    stateful: bool,
) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must(id);
    if stateful {
        spec = spec.stateful();
    }
    for &(method, description) in methods {
        let root = Arc::clone(&root);
        spec = spec.on(
            method,
            description,
            json!({"type": "object"}),
            move |_ctx, args| {
                let root = Arc::clone(&root);
                let name = format!("{id}.{method}");
                ability_support::box_fut(async move { map(root.call(&name, args).await) })
            },
        );
    }
    spec.into_provider()
}

fn map(result: Result<Value, types::Error>) -> ability_support::Result<Value> {
    result.map_err(|error| {
        if types::invalid_input(&error.message) {
            ability_support::Error::invalid_args(error.message)
        } else {
            ability_support::Error::failed(error.to_string())
        }
    })
}

struct ReadGateway {
    inner: Arc<AbilityHost>,
}

#[async_trait]
impl Gateway for ReadGateway {
    async fn invoke(&self, request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        if !OPEN.contains(&request.instance.as_str()) {
            return Err(ability_support::Error::unavailable("实例未开放"));
        }
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter(|entry| OPEN.contains(&entry.instance.as_str()))
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !OPEN.contains(&instance.as_str()) {
            return None;
        }
        self.inner.get_entry(instance, method)
    }
}
