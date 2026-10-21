//! 运维 HTTP 与控制台使用的 Gateway 表面。

use std::collections::HashSet;
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{Error, InstanceId, MethodId, Result};
use async_trait::async_trait;
use frame_core::{
    AbilityExpose, CatalogEntry, Gateway, InvokeMask, InvokeRequest, InvokeResponse,
    invoke_mask_from_expose,
};

fn mask_for_host(host: &AbilityHost, expose: &AbilityExpose) -> InvokeMask {
    let ids: Vec<String> = host
        .catalog()
        .into_iter()
        .map(|entry| entry.instance.to_string())
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    invoke_mask_from_expose(expose, &refs)
}

/// 控制台与进程内写路径：Expose 白名单 + 调用链 InvokeMask。
pub struct ConsoleGateway {
    host: Arc<AbilityHost>,
    mask: InvokeMask,
}

impl ConsoleGateway {
    pub fn new(host: Arc<AbilityHost>, expose: AbilityExpose) -> Arc<dyn Gateway> {
        let mask = mask_for_host(host.as_ref(), &expose);
        Arc::new(Self { host, mask })
    }
}

#[async_trait]
impl Gateway for ConsoleGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.host
            .invoke(request.with_invoke_mask(self.mask.clone()))
            .await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.host
            .catalog()
            .into_iter()
            .filter(|entry| {
                self.mask
                    .allows(entry.instance.as_str())
            })
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !self.mask.allows(instance.as_str()) {
            return None;
        }
        self.host.get_entry(instance, method)
    }
}

/// 对外 `/v1/abilities`：Expose 白名单 + InvokeMask，不经全局 access_gate。
pub struct HttpOpsGateway {
    host: Arc<AbilityHost>,
    expose: AbilityExpose,
    mask: InvokeMask,
    http_instances: HashSet<String>,
}

impl HttpOpsGateway {
    pub fn new(
        host: Arc<AbilityHost>,
        expose: AbilityExpose,
        http_instances: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Arc<dyn Gateway> {
        let http_instances: HashSet<String> = http_instances
            .into_iter()
            .map(|id| id.as_ref().to_string())
            .collect();
        let mask = mask_for_host(host.as_ref(), &expose);
        Arc::new(Self {
            host,
            expose,
            mask,
            http_instances,
        })
    }

    fn ensure_exposed(&self, instance: &str) -> Result<()> {
        if !self.expose.resolve(instance).enabled {
            return Err(Error::unavailable("实例未开放"));
        }
        if !self.http_instances.contains(instance) {
            return Err(Error::unavailable("实例未开放"));
        }
        Ok(())
    }
}

#[async_trait]
impl Gateway for HttpOpsGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.ensure_exposed(request.instance.as_str())?;
        self.host
            .invoke(request.with_invoke_mask(self.mask.clone()))
            .await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.host
            .catalog()
            .into_iter()
            .filter(|entry| {
                self.expose.resolve(entry.instance.as_str()).enabled
                    && self.http_instances.contains(entry.instance.as_str())
                    && self.mask.allows(entry.instance.as_str())
            })
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !self.expose.resolve(instance.as_str()).enabled
            || !self.http_instances.contains(instance.as_str())
            || !self.mask.allows(instance.as_str())
        {
            return None;
        }
        self.host.get_entry(instance, method)
    }
}

/// MCP / Tools 表面：Expose 白名单 + InvokeMask。
pub struct McpReadGateway {
    host: Arc<AbilityHost>,
    expose: AbilityExpose,
    mask: InvokeMask,
    mcp_instances: HashSet<String>,
}

impl McpReadGateway {
    pub fn new(
        host: Arc<AbilityHost>,
        expose: AbilityExpose,
        mcp_instances: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Arc<dyn Gateway> {
        let mcp_instances: HashSet<String> = mcp_instances
            .into_iter()
            .map(|id| id.as_ref().to_string())
            .collect();
        let mask = mask_for_host(host.as_ref(), &expose);
        Arc::new(Self {
            host,
            expose,
            mask,
            mcp_instances,
        })
    }

    fn ensure_exposed(&self, instance: &str) -> Result<()> {
        if !self.expose.resolve(instance).enabled {
            return Err(Error::unavailable("实例未开放"));
        }
        if !self.mcp_instances.contains(instance) {
            return Err(Error::unavailable("实例未开放"));
        }
        Ok(())
    }
}

#[async_trait]
impl Gateway for McpReadGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.ensure_exposed(request.instance.as_str())?;
        self.host
            .invoke(request.with_invoke_mask(self.mask.clone()))
            .await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.host
            .catalog()
            .into_iter()
            .filter(|entry| {
                self.expose.resolve(entry.instance.as_str()).enabled
                    && self.mcp_instances.contains(entry.instance.as_str())
                    && self.mask.allows(entry.instance.as_str())
            })
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !self.expose.resolve(instance.as_str()).enabled
            || !self.mcp_instances.contains(instance.as_str())
            || !self.mask.allows(instance.as_str())
        {
            return None;
        }
        self.host.get_entry(instance, method)
    }
}
