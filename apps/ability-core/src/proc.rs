//! 子进程 Ability 源。协议在 `ability_support::remote`，客端在 ability-sdk。

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use ability_support::remote::{
    Body, CtxOp, Frame, WireError, decode_frame, encode_frame, ok_value,
};
use ability_support::{
    Ability, AbilityCtx, AbilityId, AbilityManifest, AbilityProvider, BusEvent, Error, EventFilter,
    EventHandler, EventMiddleware, EventMode, EventNext, EventOpts, EventPipeline, EventReg,
    EventSink, InstanceId, Lifecycle, MethodId, MethodMeta, Result,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// 子进程监督：登记 PID，批量回收。
pub struct ProcSupervisor;

impl ProcSupervisor {
    pub fn register(pid: u32) {
        supervisor_pids().lock().expect("supervisor").push(pid);
    }

    pub fn unregister(pid: u32) {
        supervisor_pids()
            .lock()
            .expect("supervisor")
            .retain(|p| *p != pid);
    }

    pub fn shutdown_all() {
        let pids: Vec<u32> = supervisor_pids()
            .lock()
            .expect("supervisor")
            .drain(..)
            .collect();
        for pid in pids {
            #[cfg(unix)]
            {
                use std::process::Command;
                let _ = Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status();
            }
            #[cfg(windows)]
            {
                // Windows 无统一 SIGTERM；依赖 ProcAbility Drop 路径。
                let _ = pid;
            }
        }
        std::thread::sleep(SHUTDOWN_GRACE);
    }
}

fn supervisor_pids() -> &'static StdMutex<Vec<u32>> {
    static PIDS: StdMutex<Vec<u32>> = StdMutex::new(Vec::new());
    &PIDS
}

struct Bridge {
    out: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<String, oneshot::Sender<Frame>>>,
    ctxs: Mutex<HashMap<String, Arc<dyn AbilityCtx>>>,
    sub_tasks: Mutex<Vec<JoinHandle<()>>>,
    handle_regs: Mutex<HashMap<u64, EventReg>>,
    pending_next: Mutex<HashMap<String, EventNext>>,
}

impl Bridge {
    async fn send(&self, frame: &Frame) -> Result<()> {
        let line = encode_frame(frame)?;
        self.out
            .send(line)
            .map_err(|_| Error::failed("远程 stdin 已关闭"))
    }

    fn shutdown_sync(&self) {
        if let Ok(line) = encode_frame(&Frame::request(Body::Shutdown)) {
            let _ = self.out.send(line);
        }
    }

    async fn request_frame(&self, body: Body) -> Result<Frame> {
        let frame = Frame::request(body);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(frame.id.clone(), tx);
        self.send(&frame).await?;
        tokio::time::timeout(HANDSHAKE_TIMEOUT, rx)
            .await
            .map_err(|_| Error::failed("远程 Ability 应答超时"))?
            .map_err(|_| Error::failed("远程 Ability 已退出"))
    }

    async fn request(&self, body: Body) -> Result<Value> {
        ok_value(self.request_frame(body).await?)
    }

    async fn call_with_ctx(&self, ctx: Arc<dyn AbilityCtx>, body: Body) -> Result<Value> {
        let frame = Frame::request(body);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(frame.id.clone(), tx);
        self.ctxs.lock().await.insert(frame.id.clone(), ctx);
        if let Err(e) = self.send(&frame).await {
            self.pending.lock().await.remove(&frame.id);
            self.ctxs.lock().await.remove(&frame.id);
            return Err(e);
        }
        let reply = tokio::time::timeout(HANDSHAKE_TIMEOUT, rx)
            .await
            .map_err(|_| Error::failed("远程 Ability 应答超时"))?
            .map_err(|_| Error::failed("远程 Ability 已退出"));
        self.ctxs.lock().await.remove(&frame.id);
        ok_value(reply?)
    }
}

async fn pump_incoming(mut rx: mpsc::UnboundedReceiver<String>, bridge: Arc<Bridge>) {
    while let Some(line) = rx.recv().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(frame) = decode_frame(&line) else {
            continue;
        };
        if let Some(to) = frame.reply_to.clone() {
            if let Some(tx) = bridge.pending.lock().await.remove(&to) {
                let _ = tx.send(frame);
                continue;
            }
        }
        if let Body::Ctx { ctx_id, op } = frame.body {
            let cmd_id = frame.id.clone();
            let bridge = Arc::clone(&bridge);
            tokio::spawn(async move {
                let reply = match dispatch_ctx(Arc::clone(&bridge), &ctx_id, op).await {
                    Ok(value) => Body::Ok { value },
                    Err(e) => Body::Err {
                        error: WireError::from(&e),
                    },
                };
                let _ = bridge.send(&Frame::reply(&cmd_id, reply)).await;
            });
        }
    }
    let mut pending = bridge.pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(Frame::reply(
            "",
            Body::Err {
                error: WireError {
                    kind: "failed".into(),
                    message: "远程进程已退出".into(),
                },
            },
        ));
    }
}

async fn dispatch_ctx(bridge: Arc<Bridge>, ctx_id: &str, op: CtxOp) -> Result<Value> {
    let ctx = bridge
        .ctxs
        .lock()
        .await
        .get(ctx_id)
        .cloned()
        .ok_or_else(|| Error::not_found(format!("远程 ctx {ctx_id}")))?;
    match op {
        CtxOp::Call {
            id_or_alias,
            method,
            args,
        } => {
            let method = MethodId::new(method).map_err(|e| Error::invalid_args(e.to_string()))?;
            ctx.call(&id_or_alias, &method, args).await
        }
        CtxOp::Dep { id_or_alias } => {
            let id = ctx.dep(&id_or_alias)?;
            Ok(Value::String(id.to_string()))
        }
        CtxOp::Catalog => {
            serde_json::to_value(ctx.catalog()).map_err(|e| Error::failed(e.to_string()))
        }
        CtxOp::LocalGet { key } => match ctx.local_get(&key).await? {
            Some(value) => Ok(json!({ "hit": true, "value": value })),
            None => Ok(json!({ "hit": false })),
        },
        CtxOp::LocalSet { key, value } => {
            ctx.local_set(&key, value).await?;
            Ok(Value::Null)
        }
        CtxOp::Emit { name, payload } => {
            ctx.emit(&name, payload).await?;
            Ok(Value::Null)
        }
        CtxOp::Load { spec } => {
            let instance = ctx.load(spec).await?;
            Ok(json!({ "instance": instance.to_string() }))
        }
        CtxOp::Unload { instance } => {
            let id = InstanceId::new(instance).map_err(|e| Error::invalid_args(e.to_string()))?;
            ctx.unload(&id).await?;
            Ok(Value::Null)
        }
        CtxOp::Subscribe { filter } => subscribe_ctx(bridge, ctx, filter).await,
        CtxOp::Handle {
            filter,
            prepend,
            mode,
            guest_id,
        } => {
            let handler = remote_handler(Arc::clone(&bridge), guest_id, mode);
            let reg = ctx.handle(filter, handler, EventOpts { prepend }).await?;
            let id = reg.id();
            bridge.handle_regs.lock().await.insert(id, reg);
            Ok(json!({ "id": id }))
        }
        CtxOp::Unhandle { id } => {
            bridge.handle_regs.lock().await.remove(&id);
            Ok(Value::Null)
        }
        CtxOp::EventNext { token, event } => {
            let next = bridge
                .pending_next
                .lock()
                .await
                .remove(&token)
                .ok_or_else(|| Error::failed("事件 next 已失效"))?;
            let out = next.run(event).await?;
            serde_json::to_value(&out).map_err(|e| Error::failed(e.to_string()))
        }
        CtxOp::Share => {
            let shared = ctx.share();
            let id = Uuid::new_v4().to_string();
            bridge.ctxs.lock().await.insert(id.clone(), shared);
            Ok(json!({ "ctx_id": id }))
        }
        CtxOp::Feedback { record } => {
            ability_support::feedback::host::accept(record);
            Ok(Value::Null)
        }
        CtxOp::ChainStatus { chain_id } => {
            let st = ctx.chain_status(chain_id.as_deref());
            serde_json::to_value(st).map_err(|e| Error::failed(e.to_string()))
        }
    }
}

async fn subscribe_ctx(
    bridge: Arc<Bridge>,
    ctx: Arc<dyn AbilityCtx>,
    filter: EventFilter,
) -> Result<Value> {
    let mut listener = ctx.subscribe(filter).await?;
    let sub = Uuid::new_v4().to_string();
    let sub_id = sub.clone();
    let send_bridge = Arc::clone(&bridge);
    let handle = tokio::spawn(async move {
        while let Some(event) = listener.next().await {
            let frame = Frame::request(Body::Event {
                sub: sub_id.clone(),
                event,
            });
            if send_bridge.send(&frame).await.is_err() {
                break;
            }
        }
    });
    bridge.sub_tasks.lock().await.push(handle);
    Ok(json!({ "sub": sub }))
}

struct RemoteMw {
    bridge: Arc<Bridge>,
    guest_id: String,
}

struct RemotePipe {
    bridge: Arc<Bridge>,
    guest_id: String,
}

struct RemoteSink {
    bridge: Arc<Bridge>,
    guest_id: String,
}

fn remote_handler(bridge: Arc<Bridge>, guest_id: String, mode: EventMode) -> EventHandler {
    match mode {
        EventMode::Middleware => EventHandler::Middleware(Arc::new(RemoteMw { bridge, guest_id })),
        EventMode::Pipeline => EventHandler::Pipeline(Arc::new(RemotePipe { bridge, guest_id })),
        EventMode::Parallel => EventHandler::Parallel(Arc::new(RemoteSink { bridge, guest_id })),
        EventMode::Message => EventHandler::Message(Arc::new(RemoteSink { bridge, guest_id })),
    }
}

async fn run_guest_handle(
    bridge: &Bridge,
    guest_id: &str,
    event: BusEvent,
    next_token: Option<String>,
) -> Result<Value> {
    bridge
        .request(Body::RunHandle {
            guest_id: guest_id.to_string(),
            event,
            next_token,
        })
        .await
}

#[async_trait]
impl EventMiddleware for RemoteMw {
    async fn around(&self, event: BusEvent, next: EventNext) -> Result<BusEvent> {
        let token = Uuid::new_v4().to_string();
        self.bridge
            .pending_next
            .lock()
            .await
            .insert(token.clone(), next);
        let reply =
            run_guest_handle(&self.bridge, &self.guest_id, event, Some(token.clone())).await;
        self.bridge.pending_next.lock().await.remove(&token);
        let v = reply?;
        serde_json::from_value(v).map_err(|e| Error::failed(e.to_string()))
    }
}

#[async_trait]
impl EventPipeline for RemotePipe {
    async fn process(&self, event: BusEvent) -> Result<BusEvent> {
        let v = run_guest_handle(&self.bridge, &self.guest_id, event, None).await?;
        serde_json::from_value(v).map_err(|e| Error::failed(e.to_string()))
    }
}

#[async_trait]
impl EventSink for RemoteSink {
    async fn on_event(&self, event: &BusEvent) {
        let _ = run_guest_handle(&self.bridge, &self.guest_id, event.clone(), None).await;
    }
}

/// 握手后的远程 Ability。Drop 时 graceful Shutdown 后回收子进程。
pub struct ProcAbility {
    manifest: AbilityManifest,
    bridge: Arc<Bridge>,
    child: StdMutex<Option<std::process::Child>>,
    pump: JoinHandle<()>,
}

impl Drop for ProcAbility {
    fn drop(&mut self) {
        self.bridge.shutdown_sync();
        std::thread::sleep(SHUTDOWN_GRACE);
        self.pump.abort();
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut child) = g.take() {
                let pid = child.id();
                ProcSupervisor::unregister(pid);
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Ok(mut tasks) = self.bridge.sub_tasks.try_lock() {
            for t in tasks.drain(..) {
                t.abort();
            }
        }
    }
}

#[async_trait]
impl Ability for ProcAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        let keep = matches!(phase, Lifecycle::Started);
        let body = Body::Lifecycle {
            phase,
            instance: ctx.instance().to_string(),
            chain_id: ctx.chain_id().to_string(),
            span: ctx.span(),
        };
        let frame = Frame::request(body);
        let (tx, rx) = oneshot::channel();
        self.bridge
            .pending
            .lock()
            .await
            .insert(frame.id.clone(), tx);
        self.bridge
            .ctxs
            .lock()
            .await
            .insert(frame.id.clone(), ctx.retain());
        self.bridge.send(&frame).await?;
        let reply = tokio::time::timeout(HANDSHAKE_TIMEOUT, rx)
            .await
            .map_err(|_| Error::failed("远程生命周期应答超时"))?
            .map_err(|_| Error::failed("远程 Ability 已退出"))?;
        if !keep {
            self.bridge.ctxs.lock().await.remove(&frame.id);
        }
        ok_value(reply).map(|_| ())
    }

    async fn invoke(&self, method: &MethodId, ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        let body = Body::Invoke {
            method: method.to_string(),
            args,
            instance: ctx.instance().to_string(),
            chain_id: ctx.chain_id().to_string(),
            span: ctx.span(),
        };
        self.bridge.call_with_ctx(ctx.retain(), body).await
    }
}

async fn finish_handshake(
    bridge: Arc<Bridge>,
    pump: JoinHandle<()>,
    config: Value,
    child: Option<std::process::Child>,
) -> Result<Arc<dyn Ability>> {
    let hello = bridge.request_frame(Body::Hello).await?;
    let manifest = match hello.body {
        Body::Manifest { manifest } => manifest,
        Body::Err { error } => return Err(error.into_error()),
        other => {
            return Err(Error::failed(format!(
                "远程 Hello 应答不是 Manifest: {other:?}"
            )));
        }
    };
    bridge.request(Body::Create { config }).await?;
    if let Some(ref c) = child {
        ProcSupervisor::register(c.id());
    }
    Ok(Arc::new(ProcAbility {
        manifest,
        bridge,
        child: StdMutex::new(child),
        pump,
    }))
}

fn new_bridge(out: mpsc::UnboundedSender<String>) -> Arc<Bridge> {
    Arc::new(Bridge {
        out,
        pending: Mutex::new(HashMap::new()),
        ctxs: Mutex::new(HashMap::new()),
        sub_tasks: Mutex::new(Vec::new()),
        handle_regs: Mutex::new(HashMap::new()),
        pending_next: Mutex::new(HashMap::new()),
    })
}

async fn handshake<R, W>(reader: R, writer: W, config: Value) -> Result<Arc<dyn Ability>>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut writer = writer;
        while let Some(line) = out_rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if writer.flush().await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if in_tx.send(line).is_err() {
                break;
            }
        }
    });
    let bridge = new_bridge(out_tx);
    let pump = tokio::spawn(pump_incoming(in_rx, Arc::clone(&bridge)));
    finish_handshake(bridge, pump, config, None).await
}

/// 用已有读写器握手（测试用 duplex，与子进程同一协议）。
pub async fn connect<R, W>(reader: R, writer: W, config: Value) -> Result<Arc<dyn Ability>>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    handshake(reader, writer, config).await
}

/// 已安装的子进程源。每次 `create` 拉起新进程。
pub struct ProcProvider {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
    cached: StdMutex<Option<AbilityManifest>>,
}

impl ProcProvider {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            cached: StdMutex::new(None),
        }
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, val: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_os_string(), val.as_ref().to_os_string()));
        self
    }
}

pub(crate) fn unbound_guest_manifest() -> AbilityManifest {
    AbilityManifest {
        id: AbilityId::new("remote.unbound").expect("id"),
        depends: vec![],
        stateful: false,
        config_schema: Value::Null,
        methods: vec![MethodMeta {
            id: MethodId::new("unbound").expect("m"),
            description: String::new(),
            input_schema: Value::Null,
            output_schema: None,
        }],
        events: vec![],
    }
}

#[async_trait]
impl AbilityProvider for ProcProvider {
    fn manifest(&self) -> AbilityManifest {
        self.cached
            .lock()
            .expect("cached")
            .clone()
            .unwrap_or_else(unbound_guest_manifest)
    }

    async fn create(&self, config: Value) -> Result<Arc<dyn Ability>> {
        use std::io::{BufRead, Write};

        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn().map_err(|e| Error::io(e.to_string()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::failed("子进程 stdin 不可用"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::failed("子进程 stdout 不可用"))?;
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
        std::thread::spawn(move || {
            while let Some(line) = out_rx.blocking_recv() {
                if stdin.write_all(line.as_bytes()).is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut r = std::io::BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if in_tx.send(line.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let bridge = new_bridge(out_tx);
        let pump = tokio::spawn(pump_incoming(in_rx, Arc::clone(&bridge)));
        let ability = finish_handshake(bridge, pump, config, Some(child)).await?;
        *self.cached.lock().expect("cached") = Some(ability.manifest().clone());
        Ok(ability)
    }
}
