//! 只读进程。目录只含读取、健康状态、配置检查和两个抽象类型。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    InstanceId, LoadSpec, MethodId, SourceId, discover_ability_roots, load_ability_decl,
};
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
        // web.search / web.fetch / web.parse：代理到本机回环领域 API（只读）。
        if request.instance.as_str() == "web" {
            let data = web_proxy(request.method.as_str(), request.args.clone())
                .await
                .map_err(ability_support::Error::failed)?;
            return Ok(InvokeResponse {
                data,
                locals_scope: None,
            });
        }
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

/// web 工具 → 本机回环领域 API。只读投影：search / fetch / parse。
fn web_provider() -> Arc<dyn ability_support::AbilityProvider> {
    use ability_support::{AbilityId, CtxFnAbility, MethodMeta, StaticProvider};
    use serde_json::json;

    let manifest = ability_support::AbilityManifest {
        id: AbilityId::new("web").unwrap(),
        depends: vec![],
        stateful: false,
        config_schema: json!({}),
        methods: ["search", "fetch", "parse"]
            .iter()
            .map(|method| MethodMeta::new(method, "", json!({ "type": "object" })))
            .collect(),
        events: vec![],
    };
    Arc::new(StaticProvider::new(Arc::new(CtxFnAbility::new(
        manifest,
        |ctx, args| {
            let method = ctx
                .method()
                .map(|method| method.as_str().to_string())
                .unwrap_or_default();
            Box::pin(async move {
                let data = web_proxy(&method, args)
                    .await
                    .map_err(ability_support::Error::failed)?;
                Ok(data)
            })
        },
    ))))
}

/// web 工具 → 本机回环领域 API 的 HTTP 代理。
async fn web_proxy(method: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let base = std::env::var("MODALPRISM_HTTP").unwrap_or_else(|_| "http://127.0.0.1:17880".into());
    let path = match method {
        "search" => "/api/v1/search",
        "fetch" => "/api/v1/fetch",
        "parse" => "/api/v1/parse",
        other => return Err(format!("web 能力无方法 {other}")),
    };
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("领域 API 不可达: {error}"))?;
    let status = response.status();
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("领域 API 响应解析失败: {error}"))?;
    if !status.is_success() {
        return Err(format!("领域 API 返回 {status}: {value}"));
    }
    Ok(value)
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

fn protocol_kinds_for(workspace: &Path, id: &str) -> Vec<AccessKind> {
    for root in discover_ability_roots(workspace) {
        let yml = root.join("ability.yml");
        if let Ok(decl) = load_ability_decl(&yml) {
            if decl.id == id {
                return decl
                    .protocols
                    .iter()
                    .filter_map(|kind| AccessKind::from_protocol(*kind))
                    .collect();
            }
        }
    }
    Vec::new()
}
async fn link(root: Arc<ReadRoot>) -> Result<Arc<AbilityHost>, types::Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    let ctx: Arc<dyn ReadCtx> = Arc::clone(&root) as Arc<dyn ReadCtx>;
    let providers: Vec<(&str, Arc<dyn ability_support::AbilityProvider>)> = vec![
        ("catalog_admin", catalog_admin::provider(Arc::clone(&ctx))),
        ("strata_read", strata_read::provider(Arc::clone(&ctx))),
        ("health", health::provider(Arc::clone(&ctx))),
        ("config_inspect", config_inspect::provider(Arc::clone(&ctx))),
        ("type_article", type_article::provider(Arc::clone(&ctx))),
        ("type_hotlist", type_hotlist::provider(Arc::clone(&ctx))),
        ("workflow", ability_workflow::provider(Arc::clone(&ctx))),
        ("web", web_provider()),
    ];
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (id, provider) in providers {
        // 协议投影改从 ability.yml 读取（AbilityManifest 不再承载 protocols）。
        let kinds = protocol_kinds_for(&workspace, id);
        host.install_str(id, provider)
            .await
            .map_err(|error| types::Error::new(error.to_string()))?;
        if kinds.is_empty() {
            host.set_protocol_support(
                id,
                vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
            );
        } else {
            host.set_protocol_support(id, kinds);
        }
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
        if !open(request.instance.as_str(), request.method.as_str()) {
            return Err(ability_support::Error::unavailable("实例未开放"));
        }
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter(|entry| open(entry.instance.as_str(), entry.method.as_str()))
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !open(instance.as_str(), method.as_str()) {
            return None;
        }
        self.inner.get_entry(instance, method)
    }
}

/// MCP 面按方法放行：只读面 + 写白名单（可恢复的声明文件草稿）。
/// 装了能力不等于整个能力都对外：删除、设置与采集驱动写一律不出去。
fn open(instance: &str, method: &str) -> bool {
    surface::installed(surface::AGENT, instance)
        && (surface::exposes(surface::AGENT_MCP, instance, method)
            || surface::mcp_write_allowed(instance, method))
}
