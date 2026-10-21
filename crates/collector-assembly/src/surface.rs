//! 运维 HTTP 与控制台使用的 Gateway 表面。

use std::collections::HashSet;
use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{Error, InstanceId, MethodId, Result};
use async_trait::async_trait;
use frame_core::{
    AbilityExpose, AccessKind, CatalogEntry, Gateway, InvokeRequest, InvokeResponse,
};

/// 控制台与进程内写路径：不施加 HTTP 协议门控，归属应用内全部已加载实例可调用。
pub struct ConsoleGateway {
    host: Arc<AbilityHost>,
}

impl ConsoleGateway {
    pub fn new(host: Arc<AbilityHost>) -> Arc<dyn Gateway> {
        Arc::new(Self { host })
    }
}

#[async_trait]
impl Gateway for ConsoleGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.host.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.host.catalog()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        self.host.get_entry(instance, method)
    }
}

/// 对外 `/v1/abilities`：HTTP 协议门控 + Expose 白名单。
pub struct HttpOpsGateway {
    host: Arc<AbilityHost>,
    expose: AbilityExpose,
    http_instances: HashSet<String>,
    lock: tokio::sync::Mutex<()>,
}

impl HttpOpsGateway {
    pub fn new(
        host: Arc<AbilityHost>,
        expose: AbilityExpose,
        http_instances: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Arc<dyn Gateway> {
        Arc::new(Self {
            host,
            expose,
            http_instances: http_instances
                .into_iter()
                .map(|id| id.as_ref().to_string())
                .collect(),
            lock: tokio::sync::Mutex::new(()),
        })
    }

    fn ensure_exposed(&self, instance: &str) -> Result<()> {
        if !self.expose.resolve(instance).enabled {
            return Err(Error::unavailable("实例未开放"));
        }
        Ok(())
    }
}

#[async_trait]
impl Gateway for HttpOpsGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.ensure_exposed(request.instance.as_str())?;
        let _guard = self.lock.lock().await;
        self.host.set_access_gate(Some(AccessKind::Http));
        let result = self.host.invoke(request).await;
        self.host.set_access_gate(None);
        result
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.host
            .catalog()
            .into_iter()
            .filter(|entry| {
                self.expose.resolve(entry.instance.as_str()).enabled
                    && self.http_instances.contains(entry.instance.as_str())
            })
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !self.expose.resolve(instance.as_str()).enabled
            || !self.http_instances.contains(instance.as_str())
        {
            return None;
        }
        self.host.get_entry(instance, method)
    }
}

/// MCP / Tools 表面：协议门控 + Expose 白名单。
pub struct McpReadGateway {
    host: Arc<AbilityHost>,
    expose: AbilityExpose,
    mcp_instances: HashSet<String>,
    lock: tokio::sync::Mutex<()>,
}

impl McpReadGateway {
    pub fn new(
        host: Arc<AbilityHost>,
        expose: AbilityExpose,
        mcp_instances: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Arc<dyn Gateway> {
        Arc::new(Self {
            host,
            expose,
            mcp_instances: mcp_instances
                .into_iter()
                .map(|id| id.as_ref().to_string())
                .collect(),
            lock: tokio::sync::Mutex::new(()),
        })
    }

    fn ensure_exposed(&self, instance: &str) -> Result<()> {
        if !self.expose.resolve(instance).enabled {
            return Err(Error::unavailable("实例未开放"));
        }
        Ok(())
    }
}

#[async_trait]
impl Gateway for McpReadGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse> {
        self.ensure_exposed(request.instance.as_str())?;
        let _guard = self.lock.lock().await;
        self.host.set_access_gate(Some(AccessKind::Mcp));
        let result = self.host.invoke(request).await;
        self.host.set_access_gate(None);
        result
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.host
            .catalog()
            .into_iter()
            .filter(|entry| {
                self.expose.resolve(entry.instance.as_str()).enabled
                    && self.mcp_instances.contains(entry.instance.as_str())
            })
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !self.expose.resolve(instance.as_str()).enabled
            || !self.mcp_instances.contains(instance.as_str())
        {
            return None;
        }
        self.host.get_entry(instance, method)
    }
}
