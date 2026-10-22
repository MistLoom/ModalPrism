use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{InstanceId, InvokeMask, LocalsScope, MethodId};

/// 统一调用信封。端口译到此结构后进入管道。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvokeRequest {
    pub instance: InstanceId,
    pub method: MethodId,
    #[serde(default = "default_args")]
    pub args: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locals_scope: Option<LocalsScope>,
    /// 调用链内实例可见性；仅影响本链 `ctx.catalog` / `ctx.call`，不改动全局 Expose。
    #[serde(default, skip_serializing_if = "InvokeMask::is_default")]
    pub invoke_mask: InvokeMask,
}

fn default_args() -> Value {
    Value::Object(Default::default())
}

impl InvokeRequest {
    pub fn new(instance: InstanceId, method: MethodId, args: Value) -> Self {
        Self {
            instance,
            method,
            args,
            locals_scope: None,
            invoke_mask: InvokeMask::default(),
        }
    }

    pub fn with_locals(mut self, scope: LocalsScope) -> Self {
        self.locals_scope = Some(scope);
        self
    }

    pub fn with_invoke_mask(mut self, mask: InvokeMask) -> Self {
        self.invoke_mask = mask;
        self
    }
}

/// 统一调用响应。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvokeResponse {
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locals_scope: Option<LocalsScope>,
}

impl InvokeResponse {
    pub fn new(data: Value) -> Self {
        Self {
            data,
            locals_scope: None,
        }
    }

    pub fn with_locals(mut self, scope: LocalsScope) -> Self {
        self.locals_scope = Some(scope);
        self
    }
}
