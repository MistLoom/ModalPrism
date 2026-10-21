//! 组合根持有的运行时资源（存储、配置、调度引擎）。仅供组合根与各 Ability 工厂注入，不是 LEAF Gateway 本身。

pub mod api;

pub use collector_gateway::{
    self as gateway, BoundEgress, BoundPaid, EgressGw, HttpTransport, PaidBridge, PaidGw,
    SystemResolver, call_policy_from, policy_from, policy_of, serve_proxy,
};
pub use collector_runtime::{
    self as engine, Binds, Error, invalid_input, refuse_remote, require_loopback, serve_push,
    PushCommit,
};
pub use collector_runtime::plans::{capacities, flush_interval, snapshot_interval};
pub use strata_shared::{persist, strata_options};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use collector_runtime::execute::{self, FetchEnv, FollowWork};
use collector_runtime::stage::DiskStaging;
use egress::Resolver;
use config::{ConfigStore, LoadOptions, Snapshot};
use egress::TcpDial;
use planner::{Handler, Planner, Receipts};
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::{
    ApplyOpts, ChangeSet, ErrorClass, FacetKind, FacetRef, Locator, Producer, TaskReason, Timestamp,
    VersionSel,
};

pub struct HostResources {
    data_dir: PathBuf,
    config_dir: PathBuf,
    load_options: LoadOptions,
    config: Arc<ConfigStore>,
    strata: Arc<Strata>,
    planner: Planner<persist::StrataQueue, persist::Hot>,
    egress: Arc<EgressGw>,
    paid: Arc<PaidGw>,
    proxy: Mutex<String>,
    push: accessors_builtin::PushAccessor,
    browser_program: Mutex<PathBuf>,
}

impl HostResources {
    pub async fn open(data: &Path, config_dir: &Path, binds: Binds) -> Result<Arc<Self>, Error> {
        let load_options = LoadOptions::from_process();
        let snapshot = config::validate(config_dir, &load_options)?;
        refuse_remote(&snapshot, data)?;
        require_loopback(binds.http)?;
        require_loopback(binds.proxy)?;
        require_loopback(binds.push)?;
        let config = Arc::new(ConfigStore::open(config_dir, load_options.clone())?);
        let opening = policy_of(&snapshot.system);
        let strata = Arc::new(Strata::open(data, strata_options(&snapshot.system)).await?);
        let planner = Planner::new(
            persist::StrataQueue(Arc::clone(&strata)),
            persist::Hot::new(Arc::clone(&strata), "planner"),
        );
        let egress = Arc::new(egress::Gateway::new(
            persist::Hot::new(Arc::clone(&strata), "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let paid = Arc::new(paid::Gateway::new(
            persist::Hot::new(Arc::clone(&strata), "paid"),
            PaidBridge {
                egress: Arc::clone(&egress),
                fallback: opening,
            },
            call_policy_from(&snapshot.system, &Value::Null),
        ));
        let browser = std::env::var("MODALPRISM_MOLI").unwrap_or_else(|_| "moli".into());
        Ok(Arc::new(Self {
            data_dir: collector_runtime::normalize(data),
            config_dir: config_dir.to_path_buf(),
            load_options,
            config,
            strata,
            planner,
            egress,
            paid,
            proxy: Mutex::new(format!("http://{}", binds.proxy)),
            push: accessors_builtin::PushAccessor::new(),
            browser_program: Mutex::new(PathBuf::from(browser)),
        }))
    }

    pub fn config(&self) -> Arc<ConfigStore> {
        Arc::clone(&self.config)
    }

    pub fn strata(&self) -> Arc<Strata> {
        Arc::clone(&self.strata)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn set_proxy(&self, url: impl Into<String>) {
        *self.proxy.lock().expect("代理") = url.into();
    }

    pub fn proxy_url(&self) -> String {
        self.proxy.lock().expect("代理").clone()
    }

    pub fn browser_program(&self) -> PathBuf {
        self.browser_program.lock().expect("浏览器").clone()
    }

    pub fn set_browser_program(&self, program: impl Into<PathBuf>) {
        *self.browser_program.lock().expect("浏览器") = program.into();
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        self.planner.shutdown(Timestamp::now()).await?;
        Ok(())
    }

    pub(crate) fn fetch_env(&self, timeout: Duration) -> FetchEnv {
        FetchEnv {
            snapshot: self.config.applied(),
            staging_dir: self.data_dir.join("staging"),
            proxy: self.proxy_url(),
            browser: self.browser_program(),
            timeout,
        }
    }

    pub fn access_context(&self, egress_policy: egress::Policy, paid_policy: paid::CallPolicy) -> accessors::AccessContext {
        accessors::AccessContext {
            egress: Arc::new(BoundEgress {
                inner: Arc::clone(&self.egress),
                policy: egress_policy,
            }),
            paid: Arc::new(BoundPaid {
                inner: Arc::clone(&self.paid),
                policy: paid_policy,
            }),
            sessions: Arc::new(execute::NoSession),
            staging: Arc::new(DiskStaging {
                dir: self.data_dir.join("staging"),
            }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub(crate) fn push(&self) -> &accessors_builtin::PushAccessor {
        &self.push
    }

    pub async fn commit_inbound(
        &self,
        source_id: &str,
        envelopes: &[strata_model::RawEnvelope],
    ) -> Result<(), Error> {
        let snapshot = self.config.applied();
        let module = snapshot
            .modules
            .get(source_id)
            .cloned()
            .ok_or_else(|| Error::new("config"))?;
        let env = self.fetch_env(Duration::from_secs(30));
        let ops = execute::inbound_ops(&env, source_id, &module, envelopes)
            .await
            .map_err(|error| Error::new(error.message))?;
        if ops.is_empty() {
            return Ok(());
        }
        self.strata
            .apply(
                ChangeSet {
                    producer: Producer {
                        module: "push".into(),
                        version: "1".into(),
                        config_generation: snapshot.generation,
                    },
                    idempotency_key: None,
                    ack: None,
                    ops,
                },
                ApplyOpts::default(),
            )
            .await?;
        self.strata.flush().await?;
        Ok(())
    }

    pub fn egress_gateway(&self) -> Arc<EgressGw> {
        Arc::clone(&self.egress)
    }

    /// 组合根与控制台接入层使用的统一方法名（`ability.method`）。Ability 实现应调用 `api::invoke`。
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Error> {
        dispatch_impl(self, method, args).await
    }
}

#[async_trait]
impl PushCommit for HostResources {
    fn applied_snapshot(&self) -> Arc<Snapshot> {
        self.config.applied()
    }

    fn access_context(&self) -> accessors::AccessContext {
        let snap = self.config.applied();
        let policy = policy_of(&snap.system);
        let paid_policy = call_policy_from(&snap.system, &Value::Null);
        HostResources::access_context(self, policy, paid_policy)
    }

    fn push_accessor(&self) -> &accessors_builtin::PushAccessor {
        self.push()
    }

    async fn commit_inbound(
        &self,
        source_id: &str,
        envelopes: &[strata_model::RawEnvelope],
    ) -> Result<(), Error> {
        HostResources::commit_inbound(self, source_id, envelopes).await
    }
}

pub(crate) async fn dispatch(host: &HostResources, method: &str, args: Value) -> Result<Value, Error> {
    dispatch_impl(host, method, args).await
}

struct TaskHandler {
    config: Arc<ConfigStore>,
    data_dir: PathBuf,
    proxy: String,
    browser: PathBuf,
    egress: Arc<EgressGw>,
    paid: Arc<PaidGw>,
    strata: Arc<Strata>,
    timeout: Duration,
}

#[async_trait]
impl Handler for TaskHandler {
    async fn execute(
        &self,
        task: &strata_model::AccessTask,
        cursor: Option<Value>,
    ) -> Result<planner::TaskSuccess, planner::Classified> {
        let resource = if task.reason == TaskReason::Follow {
            let key = match &task.locator {
                Locator::Url { url } => format!("url:{url}"),
                _ => String::new(),
            };
            if key.is_empty() {
                None
            } else {
                self.strata.resolve(&key).await.ok().flatten()
            }
        } else {
            None
        };
        let env = FetchEnv {
            snapshot: self.config.applied(),
            staging_dir: self.data_dir.join("staging"),
            proxy: self.proxy.clone(),
            browser: self.browser.clone(),
            timeout: self.timeout,
        };
        let module = env
            .snapshot
            .modules
            .get(task.source_id.as_str())
            .cloned()
            .ok_or_else(|| planner_classified(ErrorClass::Config, "config"))?;
        let accessor_id = if task.reason == TaskReason::Follow {
            task.accessor.as_str()
        } else {
            module
                .pointer("/access/accessor")
                .and_then(Value::as_str)
                .unwrap_or(task.accessor.as_str())
        };
        let implementation = env
            .snapshot
            .modules
            .get(accessor_id)
            .and_then(|item| item.get("implementation"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let accessor = execute::open_accessor(implementation, &env.browser, &env.proxy)
            .map_err(|error| planner_classified(ErrorClass::Config, error.message))?;
        let source_policy = module
            .get("policy")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let accessor_policy = env
            .snapshot
            .modules
            .get(accessor_id)
            .and_then(|item| item.get("policy"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let egress_policy = policy_from(&env.snapshot.system, &source_policy);
        let paid_policy = call_policy_from(&env.snapshot.system, &accessor_policy);
        let access = accessors::AccessContext {
            egress: Arc::new(BoundEgress {
                inner: Arc::clone(&self.egress),
                policy: egress_policy.clone(),
            }),
            paid: Arc::new(BoundPaid {
                inner: Arc::clone(&self.paid),
                policy: paid_policy,
            }),
            sessions: Arc::new(execute::NoSession),
            staging: Arc::new(DiskStaging {
                dir: self.data_dir.join("staging"),
            }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let fetched = execute::fetch(
            &env,
            &self.paid,
            accessor,
            &access,
            egress_policy,
            task,
            cursor,
            resource,
        )
        .await?;
        let mut success = fetched.success;
        success.follows = open_follows(&self.strata, fetched.follows, fetched.max_fetches).await;
        Ok(success)
    }
}

fn planner_classified(class: ErrorClass, message: impl Into<String>) -> planner::Classified {
    planner::Classified {
        class,
        message: message.into(),
        retry_after: None,
        retry_at: None,
    }
}

async fn open_follows(
    strata: &Strata,
    works: Vec<FollowWork>,
    max_fetches: usize,
) -> Vec<strata_model::AccessTask> {
    let mut out = Vec::new();
    for work in works {
        if out.len() >= max_fetches {
            break;
        }
        if let Some(rid) = strata.resolve(&work.identity).await.ok().flatten() {
            let reference = FacetRef::new(
                rid,
                FacetKind::Struct,
                Some("page".into()),
                VersionSel::Latest,
            );
            if let Ok(reference) = reference {
                if let Ok(Some(facet)) = strata.get(&reference, true).await {
                    if needs_met(facet.value.as_ref(), &work.needs) {
                        continue;
                    }
                }
            }
        }
        out.push(work.task);
    }
    out
}

fn needs_met(value: Option<&Value>, needs: &[String]) -> bool {
    let Some(value) = value else {
        return false;
    };
    needs.iter().all(|need| {
        value.get(need).is_some_and(|item| match item {
            Value::String(text) => !text.is_empty(),
            Value::Null => false,
            _ => true,
        })
    })
}

struct PaidReceipts {
    paid: Arc<PaidGw>,
}

#[async_trait]
impl Receipts for PaidReceipts {
    async fn complete(&self, receipt_id: &str) -> Result<(), planner::PlannerError> {
        self.paid
            .complete(receipt_id)
            .await
            .map_err(|error| planner::PlannerError::with_source(error.message.clone(), error))
    }
}

async fn dispatch_impl(services: &HostResources, method: &str, args: Value) -> Result<Value, Error> {
    let args = match args {
        Value::Null => Value::Object(Default::default()),
        Value::Object(_) => args,
        _ => return Err(Error::new("spec 无效")),
    };
    if let Some((ability, name)) = method.split_once('.') {
        if let Some(builtin) = ability.strip_prefix("accessor_") {
            return accessor_call(services, builtin, name, &args).await;
        }
    }
    match method {
        "strata_read.kinds" => strata_kinds(services, &args).await,
        "strata_read.has" => strata_has(services, &args).await,
        "strata_read.get" => strata_get(services, &args).await,
        "strata_read.slice" => strata_slice(services, &args).await,
        "strata_read.resolve" => strata_resolve(services, &args).await,
        "strata_read.history" => strata_history(services, &args).await,
        "strata_read.lineage" => strata_lineage(services, &args).await,
        "strata_read.related" => strata_related(services, &args).await,
        "strata_read.query" => strata_query(services, &args).await,
        "strata_read.changes" => strata_changes(services, &args).await,
        "strata_write.apply" => strata_apply(services, &args).await,
        "strata_queue.enqueue" => strata_enqueue(services, &args).await,
        "strata_queue.lease" => strata_lease(services, &args).await,
        "strata_queue.extend" => strata_extend(services, &args).await,
        "strata_queue.nack" => strata_nack(services, &args).await,
        "committer.flush" => committer_flush(services).await,
        "committer.maintain" => committer_maintain(services).await,
        "committer.recover" => committer_recover(services).await,
        "committer.status" => Ok(status_paths(services)),
        "planner.tick" => planner_tick(services).await,
        "planner.trigger" => planner_trigger(services, &args).await,
        "planner.next" => planner_next(services, &args).await,
        "runner.run_task" => runner_run(services, &args).await,
        "health.evaluate" => health_evaluate(services).await,
        "health.status" => health_status(services, &args).await,
        "config.load" => config_load(services).await,
        "config.generation" => Ok(generation_of(&services.config.snapshot())),
        "config_inspect.validate" => config_validate(services).await,
        "config_inspect.diff" => config_diff(services).await,
        "config_inspect.explain" => config_explain(services, &args).await,
        "egress.check" => egress_check(services, &args).await,
        "egress.status" => Ok(json!({ "proxy": services.proxy_url() })),
        "paid_gateway.reserve" => Err(Error::new("reserve")),
        "paid_gateway.complete" => paid_complete(services, &args).await,
        "paid_gateway.status" => paid_status(services, &args).await,
        _ => Err(Error::new("spec 无效")),
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, Error> {
    serde_json::from_value(value.clone()).map_err(|_| Error::new("spec 无效"))
}

fn field<'a>(args: &'a Value, key: &str) -> Result<&'a Value, Error> {
    args.get(key).ok_or_else(|| Error::new("spec 无效"))
}

fn text_field(args: &Value, key: &str) -> Result<String, Error> {
    field(args, key)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::new("spec 无效"))
}

async fn strata_kinds(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        include_variants: bool,
    }
    let args: Args = parse(args)?;
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let kinds = services.strata.kinds(&rid, args.include_variants).await?;
    serde_json::to_value(kinds).map_err(|source| Error::with_source("spec 无效", source))
}

async fn strata_has(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        kind: strata_model::FacetKind,
        #[serde(default)]
        variant: Option<String>,
    }
    let args: Args = parse(args)?;
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let found = services
        .strata
        .has(&rid, &args.kind, args.variant.as_deref())
        .await?;
    Ok(Value::Bool(found))
}

async fn strata_get(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        facet: String,
        #[serde(default = "lazy_default")]
        lazy: bool,
    }
    fn lazy_default() -> bool {
        true
    }
    let args: Args = parse(args)?;
    let reference =
        strata_model::FacetRef::parse(&args.facet).map_err(|_| Error::new("spec 无效"))?;
    let Some(facet) = services.strata.get(&reference, args.lazy).await? else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "version": facet.version,
        "contentHash": facet.content_hash,
        "mediaType": facet.media_type,
        "size": facet.size,
        "text": facet.text,
        "struct": facet.value,
        "part": facet.blob,
    }))
}

async fn strata_slice(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        span: String,
    }
    let args: Args = parse(args)?;
    let span = strata_model::SpanRef::parse(&args.span).map_err(|_| Error::new("spec 无效"))?;
    let Some(facet) = services.strata.get(&span.facet, true).await? else {
        return Err(Error::new("gateway"));
    };
    if facet.text.is_none() {
        return Err(Error::new("gateway"));
    }
    let bytes = services.strata.slice(&span).await?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| Error::new("gateway"))?;
    Ok(Value::String(text))
}

async fn strata_resolve(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let identity = text_field(args, "identity")?;
    let rid = services.strata.resolve(&identity).await?;
    Ok(match rid {
        Some(rid) => Value::String(rid.to_string()),
        None => Value::Null,
    })
}

async fn strata_history(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let facet = text_field(args, "facet")?;
    let reference = strata_model::FacetRef::parse(&facet).map_err(|_| Error::new("spec 无效"))?;
    let versions = services.strata.history(&reference).await?;
    let items: Vec<Value> = versions
        .into_iter()
        .map(|item| {
            json!({
                "version": item.version,
                "contentHash": item.content_hash,
                "createdAt": item.created_at,
                "seq": item.seq,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

async fn strata_lineage(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        facet: String,
        #[serde(default)]
        direction: Option<String>,
        #[serde(default = "one")]
        depth: u32,
    }
    fn one() -> u32 {
        1
    }
    let args: Args = parse(args)?;
    let direction = match args.direction.as_deref() {
        None | Some("up") => strata_model::LineageDir::Up,
        Some("down") => strata_model::LineageDir::Down,
        Some(_) => return Err(Error::new("spec 无效")),
    };
    let reference =
        strata_model::FacetRef::parse(&args.facet).map_err(|_| Error::new("spec 无效"))?;
    let graph = services
        .strata
        .lineage(&reference, direction, args.depth)
        .await?;
    Ok(json!({
        "nodes": graph.nodes.iter().map(|node| json!({
            "facet": node.facet.to_string(),
            "module": node.module,
            "version": node.version,
            "paramsHash": node.params_hash,
        })).collect::<Vec<_>>(),
        "edges": graph.edges.iter().map(|edge| json!({
            "from": edge.from.to_string(),
            "to": edge.to.to_string(),
        })).collect::<Vec<_>>(),
    }))
}

async fn strata_related(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        rel: Option<String>,
        #[serde(default = "out_dir")]
        direction: String,
    }
    fn out_dir() -> String {
        "out".into()
    }
    let args: Args = parse(args)?;
    let direction = match args.direction.as_str() {
        "out" => strata_model::EdgeDir::Out,
        "in" => strata_model::EdgeDir::In,
        _ => return Err(Error::new("spec 无效")),
    };
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let rows = services
        .strata
        .related(&rid, args.rel.as_deref(), direction)
        .await?;
    let items = rows
        .into_iter()
        .map(|row| {
            json!({
                "src": row.src.to_string(),
                "rel": row.rel,
                "dst": row.dst.to_string(),
                "attrs": row.attrs,
                "commitId": row.commit_id,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

async fn strata_query(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        query: strata_model::Query,
    }
    let args: Args = parse(args)?;
    let page = services.strata.query(args.query).await?;
    Ok(json!({
        "rows": page.rows,
        "nextCursor": page.next_cursor,
    }))
}

async fn strata_changes(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        after: u64,
        #[serde(default)]
        limit: Option<u32>,
    }
    let args: Args = parse(args)?;
    let rows = services.strata.changes(args.after, args.limit).await?;
    let items = rows
        .into_iter()
        .map(|row| {
            json!({
                "snapshot": row.snapshot,
                "commitIds": row.commit_ids,
                "publishedAt": row.published_at,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

async fn strata_apply(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        changeset: ChangeSet,
    }
    let args: Args = parse(args)?;
    let result = services
        .strata
        .apply(args.changeset, ApplyOpts::default())
        .await?;
    serde_json::to_value(result).map_err(|source| Error::with_source("spec 无效", source))
}

async fn strata_enqueue(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        tasks: Vec<strata_model::AccessTask>,
    }
    let args: Args = parse(args)?;
    services.strata.enqueue(&args.tasks).await?;
    Ok(json!({}))
}

async fn strata_lease(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        queue: strata_model::QueueName,
        count: u64,
        lease_ms: u64,
    }
    let args: Args = parse(args)?;
    let count = usize::try_from(args.count).map_err(|_| Error::new("spec 无效"))?;
    let tasks = services
        .strata
        .lease(args.queue, count, Duration::from_millis(args.lease_ms))
        .await?;
    serde_json::to_value(tasks).map_err(|source| Error::with_source("spec 无效", source))
}

async fn strata_extend(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        task_id: String,
        lease_ms: u64,
    }
    let args: Args = parse(args)?;
    let task_id = strata_model::TaskId::new(args.task_id).map_err(|_| Error::new("spec 无效"))?;
    services
        .strata
        .extend(&task_id, Duration::from_millis(args.lease_ms))
        .await?;
    Ok(json!({}))
}

async fn strata_nack(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        task_id: String,
        retry_at: String,
        error: String,
    }
    let args: Args = parse(args)?;
    let task_id = strata_model::TaskId::new(args.task_id).map_err(|_| Error::new("spec 无效"))?;
    let retry_at = Timestamp::parse(&args.retry_at).map_err(|_| Error::new("spec 无效"))?;
    services
        .strata
        .nack(&task_id, retry_at, &args.error)
        .await?;
    Ok(json!({}))
}

async fn committer_flush(services: &HostResources) -> Result<Value, Error> {
    let snapshot = services.strata.flush().await?;
    Ok(json!({ "snapshot": snapshot }))
}

fn status_paths(services: &HostResources) -> Value {
    json!({
        "control": services.data_dir.join("control.db"),
        "lance": services.data_dir.join("lance"),
        "staging": services.data_dir.join("staging"),
    })
}

async fn committer_maintain(services: &HostResources) -> Result<Value, Error> {
    services.strata.recover().await?;
    Ok(status_paths(services))
}

async fn committer_recover(services: &HostResources) -> Result<Value, Error> {
    services.strata.recover().await?;
    Ok(json!({}))
}

async fn planner_tick(services: &HostResources) -> Result<Value, Error> {
    let plans = collector_runtime::plans::plans_of(&services.config.applied());
    let tasks = services
        .planner
        .tick(&plans, &capacities(), Timestamp::now())
        .await?;
    serde_json::to_value(tasks).map_err(|source| Error::with_source("spec 无效", source))
}

async fn planner_trigger(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    services.planner.make_due(&source, Timestamp::now()).await?;
    planner_tick(services).await
}

async fn planner_next(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    let status = services.planner.source_status(&source).await?;
    Ok(json!({
        "nextAt": status.next_at,
        "intervalMs": status.interval_ms,
        "healthStatus": status.health_status,
        "alert": status.alert,
        "skipReason": status.skip_reason,
        "budgetUsed": status.budget_used,
        "lastYieldAt": status.last_yield_at,
        "todayYieldCount": status.today_yield_count,
        "lastError": status.last_error,
    }))
}

async fn runner_run(services: &HostResources, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        task: strata_model::AccessTask,
    }
    let args: Args = parse(args)?;
    let snapshot = services.config.applied();
    let plans = collector_runtime::plans::plans_of(&snapshot);
    let Some(mut plan) = plans
        .into_iter()
        .find(|plan| plan.source_id == args.task.source_id)
    else {
        services
            .strata
            .nack(&args.task.task_id, Timestamp::now(), "config")
            .await?;
        return Ok(json!({ "nacked": "config" }));
    };
    plan.generation = args.task.generation;
    let now = Timestamp::now();
    let deadline = now
        .checked_add(plan.timeout)
        .map_err(|source| Error::with_source("节奏", source))?;
    let handler = TaskHandler {
        config: services.config(),
        data_dir: services.data_dir().to_path_buf(),
        proxy: services.proxy_url(),
        browser: services.browser_program(),
        egress: services.egress_gateway(),
        paid: Arc::clone(&services.paid),
        strata: services.strata(),
        timeout: plan.timeout,
    };
    let receipts = PaidReceipts {
        paid: Arc::clone(&services.paid),
    };
    let follows = services
        .planner
        .run(&plan, args.task, &handler, &receipts, now, deadline)
        .await?;
    let follows = serde_json::to_value(&follows).unwrap_or(Value::Array(Vec::new()));
    Ok(json!({ "ok": true, "follows": follows }))
}

async fn health_evaluate(services: &HostResources) -> Result<Value, Error> {
    let now = Timestamp::now();
    let applied = services.config.applied();
    let ops = services
        .planner
        .snapshot(now, snapshot_interval(&applied))
        .await?;
    if !ops.is_empty() {
        let changeset = ChangeSet {
            producer: Producer {
                module: "health".into(),
                version: "1".into(),
                config_generation: applied.generation,
            },
            idempotency_key: None,
            ack: None,
            ops,
        };
        services
            .strata
            .apply(changeset, ApplyOpts::default())
            .await?;
        services.planner.commit_snapshot(now).await?;
    }
    Ok(json!({}))
}

async fn health_status(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    let status = services.planner.source_status(&source).await?;
    Ok(json!({
        "key": format!("source/{source}"),
        "status": status.health_status,
        "alert": status.alert,
        "lastError": status.last_error,
        "lastYieldAt": status.last_yield_at,
        "todayYieldCount": status.today_yield_count,
    }))
}

fn generation_of(snapshot: &Snapshot) -> Value {
    json!({ "generation": snapshot.generation })
}

async fn config_load(services: &HostResources) -> Result<Value, Error> {
    services.config.reload()?;
    Ok(generation_of(&services.config.snapshot()))
}

async fn config_validate(services: &HostResources) -> Result<Value, Error> {
    config::validate(&services.config_dir, &services.load_options)?;
    Ok(json!({ "ok": true }))
}

async fn config_diff(services: &HostResources) -> Result<Value, Error> {
    let fresh = config::validate(&services.config_dir, &services.load_options)?;
    let changes = config::diff(&services.config.snapshot(), &fresh);
    let items = changes
        .into_iter()
        .map(|change| {
            json!({
                "path": change.path,
                "before": change.before,
                "after": change.after,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

async fn config_explain(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let module = text_field(args, "module")?;
    let fields = config::explain(&services.config.snapshot(), &module)?;
    let items = fields
        .into_iter()
        .map(|field| {
            json!({
                "path": field.path,
                "value": field.value,
                "file": field.file,
                "line": field.line,
                "presetId": field.preset_id,
                "viaPreset": field.via_preset,
                "systemScope": field.system_scope,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

async fn egress_check(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let raw = text_field(args, "url")?;
    let url = url::Url::parse(&raw).map_err(|_| Error::new("gateway"))?;
    let host = url.host_str().ok_or_else(|| Error::new("gateway"))?;
    let addresses = SystemResolver
        .lookup(host)
        .await
        .map_err(|error| Error::with_source(error.message.clone(), error))?;
    let policy = policy_of(&services.config.applied().system);
    let blocked = if policy.private_networks == egress::PrivateNetworks::Allow {
        false
    } else {
        addresses.is_empty() || addresses.iter().all(|ip| egress::is_blocked(*ip))
    };
    let rendered: Vec<String> = addresses.iter().map(ToString::to_string).collect();
    Ok(json!({ "blocked": blocked, "addresses": rendered }))
}

async fn paid_complete(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let receipt = text_field(args, "receiptId")?;
    services
        .paid
        .complete(&receipt)
        .await
        .map_err(|error| Error::with_source(error.message.clone(), error))?;
    Ok(json!({}))
}

async fn paid_status(services: &HostResources, args: &Value) -> Result<Value, Error> {
    let receipt = text_field(args, "receiptId")?;
    let status = services
        .paid
        .status(&receipt)
        .await
        .map_err(|error| Error::with_source(error.message.clone(), error))?;
    Ok(json!({ "status": status }))
}

async fn accessor_call(
    services: &HostResources,
    builtin: &str,
    name: &str,
    args: &Value,
) -> Result<Value, Error> {
    let id = match builtin {
        "paid_api" => "paid-api",
        "http" | "rss" | "api" | "search" | "push" | "file" | "browser" => builtin,
        _ => return Err(Error::new("spec 无效")),
    };
    let implementation = format!("builtin:{id}");
    let browser = services.browser_program.lock().expect("浏览器").clone();
    let accessor = execute::open_accessor(&implementation, &browser, &services.proxy_url())
        .map_err(|error| Error::new(error.message))?;
    match name {
        "validate" => {
            let spec = field(args, "spec")?;
            accessor
                .validate(spec)
                .map_err(|error| Error::new(error.message))?;
            Ok(json!({}))
        }
        "access" => accessor_access(services, accessor, args).await,
        _ => Err(Error::new("spec 无效")),
    }
}

async fn accessor_access(
    services: &HostResources,
    accessor: Box<dyn accessors::Accessor>,
    args: &Value,
) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        task_id: String,
        source_id: String,
        locator: strata_model::Locator,
        spec: Value,
        #[serde(default)]
        cursor: Option<Value>,
        #[serde(default)]
        policy: Option<Value>,
    }
    let args: Args = parse(args)?;
    let policy_value = args
        .policy
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let request = accessors::AccessRequest {
        task_id: strata_model::TaskId::new(args.task_id).map_err(|_| Error::new("spec 无效"))?,
        source_id: strata_model::SourceId::new(args.source_id)
            .map_err(|_| Error::new("spec 无效"))?,
        locator: args.locator,
        spec: args.spec,
        cursor: args.cursor,
        hints: accessors::AccessHints::default(),
        policy: Arc::new(policy_value.clone()),
        deadline: std::time::Instant::now() + Duration::from_secs(30),
    };
    let snap = services.config.applied();
    let egress_policy = policy_of(&snap.system);
    let paid_policy = call_policy_from(&snap.system, &policy_value);
    let envelopes = accessor
        .access(
            request,
            &services.access_context(egress_policy, paid_policy),
        )
        .await
        .map_err(|error| Error::new(error.message))?;
    serde_json::to_value(envelopes).map_err(|source| Error::with_source("spec 无效", source))
}

