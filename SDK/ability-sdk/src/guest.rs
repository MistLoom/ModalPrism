//! 远程 Ability 客端：把本进程 Provider 接到 NDJSON 传输上。

use std::collections::HashMap;
use std::io::{self, BufRead, Write as IoWrite};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll};

use ability_support::remote::{
    Body, CtxOp, Frame, WireError, decode_frame, encode_frame, ok_value,
};
use ability_support::{
    Ability, AbilityCtx, AbilityProvider, Aspect, BusEvent, ChainStatus, DiscoveryPayload, Effect,
    Error, EventFilter, EventHandler, EventListener, EventNext, EventOpts, EventReg, EventUnreg,
    FeedbackRecord, FeedbackScope, InstanceId, LoadSpec, MethodId, Result, Undo,
};
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

struct GuestSlot {
    handler: EventHandler,
    ctx_id: String,
}

impl Clone for GuestSlot {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            ctx_id: self.ctx_id.clone(),
        }
    }
}

struct Bridge {
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Frame>>>,
    subs: Mutex<HashMap<String, mpsc::UnboundedSender<BusEvent>>>,
    handlers: StdMutex<HashMap<String, GuestSlot>>,
}

impl Bridge {
    async fn send(&self, frame: &Frame) -> Result<()> {
        let line = encode_frame(frame)?;
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| Error::io(e.to_string()))?;
        w.flush().await.map_err(|e| Error::io(e.to_string()))
    }

    async fn request(&self, body: Body) -> Result<Value> {
        let frame = Frame::request(body);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(frame.id.clone(), tx);
        self.send(&frame).await?;
        let reply = rx.await.map_err(|_| Error::failed("远程宿主已断开"))?;
        ok_value(reply)
    }
}

#[derive(Clone)]
struct GuestFwd {
    bridge: Arc<Bridge>,
    ctx_id: String,
}

tokio::task_local! {
    static GUEST_SCOPE: FeedbackScope;
    static GUEST_FWD: GuestFwd;
}

fn guest_scope() -> FeedbackScope {
    GUEST_SCOPE
        .try_with(|s| s.clone())
        .unwrap_or_else(|_| FeedbackScope::detached())
}

fn bind_guest_feedback() {
    ability_support::feedback::host::install_scope(guest_scope);
    ability_support::feedback::host::forward(Arc::new(|record: FeedbackRecord| {
        if let Ok(fwd) = GUEST_FWD.try_with(|f| f.clone()) {
            // 与 Node / Python 一样：invoke 返回前宿主已经收下这条记录。
            block_on_ctx(async move {
                let _ = fwd
                    .bridge
                    .request(Body::Ctx {
                        ctx_id: fwd.ctx_id,
                        op: CtxOp::Feedback { record },
                    })
                    .await;
            });
        }
    }));
}

/// 契约上同步的 ctx 方法在客端需等待宿主；不得在当前 worker 直接 `block_on`。
fn block_on_ctx<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

struct GuestListener {
    rx: mpsc::UnboundedReceiver<BusEvent>,
}

struct GuestUnreg {
    bridge: Arc<Bridge>,
    ctx_id: String,
    guest_id: String,
}

impl EventUnreg for GuestUnreg {
    fn unregister(&self, id: u64) {
        self.bridge
            .handlers
            .lock()
            .expect("handlers")
            .remove(&self.guest_id);
        let bridge = Arc::clone(&self.bridge);
        let ctx_id = self.ctx_id.clone();
        tokio::spawn(async move {
            let _ = bridge
                .request(Body::Ctx {
                    ctx_id,
                    op: CtxOp::Unhandle { id },
                })
                .await;
        });
    }
}

#[async_trait]
impl EventListener for GuestListener {
    async fn next(&mut self) -> Option<BusEvent> {
        self.rx.recv().await
    }
}

struct GuestCtx {
    bridge: Arc<Bridge>,
    ctx_id: String,
    instance: InstanceId,
    method: Option<MethodId>,
    chain_id: String,
    span: u64,
    undos: Arc<Mutex<Vec<Undo>>>,
}

impl GuestCtx {
    async fn ctx(&self, op: CtxOp) -> Result<Value> {
        self.bridge
            .request(Body::Ctx {
                ctx_id: self.ctx_id.clone(),
                op,
            })
            .await
    }

    async fn run_undos(&self, origin: Result<Value>) -> Result<Value> {
        let mut undos = self.undos.lock().await;
        let mut acc = origin;
        while let Some(u) = undos.pop() {
            if let Err(e) = u.run().await {
                acc = match acc {
                    Ok(_) => Err(Error::failed(format!("Undo 失败: {e}")).with_undo_error(e)),
                    Err(orig) => Err(orig.with_undo_error(e)),
                };
            }
        }
        acc
    }
}

#[async_trait]
impl AbilityCtx for GuestCtx {
    fn share(&self) -> Arc<dyn AbilityCtx> {
        let new_id = block_on_ctx(self.ctx(CtxOp::Share))
            .ok()
            .and_then(|v| v.get("ctx_id")?.as_str().map(str::to_string))
            .unwrap_or_else(|| self.ctx_id.clone());
        Arc::new(Self {
            bridge: Arc::clone(&self.bridge),
            ctx_id: new_id,
            instance: self.instance.clone(),
            method: None,
            chain_id: self.chain_id.clone(),
            span: self.span,
            undos: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn retain(&self) -> Arc<dyn AbilityCtx> {
        Arc::new(Self {
            bridge: Arc::clone(&self.bridge),
            ctx_id: self.ctx_id.clone(),
            instance: self.instance.clone(),
            method: self.method.clone(),
            chain_id: self.chain_id.clone(),
            span: self.span,
            undos: Arc::clone(&self.undos),
        })
    }

    fn instance(&self) -> &InstanceId {
        &self.instance
    }

    fn method(&self) -> Option<&MethodId> {
        self.method.as_ref()
    }

    fn chain_id(&self) -> &str {
        &self.chain_id
    }

    fn span(&self) -> u64 {
        self.span
    }

    fn chain_status(&self, chain_id: Option<&str>) -> Option<ChainStatus> {
        let v = block_on_ctx(self.ctx(CtxOp::ChainStatus {
            chain_id: chain_id.map(str::to_string),
        }))
        .ok()?;
        serde_json::from_value(v).ok()?
    }

    async fn call(&self, id_or_alias: &str, method: &MethodId, args: Value) -> Result<Value> {
        self.ctx(CtxOp::Call {
            id_or_alias: id_or_alias.to_string(),
            method: method.to_string(),
            args,
        })
        .await
    }

    fn dep(&self, id_or_alias: &str) -> Result<InstanceId> {
        let v = block_on_ctx(self.ctx(CtxOp::Dep {
            id_or_alias: id_or_alias.to_string(),
        }))?;
        let s = v
            .as_str()
            .ok_or_else(|| Error::failed("dep 返回非字符串"))?;
        InstanceId::new(s).map_err(|e| Error::invalid_args(e.to_string()))
    }

    fn catalog(&self) -> Vec<DiscoveryPayload> {
        match block_on_ctx(self.ctx(CtxOp::Catalog)) {
            Ok(v) => serde_json::from_value(v).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<Box<dyn EventListener + Send>> {
        let v = self.ctx(CtxOp::Subscribe { filter }).await?;
        let sub = v
            .get("sub")
            .and_then(|x| x.as_str())
            .ok_or_else(|| Error::failed("Subscribe 未返回 sub"))?
            .to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.bridge.subs.lock().await.insert(sub, tx);
        Ok(Box::new(GuestListener { rx }))
    }

    async fn handle(
        &self,
        filter: EventFilter,
        handler: EventHandler,
        opts: EventOpts,
    ) -> Result<EventReg> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let guest_id = format!("h{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let mode = handler.mode();
        self.bridge.handlers.lock().expect("handlers").insert(
            guest_id.clone(),
            GuestSlot {
                handler,
                ctx_id: self.ctx_id.clone(),
            },
        );
        let v = self
            .ctx(CtxOp::Handle {
                filter,
                prepend: opts.prepend,
                mode,
                guest_id: guest_id.clone(),
            })
            .await?;
        let id = v
            .get("id")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| Error::failed("Handle 未返回 id"))?;
        Ok(EventReg::new(
            id,
            Arc::new(GuestUnreg {
                bridge: Arc::clone(&self.bridge),
                ctx_id: self.ctx_id.clone(),
                guest_id,
            }),
        ))
    }

    async fn load(&self, spec: LoadSpec) -> Result<InstanceId> {
        let v = self.ctx(CtxOp::Load { spec }).await?;
        let s = v
            .get("instance")
            .and_then(|x| x.as_str())
            .ok_or_else(|| Error::failed("load 未返回 instance"))?;
        InstanceId::new(s).map_err(|e| Error::invalid_args(e.to_string()))
    }

    async fn unload(&self, instance: &InstanceId) -> Result<()> {
        self.ctx(CtxOp::Unload {
            instance: instance.to_string(),
        })
        .await?;
        Ok(())
    }

    async fn local_get(&self, key: &str) -> Result<Option<Value>> {
        let v = self
            .ctx(CtxOp::LocalGet {
                key: key.to_string(),
            })
            .await?;
        if v.get("hit").and_then(|x| x.as_bool()) == Some(true) {
            Ok(v.get("value").cloned())
        } else {
            Ok(None)
        }
    }

    async fn local_set(&self, key: &str, value: Value) -> Result<()> {
        self.ctx(CtxOp::LocalSet {
            key: key.to_string(),
            value,
        })
        .await?;
        Ok(())
    }

    async fn push_effect(&self, effect: Box<dyn Effect>) -> Result<()> {
        let undo = effect.apply(self).await?;
        self.undos.lock().await.push(undo);
        Ok(())
    }

    async fn emit(&self, name: &str, payload: Value) -> Result<()> {
        self.ctx(CtxOp::Emit {
            name: name.to_string(),
            payload,
        })
        .await?;
        Ok(())
    }

    fn register_aspect(&self, _aspect: Arc<dyn Aspect>) -> Result<()> {
        Err(Error::failed("Guest 不支持 register_aspect"))
    }
}

/// 在 NDJSON 读写器上托管一个 Provider，直到 Shutdown 或对端关闭。
pub async fn serve<R, W>(provider: Arc<dyn AbilityProvider>, reader: R, writer: W) -> Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let bridge = Arc::new(Bridge {
        writer: Mutex::new(Box::new(writer)),
        pending: Mutex::new(HashMap::new()),
        subs: Mutex::new(HashMap::new()),
        handlers: StdMutex::new(HashMap::new()),
    });
    run_loop(provider, bridge, rx).await
}

struct StdoutSink;

impl AsyncWrite for StdoutSink {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(IoWrite::write(&mut io::stdout(), buf))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(IoWrite::flush(&mut io::stdout()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// 标准输入输出：stdin 用阻塞线程读，避免 Windows 上 tokio stdin 管道挂死。
pub async fn serve_stdio(provider: Arc<dyn AbilityProvider>) -> Result<()> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("ability-guest-stdin".into())
        .spawn(move || {
            let stdin = io::stdin();
            let mut r = stdin.lock();
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(line.clone()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .map_err(|e| Error::io(e.to_string()))?;
    let bridge = Arc::new(Bridge {
        writer: Mutex::new(Box::new(StdoutSink)),
        pending: Mutex::new(HashMap::new()),
        subs: Mutex::new(HashMap::new()),
        handlers: StdMutex::new(HashMap::new()),
    });
    run_loop(provider, bridge, rx).await
}

async fn run_loop(
    provider: Arc<dyn AbilityProvider>,
    bridge: Arc<Bridge>,
    mut rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    bind_guest_feedback();
    let ability_slot: Arc<Mutex<Option<Arc<dyn Ability>>>> = Arc::new(Mutex::new(None));
    // 命令帧（invoke / lifecycle）顺序执行。读循环不持有该锁，应答与 run_handle 仍可重入。
    let command_gate = Arc::new(Mutex::new(()));
    while let Some(line) = rx.recv().await {
        if line.trim().is_empty() {
            continue;
        }
        let frame = decode_frame(&line)?;
        if let Some(to) = frame.reply_to.clone() {
            if let Some(tx) = bridge.pending.lock().await.remove(&to) {
                let _ = tx.send(frame);
                continue;
            }
        }
        match frame.body {
            Body::Event { sub, event } => {
                if let Some(tx) = bridge.subs.lock().await.get(&sub) {
                    let _ = tx.send(event);
                }
            }
            Body::RunHandle {
                guest_id,
                event,
                next_token,
            } => {
                let cmd_id = frame.id.clone();
                let bridge = Arc::clone(&bridge);
                tokio::spawn(async move {
                    let slot = bridge
                        .handlers
                        .lock()
                        .expect("handlers")
                        .get(&guest_id)
                        .cloned();
                    let reply = match slot {
                        Some(slot) => {
                            match run_guest_slot(&bridge, slot, event, next_token).await {
                                Ok(value) => Body::Ok { value },
                                Err(e) => Body::Err {
                                    error: WireError::from(&e),
                                },
                            }
                        }
                        None => Body::Err {
                            error: WireError {
                                kind: "not_found".into(),
                                message: "事件处理者已卸".into(),
                            },
                        },
                    };
                    let _ = bridge.send(&Frame::reply(&cmd_id, reply)).await;
                });
            }
            Body::Shutdown => break,
            Body::Hello => {
                let reply = Frame::reply(
                    &frame.id,
                    Body::Manifest {
                        manifest: provider.manifest(),
                    },
                );
                bridge.send(&reply).await?;
            }
            Body::Create { config } => {
                let id = frame.id.clone();
                match provider.create(config).await {
                    Ok(a) => {
                        *ability_slot.lock().await = Some(a);
                        bridge
                            .send(&Frame::reply(&id, Body::Ok { value: Value::Null }))
                            .await?;
                    }
                    Err(e) => {
                        bridge
                            .send(&Frame::reply(
                                &id,
                                Body::Err {
                                    error: WireError::from(&e),
                                },
                            ))
                            .await?;
                    }
                }
            }
            Body::Invoke {
                method,
                args,
                instance,
                chain_id,
                span,
            } => {
                let ability = ability_slot.lock().await.clone();
                let bridge = Arc::clone(&bridge);
                let command_gate = Arc::clone(&command_gate);
                let cmd_id = frame.id.clone();
                tokio::spawn(async move {
                    let _turn = command_gate.lock().await;
                    let reply = {
                        if ability.is_none() {
                            Body::Err {
                                error: WireError {
                                    kind: "failed".into(),
                                    message: "尚未 Create".into(),
                                },
                            }
                        } else if let (Ok(instance), Ok(mid)) =
                            (InstanceId::new(&instance), MethodId::new(&method))
                        {
                            let ctx = GuestCtx {
                                bridge: Arc::clone(&bridge),
                                ctx_id: cmd_id.clone(),
                                instance,
                                method: Some(mid.clone()),
                                chain_id: chain_id.clone(),
                                span,
                                undos: Arc::new(Mutex::new(Vec::new())),
                            };
                            let scope = FeedbackScope { chain_id, span };
                            let fwd = GuestFwd {
                                bridge: Arc::clone(&bridge),
                                ctx_id: cmd_id.clone(),
                            };
                            let result = GUEST_SCOPE
                                .scope(
                                    scope,
                                    GUEST_FWD.scope(fwd, ability.unwrap().invoke(&mid, &ctx, args)),
                                )
                                .await;
                            match ctx.run_undos(result).await {
                                Ok(value) => Body::Ok { value },
                                Err(e) => Body::Err {
                                    error: WireError::from(&e),
                                },
                            }
                        } else {
                            Body::Err {
                                error: WireError {
                                    kind: "invalid_input".into(),
                                    message: "非法 instance 或 method".into(),
                                },
                            }
                        }
                    };
                    let _ = bridge.send(&Frame::reply(&cmd_id, reply)).await;
                });
            }
            Body::Lifecycle {
                phase,
                instance,
                chain_id,
                span,
            } => {
                let ability = ability_slot.lock().await.clone();
                let bridge = Arc::clone(&bridge);
                let command_gate = Arc::clone(&command_gate);
                let cmd_id = frame.id.clone();
                tokio::spawn(async move {
                    let _turn = command_gate.lock().await;
                    let reply = match (ability, InstanceId::new(&instance)) {
                        (Some(ability), Ok(instance)) => {
                            let ctx = GuestCtx {
                                bridge: Arc::clone(&bridge),
                                ctx_id: cmd_id.clone(),
                                instance,
                                method: None,
                                chain_id: chain_id.clone(),
                                span,
                                undos: Arc::new(Mutex::new(Vec::new())),
                            };
                            let scope = FeedbackScope { chain_id, span };
                            let fwd = GuestFwd {
                                bridge: Arc::clone(&bridge),
                                ctx_id: cmd_id.clone(),
                            };
                            match GUEST_SCOPE
                                .scope(
                                    scope,
                                    GUEST_FWD.scope(fwd, ability.on_lifecycle(phase, &ctx)),
                                )
                                .await
                            {
                                Ok(()) => Body::Ok { value: Value::Null },
                                Err(e) => Body::Err {
                                    error: WireError::from(&e),
                                },
                            }
                        }
                        (None, _) => Body::Err {
                            error: WireError {
                                kind: "failed".into(),
                                message: "尚未 Create".into(),
                            },
                        },
                        (_, Err(e)) => Body::Err {
                            error: WireError {
                                kind: "invalid_input".into(),
                                message: e.to_string(),
                            },
                        },
                    };
                    let _ = bridge.send(&Frame::reply(&cmd_id, reply)).await;
                });
            }
            _ => {}
        }
    }
    Ok(())
}

async fn run_guest_slot(
    bridge: &Arc<Bridge>,
    slot: GuestSlot,
    event: BusEvent,
    next_token: Option<String>,
) -> Result<Value> {
    match slot.handler {
        EventHandler::Message(s) | EventHandler::Parallel(s) => {
            s.on_event(&event).await;
            serde_json::to_value(&event).map_err(|e| Error::failed(e.to_string()))
        }
        EventHandler::Pipeline(p) => {
            let ev = p.process(event).await?;
            serde_json::to_value(&ev).map_err(|e| Error::failed(e.to_string()))
        }
        EventHandler::Middleware(m) => {
            let ctx_id = slot.ctx_id;
            let bridge = Arc::clone(bridge);
            let next = EventNext::new(move |ev| {
                let bridge = Arc::clone(&bridge);
                let ctx_id = ctx_id.clone();
                let token = next_token.clone();
                Box::pin(async move {
                    let Some(token) = token else {
                        return Ok(ev);
                    };
                    let v = bridge
                        .request(Body::Ctx {
                            ctx_id,
                            op: CtxOp::EventNext { token, event: ev },
                        })
                        .await?;
                    serde_json::from_value(v).map_err(|e| Error::failed(e.to_string()))
                })
            });
            let ev = m.around(event, next).await?;
            serde_json::to_value(&ev).map_err(|e| Error::failed(e.to_string()))
        }
    }
}
