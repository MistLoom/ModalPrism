//! 采集进程的存储、配置、调度和监听装配。

mod error;
mod execute;
mod listen;
mod plans;
mod queue;
mod stage;
mod transport;

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use config::{ConfigStore, LoadOptions, Snapshot};
use egress::{Resolver, TcpDial};
use planner::{Handler, Planner, Receipts};
use serde_json::{Value, json};
use strata_client::{Options, Strata};
use strata_model::{
    ApplyOpts, ChangeSet, FacetKind, FacetRef, Locator, Producer, TaskReason, Timestamp, VersionSel,
};

pub use error::{Error, invalid_input};
pub use listen::{serve_proxy, serve_push};
pub use plans::{capacities, flush_interval, snapshot_interval};
pub use transport::{EgressGw, policy_of};

use execute::{FetchState, FollowWork, NoSession};
use queue::{Hot, StrataQueue};
use stage::DiskStaging;
use transport::{HttpTransport, PaidBridge, PaidGw, SystemResolver};

/// 三个环回监听地址。
pub struct Binds {
    pub http: SocketAddr,
    pub proxy: SocketAddr,
    pub push: SocketAddr,
}

/// 拒绝非环回地址。不创建数据目录。
pub fn require_loopback(addr: SocketAddr) -> Result<(), Error> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(Error::new("监听地址不是环回地址"))
    }
}

pub struct Services {
    data_dir: PathBuf,
    config_dir: PathBuf,
    load_options: LoadOptions,
    config: Arc<ConfigStore>,
    strata: Arc<Strata>,
    planner: Planner<StrataQueue, Hot>,
    egress: Arc<EgressGw>,
    paid: Arc<PaidGw>,
    proxy: Mutex<String>,
    push: accessors_builtin::PushAccessor,
    browser_program: Mutex<PathBuf>,
}

impl Services {
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
            StrataQueue(Arc::clone(&strata)),
            Hot::new(Arc::clone(&strata), "planner"),
        );
        let egress = Arc::new(egress::Gateway::new(
            Hot::new(Arc::clone(&strata), "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let paid = Arc::new(paid::Gateway::new(
            Hot::new(Arc::clone(&strata), "paid"),
            PaidBridge {
                egress: Arc::clone(&egress),
                fallback: opening,
            },
            transport::call_policy_from(&snapshot.system, &Value::Null),
        ));
        let browser = std::env::var("MODALPRISM_MOLI").unwrap_or_else(|_| "moli".into());
        Ok(Arc::new(Self {
            data_dir: normalize(data),
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

    pub fn set_browser_program(&self, program: impl Into<PathBuf>) {
        *self.browser_program.lock().expect("浏览器") = program.into();
    }

    pub async fn shutdown(&self) -> Result<(), Error> {
        self.planner.shutdown(Timestamp::now()).await?;
        Ok(())
    }

    pub(crate) fn fetch_state(&self, timeout: Duration) -> FetchState {
        FetchState {
            snapshot: self.config.applied(),
            egress: Arc::clone(&self.egress),
            paid: Arc::clone(&self.paid),
            staging_dir: self.data_dir.join("staging"),
            proxy: self.proxy_url(),
            browser: self.browser_program.lock().expect("浏览器").clone(),
            timeout,
        }
    }

    pub(crate) fn access_context(&self) -> accessors::AccessContext {
        accessors::AccessContext {
            egress: self.egress.clone(),
            paid: self.paid.clone(),
            sessions: Arc::new(NoSession),
            staging: Arc::new(DiskStaging {
                dir: self.data_dir.join("staging"),
            }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub(crate) fn push(&self) -> &accessors_builtin::PushAccessor {
        &self.push
    }

    pub(crate) async fn commit_inbound(
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
        let state = self.fetch_state(Duration::from_secs(30));
        let ops = execute::inbound_ops(&state, source_id, &module, envelopes)
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

    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Error> {
        dispatch(self, method, args).await
    }
}

struct TaskHandler {
    state: FetchState,
    strata: Arc<Strata>,
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
        let fetched = execute::fetch(&self.state, task, cursor, resource).await?;
        let mut success = fetched.success;
        success.follows = open_follows(&self.strata, fetched.follows, fetched.max_fetches).await;
        Ok(success)
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

async fn dispatch(services: &Services, method: &str, args: Value) -> Result<Value, Error> {
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

async fn strata_kinds(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_has(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_get(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_slice(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_resolve(services: &Services, args: &Value) -> Result<Value, Error> {
    let identity = text_field(args, "identity")?;
    let rid = services.strata.resolve(&identity).await?;
    Ok(match rid {
        Some(rid) => Value::String(rid.to_string()),
        None => Value::Null,
    })
}

async fn strata_history(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_lineage(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_related(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_query(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_changes(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_apply(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_enqueue(services: &Services, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        tasks: Vec<strata_model::AccessTask>,
    }
    let args: Args = parse(args)?;
    services.strata.enqueue(&args.tasks).await?;
    Ok(json!({}))
}

async fn strata_lease(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_extend(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn strata_nack(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn committer_flush(services: &Services) -> Result<Value, Error> {
    let snapshot = services.strata.flush().await?;
    Ok(json!({ "snapshot": snapshot }))
}

fn status_paths(services: &Services) -> Value {
    json!({
        "control": services.data_dir.join("control.db"),
        "lance": services.data_dir.join("lance"),
        "staging": services.data_dir.join("staging"),
    })
}

async fn committer_maintain(services: &Services) -> Result<Value, Error> {
    services.strata.recover().await?;
    Ok(status_paths(services))
}

async fn committer_recover(services: &Services) -> Result<Value, Error> {
    services.strata.recover().await?;
    Ok(json!({}))
}

async fn planner_tick(services: &Services) -> Result<Value, Error> {
    let plans = plans::plans_of(&services.config.applied());
    let tasks = services
        .planner
        .tick(&plans, &capacities(), Timestamp::now())
        .await?;
    serde_json::to_value(tasks).map_err(|source| Error::with_source("spec 无效", source))
}

async fn planner_trigger(services: &Services, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    services.planner.make_due(&source, Timestamp::now()).await?;
    planner_tick(services).await
}

async fn planner_next(services: &Services, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    let status = services.planner.source_status(&source).await?;
    Ok(json!({
        "nextAt": status.next_at,
        "intervalMs": status.interval_ms,
        "healthStatus": status.health_status,
        "alert": status.alert,
        "skipReason": status.skip_reason,
        "budgetUsed": status.budget_used,
    }))
}

async fn runner_run(services: &Services, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        task: strata_model::AccessTask,
    }
    let args: Args = parse(args)?;
    let snapshot = services.config.applied();
    let plans = plans::plans_of(&snapshot);
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
        state: services.fetch_state(plan.timeout),
        strata: services.strata(),
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

async fn health_evaluate(services: &Services) -> Result<Value, Error> {
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

async fn health_status(services: &Services, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    let status = services.planner.source_status(&source).await?;
    Ok(json!({
        "key": format!("source/{source}"),
        "status": status.health_status,
        "alert": status.alert,
    }))
}

fn generation_of(snapshot: &Snapshot) -> Value {
    json!({ "generation": snapshot.generation })
}

async fn config_load(services: &Services) -> Result<Value, Error> {
    services.config.reload()?;
    Ok(generation_of(&services.config.snapshot()))
}

async fn config_validate(services: &Services) -> Result<Value, Error> {
    config::validate(&services.config_dir, &services.load_options)?;
    Ok(json!({ "ok": true }))
}

async fn config_diff(services: &Services) -> Result<Value, Error> {
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

async fn config_explain(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn egress_check(services: &Services, args: &Value) -> Result<Value, Error> {
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

async fn paid_complete(services: &Services, args: &Value) -> Result<Value, Error> {
    let receipt = text_field(args, "receiptId")?;
    services
        .paid
        .complete(&receipt)
        .await
        .map_err(|error| Error::with_source(error.message.clone(), error))?;
    Ok(json!({}))
}

async fn paid_status(services: &Services, args: &Value) -> Result<Value, Error> {
    let receipt = text_field(args, "receiptId")?;
    let status = services
        .paid
        .status(&receipt)
        .await
        .map_err(|error| Error::with_source(error.message.clone(), error))?;
    Ok(json!({ "status": status }))
}

async fn accessor_call(
    services: &Services,
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
    services: &Services,
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
    let request = accessors::AccessRequest {
        task_id: strata_model::TaskId::new(args.task_id).map_err(|_| Error::new("spec 无效"))?,
        source_id: strata_model::SourceId::new(args.source_id)
            .map_err(|_| Error::new("spec 无效"))?,
        locator: args.locator,
        spec: args.spec,
        cursor: args.cursor,
        hints: accessors::AccessHints::default(),
        policy: Arc::new(
            args.policy
                .unwrap_or_else(|| Value::Object(Default::default())),
        ),
        deadline: std::time::Instant::now() + Duration::from_secs(30),
    };
    let envelopes = accessor
        .access(request, &services.access_context())
        .await
        .map_err(|error| Error::new(error.message))?;
    serde_json::to_value(envelopes).map_err(|source| Error::with_source("spec 无效", source))
}

fn strata_options(system: &Value) -> Options {
    let mut options = Options::default();
    if let Some(size) = system
        .pointer("/strata/blob/inline_size_threshold")
        .and_then(Value::as_str)
        .and_then(parse_size)
    {
        options.blob_inline_threshold = size;
    }
    if let Some(size) = system
        .pointer("/strata/blob/dedicated_size_threshold")
        .and_then(Value::as_str)
        .and_then(parse_size)
    {
        options.blob_dedicated_threshold = size;
    }
    if let Some(ttl) = system
        .pointer("/strata/staging/ttl")
        .and_then(Value::as_str)
        .and_then(parse_duration)
    {
        options.staging_ttl = ttl;
    }
    options
}

fn parse_size(text: &str) -> Option<usize> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    usize::try_from(count.saturating_mul(scale)).ok()
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(Duration::from_millis(count.saturating_mul(scale)))
}

fn refuse_remote(snapshot: &Snapshot, data: &Path) -> Result<(), Error> {
    let system = &snapshot.system;
    if system
        .pointer("/strata/lance/uri")
        .and_then(Value::as_str)
        .is_some_and(|uri| !uri.is_empty())
    {
        return Err(Error::new("存储位置未规定"));
    }
    if system
        .pointer("/strata/control_store/backend")
        .and_then(Value::as_str)
        == Some("postgres")
    {
        return Err(Error::new("存储位置未规定"));
    }
    if let Some(path) = system
        .pointer("/strata/control_store/path")
        .and_then(Value::as_str)
    {
        if normalize(Path::new(path)) != normalize(&data.join("control.db")) {
            return Err(Error::new("存储位置未规定"));
        }
    }
    if let Some(path) = system
        .pointer("/strata/staging/dir")
        .and_then(Value::as_str)
    {
        if normalize(Path::new(path)) != normalize(&data.join("staging")) {
            return Err(Error::new("存储位置未规定"));
        }
    }
    Ok(())
}

fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
