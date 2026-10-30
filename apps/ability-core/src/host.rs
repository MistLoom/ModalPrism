//! AbilityHost：可空启动，只认 Provider。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};

use ability_support::{
    Ability, AbilityEvent, AbilityId, AbilityProvider, Aspect, CatalogEntry, ChainStatus,
    DiscoveryPayload, Error, EventDir, EventFilter, EventMode, EventUnreg, FeedbackScope,
    HostEvent, InstanceId, InvokeMask, Lifecycle, LoadSpec, LocalsScope, MethodId, ProtocolKind,
    Result, SourceId, UserEvent, host_event, method_catalog,
};
use async_trait::async_trait;
use frame_core::{AbilityExpose, AccessKind, Gateway, InvokeRequest, InvokeResponse};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tracing::debug;
use uuid::Uuid;

use crate::chain::run_chain;
use crate::context::InvokeContext;
use crate::event_bus::{EventBus, now_ms};
use crate::scope::{bind_scope, with_scope};

/// 在途调用计数。unload 在 Stopping 后等到 0。
struct Inflight {
    count: AtomicUsize,
    idle: Notify,
}

impl Inflight {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            count: AtomicUsize::new(0),
            idle: Notify::new(),
        })
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.count.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }
}

struct InflightGuard {
    inner: Arc<Inflight>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if self.inner.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.inner.idle.notify_waiters();
        }
    }
}

pub(crate) struct InstanceSlot {
    pub ability: Arc<dyn Ability>,
    pub phase: Lifecycle,
    pub bindings: HashMap<String, InstanceId>,
    inflight: Arc<Inflight>,
}

/// 能力对应用的归属与开放标记。
#[derive(Clone, Debug)]
pub struct AbilityGrant {
    pub app: String,
    pub open: bool,
}

/// 一次 `load` 失败的调用方可见记录（分类码 + 原始信息）。
#[derive(Clone, Debug)]
pub struct LoadFailure {
    pub source: String,
    pub instance: String,
    /// 错误分类码（`toolchain_missing` / `artifact_missing` / `handshake_failed` / …）。
    pub code: &'static str,
    pub message: String,
}

/// Guest/实例就绪探针结果。
///
/// 契约：**装载成功 ≠ 可 invoke**。只有 [`Readiness::Ready`] 表示实例已通过
/// 装载（含 Guest 握手与生命周期回调）且处于就绪阶段；调用方（如对外入口的
/// 注册逻辑）应在暴露依赖该实例的能力前先探测就绪。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// 实例已装载且生命周期就绪，可 invoke。
    Ready,
    /// 未装载或装载已回滚（未 install / load 失败 / 已 unload）。
    Absent,
    /// 最近一次装载失败；携带错误分类码与原始信息。
    Failed { code: &'static str, message: String },
}

pub(crate) struct HostInner {
    pub sources: StdRwLock<HashMap<SourceId, Arc<dyn AbilityProvider>>>,
    pub instances: StdRwLock<HashMap<InstanceId, InstanceSlot>>,
    /// 最近一次装载失败记录（按实例去重，成功装载即清除）。
    pub load_failures: StdMutex<HashMap<String, LoadFailure>>,
    pub events: EventBus,
    /// ability id / instance id → 归属。
    pub grants: StdRwLock<HashMap<String, AbilityGrant>>,
    pub entry_app: StdRwLock<Option<String>>,
    pub expose: StdRwLock<AbilityExpose>,
    pub access_gate: StdRwLock<Option<AccessKind>>,
    /// 实例 → 已声明支持的协议（来自能力目录）。
    pub protocol_support: StdRwLock<HashMap<String, Vec<AccessKind>>>,
    pub aspects: StdRwLock<Vec<Arc<dyn Aspect>>>,
    pub locals: StdRwLock<HashMap<LocalsScope, HashMap<String, Value>>>,
    pub chains: StdRwLock<HashMap<String, ChainStatus>>,
    pub event_ids: StdMutex<HashMap<InstanceId, Vec<u64>>>,
    load_lock: Mutex<()>,
}

impl HostInner {
    fn new() -> Self {
        Self {
            sources: StdRwLock::new(HashMap::new()),
            instances: StdRwLock::new(HashMap::new()),
            load_failures: StdMutex::new(HashMap::new()),
            events: EventBus::new(),
            grants: StdRwLock::new(HashMap::new()),
            entry_app: StdRwLock::new(None),
            expose: StdRwLock::new(AbilityExpose::open_all()),
            access_gate: StdRwLock::new(None),
            protocol_support: StdRwLock::new(HashMap::new()),
            aspects: StdRwLock::new(Vec::new()),
            locals: StdRwLock::new(HashMap::new()),
            chains: StdRwLock::new(HashMap::new()),
            event_ids: StdMutex::new(HashMap::new()),
            load_lock: Mutex::new(()),
        }
    }

    fn copy_grant(&self, ability_id: &str, instance_id: &str) {
        let mut grants = self.grants.write().expect("grants");
        if let Some(grant) = grants.get(ability_id).cloned() {
            grants.insert(instance_id.to_string(), grant);
        }
    }

    fn grant(&self, id: &str) -> Option<AbilityGrant> {
        self.grants.read().expect("grants").get(id).cloned()
    }

    fn entry_app(&self) -> Option<String> {
        self.entry_app.read().expect("entry_app").clone()
    }

    pub(crate) fn authorize_port(&self, target: &str) -> Result<()> {
        let Some(grant) = self.grant(target) else {
            return Ok(());
        };
        if grant.open {
            return Ok(());
        }
        match self.entry_app() {
            Some(app) if app == grant.app => Ok(()),
            _ => Err(Error::unavailable(format!(
                "能力 {target} 归属应用 {}，未开放",
                grant.app
            ))),
        }
    }

    pub(crate) fn authorize_call(&self, caller: &str, target: &str) -> Result<()> {
        let Some(target_grant) = self.grant(target) else {
            return Ok(());
        };
        if target_grant.open {
            return Ok(());
        }
        let caller_app = self
            .grant(caller)
            .map(|g| g.app)
            .or_else(|| self.entry_app());
        match caller_app {
            Some(app) if app == target_grant.app => Ok(()),
            _ => Err(Error::unavailable(format!(
                "能力 {target} 归属应用 {}，未开放",
                target_grant.app
            ))),
        }
    }

    pub(crate) fn authorize_event(
        &self,
        caller: &str,
        filter: &EventFilter,
        mode: EventMode,
    ) -> Result<()> {
        let Some(caller_grant) = self.grant(caller) else {
            return Ok(());
        };
        let Some(inst) = filter.instance.as_ref() else {
            if matches!(mode, EventMode::Middleware | EventMode::Pipeline) {
                return Err(Error::unavailable("拦截须指定本应用的能力实例"));
            }
            return Ok(());
        };
        let Some(target) = self.grant(inst.as_str()) else {
            return Ok(());
        };
        let intercept = matches!(mode, EventMode::Middleware | EventMode::Pipeline);
        if target.app == "common" || caller_grant.app == target.app {
            return Ok(());
        }
        if !intercept && target.open {
            return Ok(());
        }
        let verb = if intercept { "拦截" } else { "监听" };
        Err(Error::unavailable(format!(
            "应用 {} 不能{verb}能力 {}",
            caller_grant.app, inst
        )))
    }

    pub fn lookup_chain(&self, chain_id: &str) -> Option<ChainStatus> {
        self.chains.read().expect("chains").get(chain_id).cloned()
    }

    pub(crate) fn bind_event_id(&self, instance: &InstanceId, id: u64) {
        self.event_ids
            .lock()
            .expect("event_ids")
            .entry(instance.clone())
            .or_default()
            .push(id);
    }

    pub(crate) fn drop_event_ids(&self, instance: &InstanceId) {
        let ids = self
            .event_ids
            .lock()
            .expect("event_ids")
            .remove(instance)
            .unwrap_or_default();
        for id in ids {
            self.events.unregister(id);
        }
    }

    fn allows_protocol(&self, instance: &str, kind: AccessKind) -> bool {
        let map = self.protocol_support.read().expect("protocol_support");
        match map.get(instance) {
            Some(kinds) => kinds.contains(&kind),
            None => false,
        }
    }

    pub(crate) fn instance_supports_protocol(&self, instance: &str, kind: ProtocolKind) -> bool {
        let Some(access) = AccessKind::from_protocol(kind) else {
            return false;
        };
        self.allows_protocol(instance, access)
    }

    pub fn instances_snapshot(&self) -> Vec<DiscoveryPayload> {
        let map = self.instances.read().expect("instances");
        let mut out: Vec<DiscoveryPayload> = map
            .iter()
            .filter(|(_, s)| s.phase == Lifecycle::Ready)
            .map(|(inst, slot)| {
                DiscoveryPayload::from_manifest(inst.clone(), slot.phase, slot.ability.manifest())
            })
            .collect();
        out.sort_by(|a, b| a.instance.as_str().cmp(b.instance.as_str()));
        out
    }

    pub fn catalog_snapshot(&self) -> Vec<CatalogEntry> {
        method_catalog(&self.instances_snapshot())
    }

    pub async fn local_get(&self, scope: &LocalsScope, key: &str) -> Result<Option<Value>> {
        let guard = self.locals.read().expect("locals");
        Ok(guard.get(scope).and_then(|m| m.get(key).cloned()))
    }

    pub async fn local_set(&self, scope: &LocalsScope, key: &str, value: Value) -> Result<()> {
        let mut guard = self.locals.write().expect("locals");
        guard
            .entry(scope.clone())
            .or_default()
            .insert(key.to_string(), value);
        Ok(())
    }

    pub async fn emit_ability(
        &self,
        instance: &InstanceId,
        name: &str,
        payload: Value,
    ) -> Result<()> {
        let event_decl = {
            let map = self.instances.read().expect("instances");
            let slot = map
                .get(instance)
                .ok_or_else(|| Error::not_found(instance))?;
            slot.ability
                .manifest()
                .events
                .iter()
                .find(|e| e.dir == EventDir::Emit && e.name == name)
                .cloned()
        };
        let Some(decl) = event_decl else {
            return Err(Error::failed(format!("能力事件未声明为 emit: {name}")));
        };
        if let Err(e) = ability_support::validate_schema(&payload, &decl.payload_schema) {
            return Err(Error::failed(format!(
                "能力事件 {instance}.{name} 载荷不符合 payload_schema: {e}"
            )));
        }
        self.events
            .emit_ability(AbilityEvent {
                instance: instance.clone(),
                name: name.to_string(),
                payload,
                at: now_ms(),
            })
            .await;
        Ok(())
    }

    fn discovery_of(&self, instance: &InstanceId) -> Option<DiscoveryPayload> {
        let map = self.instances.read().expect("instances");
        let slot = map.get(instance)?;
        Some(DiscoveryPayload::from_manifest(
            instance.clone(),
            slot.phase,
            slot.ability.manifest(),
        ))
    }

    fn stateful_taken(
        &self,
        ability_id: &AbilityId,
        instances: &HashMap<InstanceId, InstanceSlot>,
    ) -> bool {
        instances
            .values()
            .any(|s| s.ability.manifest().stateful && s.ability.manifest().id == *ability_id)
    }

    fn bindings_cycle(
        new_id: &InstanceId,
        new_bindings: &HashMap<String, InstanceId>,
        instances: &HashMap<InstanceId, InstanceSlot>,
    ) -> bool {
        fn neighbors<'a>(
            cur: &'a InstanceId,
            new_id: &'a InstanceId,
            new_bindings: &'a HashMap<String, InstanceId>,
            instances: &'a HashMap<InstanceId, InstanceSlot>,
        ) -> Vec<&'a InstanceId> {
            if cur == new_id {
                new_bindings.values().collect()
            } else {
                instances
                    .get(cur)
                    .map(|s| s.bindings.values().collect())
                    .unwrap_or_default()
            }
        }

        let mut stack = vec![new_id];
        let mut seen = std::collections::HashSet::new();
        seen.insert(new_id.clone());
        while let Some(cur) = stack.pop() {
            for n in neighbors(cur, new_id, new_bindings, instances) {
                if n == new_id {
                    return true;
                }
                if seen.insert(n.clone()) {
                    stack.push(n);
                }
            }
        }
        false
    }

    #[allow(unused_variables)] // used inside monitor_emit!; empty when feature off
    async fn emit_lifecycle(
        &self,
        instance: &InstanceId,
        from: Option<Lifecycle>,
        phase: Lifecycle,
        ability_id: Option<&str>,
    ) {
        let mut discovery = self.discovery_of(instance);
        if let Some(ref mut d) = discovery {
            d.phase = phase;
        }
        ability_support::monitor_emit!({
            let mut ev = ability_support::monitor::MonitorEvent::new("lifecycle", now_ms())
                .phase(phase.as_str())
                .instance(instance.as_str())
                .to_state(phase.as_str());
            if let Some(f) = from {
                ev = ev.from_state(f.as_str());
            }
            if let Some(aid) = ability_id {
                ev = ev.ability_id(aid);
            }
            ev
        });
        self.events
            .emit_host(HostEvent {
                name: host_event::lifecycle(phase.as_str()),
                at: now_ms(),
                discovery,
                detail: Value::Null,
            })
            .await;
    }

    fn ability_id_of(&self, instance: &InstanceId) -> Option<String> {
        self.instances
            .read()
            .expect("instances")
            .get(instance)
            .map(|s| s.ability.manifest().id.to_string())
    }

    fn phase_of(&self, instance: &InstanceId) -> Option<Lifecycle> {
        self.instances
            .read()
            .expect("instances")
            .get(instance)
            .map(|s| s.phase)
    }

    async fn transition_lifecycle(
        &self,
        instance: &InstanceId,
        to: Lifecycle,
    ) {
        let from = self.phase_of(instance);
        let aid = self.ability_id_of(instance);
        self.set_phase(instance, to);
        self.emit_lifecycle(instance, from, to, aid.as_deref()).await;
    }


    fn resolve_bindings(
        &self,
        manifest_id: &AbilityId,
        depends: &[ability_support::Dependency],
        spec_bindings: Option<&std::collections::BTreeMap<String, InstanceId>>,
        instances: &HashMap<InstanceId, InstanceSlot>,
    ) -> Result<HashMap<String, InstanceId>> {
        let mut out = HashMap::new();
        for dep in depends {
            let key = dep.key();
            if let Some(spec) = spec_bindings.and_then(|b| b.get(&key)) {
                let slot = instances
                    .get(spec)
                    .ok_or_else(|| Error::failed(format!("绑定目标实例不存在: {spec}")))?;
                if slot.phase != Lifecycle::Ready {
                    return Err(Error::failed(format!(
                        "绑定目标实例未就绪: {spec} ({:?})",
                        slot.phase
                    )));
                }
                if slot.ability.manifest().id != dep.id {
                    return Err(Error::failed(format!(
                        "绑定 {} 的能力 id 与依赖 {} 不符",
                        spec, dep.id
                    )));
                }
                out.insert(key, spec.clone());
                continue;
            }
            let matches: Vec<&InstanceId> = instances
                .iter()
                .filter(|(_, s)| s.phase == Lifecycle::Ready && s.ability.manifest().id == dep.id)
                .map(|(id, _)| id)
                .collect();
            match matches.len() {
                0 => {
                    if dep.optional {
                        continue;
                    }
                    return Err(Error::failed(format!(
                        "能力 {} 缺少依赖 {}",
                        manifest_id, dep.id
                    )));
                }
                1 => {
                    out.insert(key, matches[0].clone());
                }
                _ => {
                    return Err(Error::failed(format!(
                        "依赖 {} 存在多个实例，必须显式绑定",
                        dep.id
                    )));
                }
            }
        }
        Ok(out)
    }

    fn set_phase(&self, id: &InstanceId, phase: Lifecycle) {
        if let Some(slot) = self.instances.write().expect("instances").get_mut(id) {
            slot.phase = phase;
        }
    }

    fn make_ctx(
        self: &Arc<Self>,
        instance: InstanceId,
        method: Option<MethodId>,
        bindings: HashMap<String, InstanceId>,
        locals_scope: LocalsScope,
        chain_id: String,
        span: u64,
        span_id: String,
        port_path: bool,
        invoke_mask: InvokeMask,
    ) -> InvokeContext {
        InvokeContext {
            host: Arc::clone(self),
            instance,
            method,
            chain_id,
            span,
            span_id,
            locals_scope,
            bindings,
            undos: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            port_path,
            invoke_mask,
            protocol_filter: None,
        }
    }

    pub async fn install(self: &Arc<Self>, source: SourceId, provider: Arc<dyn AbilityProvider>) {
        self.sources
            .write()
            .expect("sources")
            .insert(source, provider);
    }

    pub async fn load(self: &Arc<Self>, spec: LoadSpec) -> Result<InstanceId> {
        let _guard = self.load_lock.lock().await;
        if self
            .instances
            .read()
            .expect("instances")
            .contains_key(&spec.as_name)
        {
            return Err(Error::failed(format!("实例已存在: {}", spec.as_name)));
        }
        let provider = self
            .sources
            .read()
            .expect("sources")
            .get(&spec.source)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("未 install 的 source: {}", spec.source)))?;

        let declared = provider.manifest();
        if declared.methods.is_empty() {
            return Err(Error::failed("Ability 至少需要一个 method"));
        }
        if declared.stateful {
            let map = self.instances.read().expect("instances");
            if self.stateful_taken(&declared.id, &map) {
                return Err(Error::failed(format!(
                    "有状态能力 {} 仅允许单实例",
                    declared.id
                )));
            }
        }

        let ability = provider.create(spec.config.clone()).await?;
        let manifest = ability.manifest().clone();
        if manifest.id != declared.id || manifest.stateful != declared.stateful {
            return Err(Error::failed(format!(
                "create 产出的 Manifest 与 Provider 声明不一致: {} ",
                spec.source
            )));
        }
        if manifest.methods.is_empty() {
            return Err(Error::failed("Ability 至少需要一个 method"));
        }

        let bindings = {
            let map = self.instances.read().expect("instances");
            let bindings = self.resolve_bindings(
                &manifest.id,
                &manifest.depends,
                spec.bindings.as_ref(),
                &map,
            )?;
            if Self::bindings_cycle(&spec.as_name, &bindings, &map) {
                return Err(Error::failed(format!("能力 {} 依赖成环", manifest.id)));
            }
            bindings
        };

        {
            let mut map = self.instances.write().expect("instances");
            map.insert(
                spec.as_name.clone(),
                InstanceSlot {
                    ability: Arc::clone(&ability),
                    phase: Lifecycle::Loaded,
                    bindings: bindings.clone(),
                    inflight: Inflight::new(),
                },
            );
        }
        self.copy_grant(manifest.id.as_str(), spec.as_name.as_str());
        self.emit_lifecycle(&spec.as_name, None, Lifecycle::Loaded, self.ability_id_of(&spec.as_name).as_deref()).await;

        self.transition_lifecycle(&spec.as_name, Lifecycle::Bound).await;

        let ctx = self.make_ctx(
            spec.as_name.clone(),
            None,
            bindings.clone(),
            LocalsScope::new(),
            Uuid::new_v4().to_string(),
            0, Uuid::new_v4().to_string(),
            false,
            InvokeMask::default(),
        );
        let life_scope = FeedbackScope {
            chain_id: ctx.chain_id.clone(),
            span: ctx.span,
        };
        if let Err(e) = with_scope(life_scope, ability.on_lifecycle(Lifecycle::Started, &ctx)).await
        {
            self.transition_lifecycle(&spec.as_name, Lifecycle::Failed).await;
            self.instances
                .write()
                .expect("instances")
                .remove(&spec.as_name);
            return Err(e);
        }
        self.transition_lifecycle(&spec.as_name, Lifecycle::Started).await;

        self.transition_lifecycle(&spec.as_name, Lifecycle::Ready).await;
        ability_support::monitor_emit!(
            ability_support::monitor::MonitorEvent::new("host", now_ms())
                .phase("load")
                .instance(spec.as_name.as_str())
                .ability_id(ability.manifest().id.as_str())
                .from_state("absent")
                .to_state("loaded")
        );
        self.events
            .emit_host(HostEvent {
                name: host_event::LOAD.into(),
                at: now_ms(),
                discovery: self.discovery_of(&spec.as_name),
                detail: Value::Null,
            })
            .await;

        Ok(spec.as_name)
    }

    pub async fn unload(self: &Arc<Self>, instance: &InstanceId) -> Result<()> {
        let _guard = self.load_lock.lock().await;
        let (ability, bindings, inflight, from_phase) = {
            let mut map = self.instances.write().expect("instances");
            let slot = map
                .get_mut(instance)
                .ok_or_else(|| Error::not_found(instance))?;
            if slot.phase != Lifecycle::Ready && slot.phase != Lifecycle::Started {
                return Err(Error::failed(format!(
                    "instance {} phase {:?} cannot unload",
                    instance, slot.phase
                )));
            }
            let from_phase = slot.phase;
            slot.phase = Lifecycle::Stopping;
            (
                Arc::clone(&slot.ability),
                slot.bindings.clone(),
                Arc::clone(&slot.inflight),
                from_phase,
            )
        };
        self.emit_lifecycle(
            instance,
            Some(from_phase),
            Lifecycle::Stopping,
            Some(ability.manifest().id.as_str()),
        )
        .await;
        inflight.wait_idle().await;

        let ctx = self.make_ctx(
            instance.clone(),
            None,
            bindings,
            LocalsScope::new(),
            Uuid::new_v4().to_string(),
            0, Uuid::new_v4().to_string(),
            false,
            InvokeMask::default(),
        );
        let life_scope = FeedbackScope {
            chain_id: ctx.chain_id.clone(),
            span: ctx.span,
        };
        let _ = with_scope(life_scope, ability.on_lifecycle(Lifecycle::Stopped, &ctx)).await;
        self.transition_lifecycle(instance, Lifecycle::Stopped).await;
        self.drop_event_ids(instance);

        self.instances.write().expect("instances").remove(instance);
        ability_support::monitor_emit!(
            ability_support::monitor::MonitorEvent::new("host", now_ms())
                .phase("unload")
                .instance(instance.as_str())
                .ability_id(ability.manifest().id.as_str())
                .from_state("loaded")
                .to_state("unloaded")
        );
        self.events
            .emit_host(HostEvent {
                name: host_event::UNLOAD.into(),
                at: now_ms(),
                discovery: Some(DiscoveryPayload::from_manifest(
                    instance.clone(),
                    Lifecycle::Unloaded,
                    ability.manifest(),
                )),
                detail: Value::Null,
            })
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke_internal(
        self: &Arc<Self>,
        instance: &InstanceId,
        method: &MethodId,
        args: Value,
        locals_scope: LocalsScope,
        chain_id: String,
        span: u64,
        parent_span_id: Option<String>,
        port_path: bool,
        invoke_mask: InvokeMask,
    ) -> Result<InvokeResponse> {
        let (ability, bindings, _guard) = {
            let mut map = self.instances.write().expect("instances");
            let slot = map
                .get_mut(instance)
                .ok_or_else(|| Error::not_found(instance))?;
            if slot.phase != Lifecycle::Ready {
                return Err(Error::unavailable(format!(
                    "实例 {} 未就绪 ({:?})",
                    instance, slot.phase
                )));
            }
            if slot.ability.manifest().method(method).is_none() {
                return Err(Error::not_found(format!("{instance}.{method}")));
            }
            slot.inflight.count.fetch_add(1, Ordering::SeqCst);
            (
                Arc::clone(&slot.ability),
                slot.bindings.clone(),
                InflightGuard {
                    inner: Arc::clone(&slot.inflight),
                },
            )
        };

        let discovery =
            DiscoveryPayload::from_manifest(instance.clone(), Lifecycle::Ready, ability.manifest());
        let ability_id = ability.manifest().id.to_string();
        let span_id = Uuid::new_v4().to_string();
        let started = now_ms();
        ability_support::monitor_emit!({
            let mut before = ability_support::monitor::MonitorEvent::new("invoke", started)
                .phase("before")
                .instance(instance.as_str())
                .method(method.as_str())
                .chain(&chain_id, span)
                .span_id(&span_id)
                .ability_id(&ability_id)
                .started_at(started);
            if let Some(ref p) = parent_span_id {
                before = before.parent_span_id(p);
            }
            before
        });
        self.events
            .emit_host(HostEvent {
                name: host_event::INVOKE_BEFORE.into(),
                at: started,
                discovery: Some(discovery.clone()),
                detail: serde_json::json!({
                    "method": method.as_str(),
                    "chain_id": chain_id,
                    "span": span,
                    "span_id": span_id,
                    "parent_span_id": parent_span_id,
                    "ability_id": ability_id,
                    "args": args.clone(),
                }),
            })
            .await;

        let ctx = self.make_ctx(
            instance.clone(),
            Some(method.clone()),
            bindings,
            locals_scope.clone(),
            chain_id.clone(),
            span,
            span_id.clone(),
            port_path,
            invoke_mask,
        );
        {
            let mut g = self.chains.write().expect("chains");
            // Root only: register chain; nested calls share the chain (C1).
            if span == 0 {
                g.insert(
                    chain_id.clone(),
                    ChainStatus {
                        chain_id: chain_id.clone(),
                        instance: instance.to_string(),
                        method: Some(method.to_string()),
                        span,
                        running: true,
                        started_at: Some(started),
                        span_id: Some(span_id.clone()),
                        parent_span_id: parent_span_id.clone(),
                        ability_id: Some(ability_id.clone()),
                    },
                );
            }
        }
let aspects = self.aspects.read().expect("aspects").clone();
        let scope = FeedbackScope {
            chain_id: chain_id.clone(),
            span,
        };
        let result = with_scope(scope, async {
            let result = run_chain(&aspects, &ctx, ability, method.clone(), args).await;
            ctx.run_undos(result).await
        })
        .await;
        // 仅根（span==0）返回时拆链；嵌套子调用返回不得拆掉父链（C1）。
        if span == 0 {
            self.chains.write().expect("chains").remove(&chain_id);
        }

        match &result {
            Ok(_) => {
                let ended = now_ms();
                ability_support::monitor_emit!({
                    let mut after = ability_support::monitor::MonitorEvent::new("invoke", ended)
                        .phase("after")
                        .instance(instance.as_str())
                        .method(method.as_str())
                        .chain(&chain_id, span)
                        .span_id(&span_id)
                        .ability_id(&ability_id)
                        .started_at(started)
                        .ended_at(ended);
                    if let Some(ref p) = parent_span_id {
                        after = after.parent_span_id(p);
                    }
                    after
                });
                self.events
                    .emit_host(HostEvent {
                        name: host_event::INVOKE_AFTER.into(),
                        at: ended,
                        discovery: Some(discovery),
                        detail: serde_json::json!({
                            "method": method.as_str(),
                            "chain_id": chain_id,
                            "span": span,
                            "span_id": span_id,
                            "parent_span_id": parent_span_id,
                            "ability_id": ability_id,
                            "started_at": started,
                            "ended_at": ended,
                        }),
                    })
                    .await;
            }
            Err(e) => {
                let ended = now_ms();
                ability_support::monitor_emit!({
                    let mut err_ev = ability_support::monitor::MonitorEvent::new("invoke", ended)
                        .phase("error")
                        .instance(instance.as_str())
                        .method(method.as_str())
                        .chain(&chain_id, span)
                        .span_id(&span_id)
                        .ability_id(&ability_id)
                        .started_at(started)
                        .ended_at(ended)
                        .error(e.to_string());
                    if let Some(ref p) = parent_span_id {
                        err_ev = err_ev.parent_span_id(p);
                    }
                    err_ev
                });
                self.events
                    .emit_host(HostEvent {
                        name: host_event::INVOKE_ERROR.into(),
                        at: ended,
                        discovery: Some(discovery),
                        detail: serde_json::json!({
                            "method": method.as_str(),
                            "chain_id": chain_id,
                            "span": span,
                            "span_id": span_id,
                            "parent_span_id": parent_span_id,
                            "ability_id": ability_id,
                            "started_at": started,
                            "ended_at": ended,
                            "error": e.to_string()
                        }),
                    })
                    .await;
            }
        }

        let data = result?;
        let _ = ctx.port_path;
        Ok(InvokeResponse::new(data).with_locals(locals_scope))
    }
}

/// 运行宿主。
#[derive(Clone)]
pub struct AbilityHost {
    inner: Arc<HostInner>,
}

impl Default for AbilityHost {
    fn default() -> Self {
        Self::new()
    }
}

impl AbilityHost {
    pub fn new() -> Self {
        bind_scope();
        Self {
            inner: Arc::new(HostInner::new()),
        }
    }

    pub async fn install(&self, source: SourceId, provider: Arc<dyn AbilityProvider>) {
        self.inner.install(source, provider).await;
    }

    pub async fn install_str(
        &self,
        source: &str,
        provider: Arc<dyn AbilityProvider>,
    ) -> Result<()> {
        let id = SourceId::new(source).map_err(|e| Error::invalid_args(e.to_string()))?;
        self.install(id, provider).await;
        Ok(())
    }

    pub async fn load(&self, spec: LoadSpec) -> Result<InstanceId> {
        let result = self.inner.load(spec.clone()).await;
        let instance = spec.as_name.to_string();
        let source = spec.source.to_string();
        match &result {
            Ok(_) => {
                self.inner
                    .load_failures
                    .lock()
                    .expect("load_failures")
                    .remove(&instance);
            }
            Err(e) => {
                self.inner
                    .load_failures
                    .lock()
                    .expect("load_failures")
                    .insert(
                        instance.clone(),
                        LoadFailure {
                            source,
                            instance,
                            code: e.code(),
                            message: e.to_string(),
                        },
                    );
            }
        }
        result
    }

    pub async fn unload(&self, instance: &InstanceId) -> Result<()> {
        self.inner.unload(instance).await
    }

    /// 就绪探针：装载成功 ≠ 可 invoke。
    ///
    /// - `Ready`：实例在册且生命周期处于就绪阶段，可 invoke；
    /// - `Failed`：最近一次装载失败（含分类码与原始信息，如缺工具链 / 缺预构建产物 / 握手失败）；
    /// - `Absent`：未装载或装载已回滚。
    pub fn readiness(&self, instance: &InstanceId) -> Readiness {
        let slot = self
            .inner
            .instances
            .read()
            .expect("instances")
            .get(instance)
            .map(|s| s.phase);
        let failure = self
            .inner
            .load_failures
            .lock()
            .expect("load_failures")
            .get(instance.as_str())
            .cloned();
        match slot {
            Some(Lifecycle::Ready) => Readiness::Ready,
            Some(Lifecycle::Failed) => failure
                .map(|f| Readiness::Failed {
                    code: f.code,
                    message: f.message,
                })
                .unwrap_or(Readiness::Failed {
                    code: "failed",
                    message: "实例生命周期失败".into(),
                }),
            _ => failure
                .map(|f| Readiness::Failed {
                    code: f.code,
                    message: f.message,
                })
                .unwrap_or(Readiness::Absent),
        }
    }

    /// 最近一次装载失败记录（按实例去重）。
    pub fn load_failures(&self) -> Vec<LoadFailure> {
        self.inner
            .load_failures
            .lock()
            .expect("load_failures")
            .values()
            .cloned()
            .collect()
    }

    /// 清空装载失败记录。
    pub fn clear_load_failures(&self) {
        self.inner
            .load_failures
            .lock()
            .expect("load_failures")
            .clear();
    }

    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner.catalog_snapshot()
    }

    /// 已装实例摘要（含可注册事件声明）。
    pub fn instances_snapshot(&self) -> Vec<DiscoveryPayload> {
        self.inner.instances_snapshot()
    }

    pub fn set_expose(&self, expose: AbilityExpose) {
        *self.inner.expose.write().expect("expose") = expose;
    }

    pub fn set_access_gate(&self, kind: Option<AccessKind>) {
        *self.inner.access_gate.write().expect("gate") = kind;
    }

    /// 登记实例支持的协议（由组合根在 load 前注入）。
    pub fn set_entry_app(&self, app: impl Into<String>) {
        *self.inner.entry_app.write().expect("entry_app") = Some(app.into());
    }

    pub fn entry_app(&self) -> Option<String> {
        self.inner.entry_app.read().expect("entry_app").clone()
    }

    pub fn set_ability_grant(&self, id: &str, grant: AbilityGrant) {
        self.inner
            .grants
            .write()
            .expect("grants")
            .insert(id.to_string(), grant);
    }

    pub fn grant(&self, id: &str) -> Option<AbilityGrant> {
        self.inner.grants.read().expect("grants").get(id).cloned()
    }

    /// 端口调用：未登记归属则放行；已登记时，属主或 `open` 放行。
    pub fn authorize_port(&self, target: &str) -> Result<()> {
        self.inner.authorize_port(target)
    }

    /// 能力间调用：目标未登记则放行；否则须同应用或目标 `open`。
    pub fn authorize_call(&self, caller: &str, target: &str) -> Result<()> {
        self.inner.authorize_call(caller, target)
    }

    /// 事件控制：拦截仅属主；监听允许属主，或目标 `open`。
    pub fn authorize_event(
        &self,
        caller: &str,
        filter: &EventFilter,
        mode: EventMode,
    ) -> Result<()> {
        self.inner.authorize_event(caller, filter, mode)
    }

    /// 读取某能力实例登记的协议集合（只读列举，供一致性校验）。
    pub fn protocol_support(&self, instance: &str) -> Vec<AccessKind> {
        self.inner
            .protocol_support
            .read()
            .expect("protocol_support")
            .get(instance)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_protocol_support(&self, instance: &str, kinds: Vec<AccessKind>) {
        self.inner
            .protocol_support
            .write()
            .expect("protocol_support")
            .insert(instance.to_string(), kinds);
    }

    fn allows_protocol(&self, instance: &str, kind: AccessKind) -> bool {
        self.inner.allows_protocol(instance, kind)
    }

    pub fn add_aspect(&self, aspect: Arc<dyn Aspect>) {
        self.inner.aspects.write().expect("aspects").push(aspect);
    }

    pub fn events(&self) -> &EventBus {
        &self.inner.events
    }

    /// 在途调用链快照。
    pub fn chains_snapshot(&self) -> Vec<ChainStatus> {
        self.inner
            .chains
            .read()
            .expect("chains")
            .values()
            .cloned()
            .collect()
    }

    pub async fn emit_user(&self, mut event: UserEvent) {
        if event.at == 0 {
            event.at = now_ms();
        }
        self.inner.events.emit_user(event).await;
    }

    pub fn into_gateway(self) -> Arc<dyn Gateway> {
        Arc::new(self)
    }
}

#[async_trait]
impl Gateway for AbilityHost {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.authorize_port(request.instance.as_str())?;
        if let Some(gate) = *self.inner.access_gate.read().expect("gate") {
            if !self.allows_protocol(request.instance.as_str(), gate) {
                return Err(Error::unavailable(format!(
                    "实例 {} 未声明协议 {}",
                    request.instance,
                    gate.as_str()
                )));
            }
        }
        let scope = request.locals_scope.clone().unwrap_or_default();
        debug!(instance = %request.instance, method = %request.method, "invoke");
        self.inner
            .invoke_internal(
                &request.instance,
                &request.method,
                request.args,
                scope,
                Uuid::new_v4().to_string(),
                0,
                None,
                true,
                request.invoke_mask,
            )
            .await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        let entries = self.inner.catalog_snapshot();
        let gate = *self.inner.access_gate.read().expect("gate");
        match gate {
            Some(kind) => entries
                .into_iter()
                .filter(|e| self.allows_protocol(e.instance.as_str(), kind))
                .collect(),
            None => entries,
        }
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        self.inner
            .catalog_snapshot()
            .into_iter()
            .find(|e| e.instance == *instance && e.method == *method)
    }
}
