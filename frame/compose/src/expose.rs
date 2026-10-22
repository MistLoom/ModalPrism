//! 顶层宿主：开放表 + 协议控制器。

use std::sync::Arc;

use frame_core::{AbilityExpose, AccessKind, ControllerRegistry, ProtocolController, ProtocolKind};
use serde_json::Value;

/// 组合根开放管理入口。
#[derive(Clone)]
pub struct ToolHost {
    expose: AbilityExpose,
    controllers: ControllerRegistry,
}

impl Default for ToolHost {
    fn default() -> Self {
        Self {
            expose: AbilityExpose::open_all(),
            controllers: ControllerRegistry::default(),
        }
    }
}

impl ToolHost {
    pub fn new(expose: AbilityExpose) -> Self {
        Self {
            expose,
            controllers: ControllerRegistry::default(),
        }
    }

    pub fn from_value(value: &Value) -> Result<Self, frame_core::Error> {
        Ok(Self::new(AbilityExpose::from_value(value)?))
    }

    pub fn with_protocols(mut self, protocols: &[ProtocolKind]) -> Self {
        self.controllers = ControllerRegistry::with_defaults(protocols);
        self
    }

    pub fn override_controller(mut self, ctrl: Arc<dyn ProtocolController>) -> Self {
        self.controllers.override_controller(ctrl);
        self
    }

    pub fn expose(&self) -> &AbilityExpose {
        &self.expose
    }

    pub fn into_expose(self) -> AbilityExpose {
        self.expose
    }

    pub fn controllers(&self) -> &ControllerRegistry {
        &self.controllers
    }

    pub fn controller(&self, kind: ProtocolKind) -> Option<Arc<dyn ProtocolController>> {
        self.controllers.get(kind)
    }

    pub fn is_protocol_enabled(&self, kind: AccessKind) -> bool {
        self.controllers.is_enabled(kind)
    }
}
