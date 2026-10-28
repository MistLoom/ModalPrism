//! 单次调用上下文。

use std::collections::HashMap;
use std::sync::Arc;

use ability_support::{
    AbilityCtx, Aspect, ChainStatus, DiscoveryPayload, Effect, Error, EventFilter, EventHandler,
    EventListener, EventOpts, EventReg, InstanceId, InvokeMask, LoadSpec, LocalsScope, MethodId,
    ProtocolKind, Result, Undo,
};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::host::HostInner;

pub struct InvokeContext {
    pub(crate) host: Arc<HostInner>,
    pub(crate) instance: InstanceId,
    pub(crate) method: Option<MethodId>,
    pub(crate) chain_id: String,
    pub(crate) span: u64,
    pub(crate) span_id: String,
    pub(crate) locals_scope: LocalsScope,
    pub(crate) bindings: HashMap<String, InstanceId>,
    pub(crate) undos: Arc<Mutex<Vec<Undo>>>,
    pub(crate) port_path: bool,
    pub(crate) invoke_mask: InvokeMask,
    pub(crate) protocol_filter: Option<ProtocolKind>,
}

#[async_trait]
impl AbilityCtx for InvokeContext {
    fn share(&self) -> Arc<dyn AbilityCtx> {
        Arc::new(Self {
            host: Arc::clone(&self.host),
            instance: self.instance.clone(),
            method: None,
            chain_id: self.chain_id.clone(),
            span: self.span,
            span_id: self.span_id.clone(),
            locals_scope: LocalsScope::new(),
            bindings: self.bindings.clone(),
            undos: Arc::new(Mutex::new(Vec::new())),
            port_path: false,
            invoke_mask: self.invoke_mask.clone(),
            protocol_filter: self.protocol_filter,
        })
    }

    fn with_protocol_filter(&self, kind: ProtocolKind) -> Arc<dyn AbilityCtx> {
        Arc::new(Self {
            host: Arc::clone(&self.host),
            instance: self.instance.clone(),
            method: self.method.clone(),
            chain_id: self.chain_id.clone(),
            span: self.span,
            span_id: self.span_id.clone(),
            locals_scope: self.locals_scope.clone(),
            bindings: self.bindings.clone(),
            undos: Arc::clone(&self.undos),
            port_path: self.port_path,
            invoke_mask: self.invoke_mask.clone(),
            protocol_filter: Some(kind),
        })
    }

    fn retain(&self) -> Arc<dyn AbilityCtx> {
        Arc::new(Self {
            host: Arc::clone(&self.host),
            instance: self.instance.clone(),
            method: self.method.clone(),
            chain_id: self.chain_id.clone(),
            span: self.span,
            span_id: self.span_id.clone(),
            locals_scope: self.locals_scope.clone(),
            bindings: self.bindings.clone(),
            undos: Arc::clone(&self.undos),
            port_path: self.port_path,
            invoke_mask: self.invoke_mask.clone(),
            protocol_filter: self.protocol_filter,
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
        let id = chain_id.unwrap_or(&self.chain_id);
        self.host.lookup_chain(id)
    }

    async fn call(&self, id_or_alias: &str, method: &MethodId, args: Value) -> Result<Value> {
        let target = self.dep(id_or_alias)?;
        if let Some(kind) = self.protocol_filter {
            if !self.host.instance_supports_protocol(target.as_str(), kind) {
                return Err(Error::unavailable(format!(
                    "实例 {} 未声明协议 {}",
                    target,
                    kind.as_str()
                )));
            }
        }
        if !self.invoke_mask.allows(target.as_str()) {
            return Err(Error::unavailable(format!("调用链策略拒绝实例 {}", target)));
        }
        self.host
            .authorize_call(self.instance.as_str(), target.as_str())?;
        self.host
            .invoke_internal(
                &target,
                method,
                args,
                self.locals_scope.clone(),
                self.chain_id.clone(),
                self.span + 1,
                Some(self.span_id.clone()),
                false,
                self.invoke_mask.clone(),
            )
            .await
            .map(|r| r.data)
            .map_err(|e| Error::dependency(id_or_alias, e))
    }

    fn dep(&self, id_or_alias: &str) -> Result<InstanceId> {
        let id = self.lookup_dep(id_or_alias)?;
        if !self.invoke_mask.allows(id.as_str()) {
            return Err(Error::unavailable(format!("调用链策略拒绝实例 {id}")));
        }
        Ok(id)
    }

    fn catalog(&self) -> Vec<DiscoveryPayload> {
        let items = self
            .invoke_mask
            .filter_discoveries(self.host.instances_snapshot());
        match self.protocol_filter {
            Some(kind) => items
                .into_iter()
                .filter(|d| {
                    self.host
                        .instance_supports_protocol(d.instance.as_str(), kind)
                })
                .collect(),
            None => items,
        }
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<Box<dyn EventListener + Send>> {
        self.host.authorize_event(
            self.instance.as_str(),
            &filter,
            ability_support::EventMode::Message,
        )?;
        let listener = self.host.events.listen(filter).await;
        self.host.bind_event_id(&self.instance, listener.id());
        Ok(Box::new(listener))
    }

    async fn handle(
        &self,
        filter: EventFilter,
        handler: EventHandler,
        opts: EventOpts,
    ) -> Result<EventReg> {
        self.host
            .authorize_event(self.instance.as_str(), &filter, handler.mode())?;
        let reg = self.host.events.handle(filter, handler, opts);
        self.host.bind_event_id(&self.instance, reg.id());
        Ok(reg)
    }

    async fn load(&self, spec: LoadSpec) -> Result<InstanceId> {
        self.host.load(spec).await
    }

    async fn unload(&self, instance: &InstanceId) -> Result<()> {
        self.host.unload(instance).await
    }

    async fn local_get(&self, key: &str) -> Result<Option<Value>> {
        self.host.local_get(&self.locals_scope, key).await
    }

    async fn local_set(&self, key: &str, value: Value) -> Result<()> {
        self.host.local_set(&self.locals_scope, key, value).await
    }

    async fn push_effect(&self, effect: Box<dyn Effect>) -> Result<()> {
        let undo = effect.apply(self).await?;
        self.undos.lock().await.push(undo);
        Ok(())
    }

    async fn emit(&self, name: &str, payload: Value) -> Result<()> {
        self.host.emit_ability(&self.instance, name, payload).await
    }

    fn register_aspect(&self, aspect: Arc<dyn Aspect>) -> Result<()> {
        self.host.aspects.write().expect("aspects").push(aspect);
        Ok(())
    }
}

impl InvokeContext {
    fn lookup_dep(&self, id_or_alias: &str) -> Result<InstanceId> {
        if let Some(id) = self.bindings.get(id_or_alias) {
            return Ok(id.clone());
        }
        if let Ok(id) = InstanceId::new(id_or_alias) {
            let map = self.host.instances.read().expect("instances");
            if map
                .get(&id)
                .is_some_and(|s| s.phase == ability_support::Lifecycle::Ready)
            {
                return Ok(id);
            }
        }
        Err(Error::not_found(format!("未绑定依赖 {id_or_alias}")))
    }

    pub async fn run_undos(&self, origin: Result<Value>) -> Result<Value> {
        let mut undos = self.undos.lock().await;
        let mut acc = origin;
        while let Some(u) = undos.pop() {
            if let Err(e) = u.run().await {
                acc = match acc {
                    Ok(_v) => Err(Error::failed(format!("Undo 失败: {e}")).with_undo_error(e)),
                    Err(orig) => Err(orig.with_undo_error(e)),
                };
            }
        }
        acc
    }
}
