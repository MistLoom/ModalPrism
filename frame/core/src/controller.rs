//! 协议控制器：编解码、默认值补全；传输生命周期仍由 Port 负责。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::merge::merge_defaults;
use crate::{
    AccessKind, CatalogEntry, Gateway, InstanceId, InvokeRequest, InvokeResponse, LocalsScope,
    MethodId, Result,
};

pub type ProtocolKind = AccessKind;

#[async_trait]
pub trait ProtocolController: Send + Sync {
    fn kind(&self) -> ProtocolKind;

    fn normalize_args(&self, raw: Value, defaults: Option<&Value>) -> Result<Value> {
        Ok(merge_defaults(raw, defaults))
    }

    fn build_request(
        &self,
        instance: InstanceId,
        method: MethodId,
        args: Value,
        locals_scope: Option<LocalsScope>,
    ) -> InvokeRequest {
        let mut req = InvokeRequest::new(instance, method, args);
        if let Some(sid) = locals_scope {
            req = req.with_locals(sid);
        }
        req
    }

    fn map_response(&self, response: InvokeResponse) -> Result<Value> {
        Ok(response.data)
    }

    async fn invoke(
        &self,
        gateway: &dyn Gateway,
        instance: InstanceId,
        method: MethodId,
        raw_args: Value,
        defaults: Option<&Value>,
        locals_scope: Option<LocalsScope>,
    ) -> Result<InvokeResponse> {
        let args = self.normalize_args(raw_args, defaults)?;
        let request = self.build_request(instance, method, args, locals_scope);
        gateway.invoke(request).await
    }

    fn catalog(&self, gateway: &dyn Gateway) -> Vec<CatalogEntry> {
        gateway.catalog()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultCliController;

#[async_trait]
impl ProtocolController for DefaultCliController {
    fn kind(&self) -> ProtocolKind {
        AccessKind::Cli
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultFunController;

#[async_trait]
impl ProtocolController for DefaultFunController {
    fn kind(&self) -> ProtocolKind {
        AccessKind::Fun
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultMcpController;

#[async_trait]
impl ProtocolController for DefaultMcpController {
    fn kind(&self) -> ProtocolKind {
        AccessKind::Mcp
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultHttpController;

#[async_trait]
impl ProtocolController for DefaultHttpController {
    fn kind(&self) -> ProtocolKind {
        AccessKind::Http
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultGrpcController;

#[async_trait]
impl ProtocolController for DefaultGrpcController {
    fn kind(&self) -> ProtocolKind {
        AccessKind::Grpc
    }
}

#[derive(Clone)]
pub struct ControllerRegistry {
    controllers: HashMap<ProtocolKind, Arc<dyn ProtocolController>>,
    enabled: Vec<ProtocolKind>,
}

impl Default for ControllerRegistry {
    fn default() -> Self {
        Self::with_defaults(&[
            AccessKind::Cli,
            AccessKind::Fun,
            AccessKind::Mcp,
            AccessKind::Http,
            AccessKind::Grpc,
        ])
    }
}

impl ControllerRegistry {
    pub fn with_defaults(enabled: &[ProtocolKind]) -> Self {
        let mut controllers: HashMap<ProtocolKind, Arc<dyn ProtocolController>> = HashMap::new();
        for &kind in enabled {
            if kind == AccessKind::Tools {
                // tools 仅用于 runtime 门控，无 Port 控制器
                continue;
            }
            let ctrl: Arc<dyn ProtocolController> = match kind {
                AccessKind::Cli => Arc::new(DefaultCliController),
                AccessKind::Fun => Arc::new(DefaultFunController),
                AccessKind::Mcp => Arc::new(DefaultMcpController),
                AccessKind::Http => Arc::new(DefaultHttpController),
                AccessKind::Grpc => Arc::new(DefaultGrpcController),
                AccessKind::Tools => unreachable!("已在上方跳过"),
            };
            controllers.insert(kind, ctrl);
        }
        Self {
            controllers,
            enabled: enabled.to_vec(),
        }
    }

    pub fn override_controller(&mut self, ctrl: Arc<dyn ProtocolController>) {
        let kind = ctrl.kind();
        if !self.enabled.contains(&kind) {
            self.enabled.push(kind);
        }
        self.controllers.insert(kind, ctrl);
    }

    pub fn get(&self, kind: ProtocolKind) -> Option<Arc<dyn ProtocolController>> {
        self.controllers.get(&kind).cloned()
    }

    pub fn enabled(&self) -> &[ProtocolKind] {
        &self.enabled
    }

    pub fn is_enabled(&self, kind: ProtocolKind) -> bool {
        self.enabled.contains(&kind) && self.controllers.contains_key(&kind)
    }
}
