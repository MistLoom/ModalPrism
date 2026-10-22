//! Fun 端口：程序内调用 Gateway。

use std::sync::Arc;

use frame_core::{
    CatalogEntry, DefaultFunController, Error, Gateway, InstanceId, InvokeResponse, LocalsScope,
    MethodId, ProtocolController, resolve_target,
};
use serde_json::Value;

#[derive(Clone)]
pub struct FunPort {
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
}

impl FunPort {
    pub fn new(gateway: Arc<dyn Gateway>) -> Self {
        Self::with_controller(gateway, Arc::new(DefaultFunController))
    }

    pub fn with_controller(
        gateway: Arc<dyn Gateway>,
        controller: Arc<dyn ProtocolController>,
    ) -> Self {
        Self {
            gateway,
            controller,
        }
    }

    pub fn controller(&self) -> &Arc<dyn ProtocolController> {
        &self.controller
    }

    pub async fn invoke(
        &self,
        instance: InstanceId,
        method: MethodId,
        args: Value,
        locals_scope: Option<LocalsScope>,
    ) -> Result<InvokeResponse, Error> {
        self.controller
            .invoke(
                self.gateway.as_ref(),
                instance,
                method,
                args,
                None,
                locals_scope,
            )
            .await
    }

    pub async fn invoke_str(
        &self,
        target: &str,
        args: Value,
        locals_scope: Option<LocalsScope>,
    ) -> Result<InvokeResponse, Error> {
        let (inst, method) = resolve_target(target, &self.gateway.catalog())?;
        self.invoke(inst, method, args, locals_scope).await
    }

    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.controller.catalog(self.gateway.as_ref())
    }
}
