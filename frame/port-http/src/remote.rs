//! 连接型跨进程 Gateway（可选）：把本进程 `Gateway` 调用转发到远端 Leaf 的 HTTP 端口。
//!
//! 语义：进程内 Gateway 与 HTTP 端口是一一投影关系（catalog/invoke 走同一 REST 面），
//! 因此一个连接到远端 HTTP 端口的客户端即可作为本地调用方的 `Gateway` 使用——
//! 端口、Fun、组合根无需感知远端与本地差别。
//!
//! 目录策略：`Gateway::catalog` / `get_entry` 是同步方法，连接时预取远端目录缓存；
//! invoke 实时转发。远端目录变化可通过 `refresh` 重新拉取。错误经 `error.type`
//! 分类码还原为统一 `Error`，不降级为泛化 5xx。

use std::sync::Arc;

use async_trait::async_trait;
use frame_core::{CatalogEntry, Gateway, InstanceId, InvokeRequest, InvokeResponse, MethodId};
use serde_json::Value;

/// 把远端错误响应还原为统一 `Error`：优先用 body 里的分类码，缺失时按状态码归类。
pub fn remote_error_from_body(status: u16, body: &str) -> frame_core::Error {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body) {
        if let Some(err) = map.get("error") {
            let code = err.get("type").and_then(Value::as_str).unwrap_or("");
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(body)
                .to_string();
            if let Some(kind) = frame_core::ErrorKind::from_code(code) {
                return frame_core::Error::new(kind, message);
            }
        }
    }
    frame_core::Error::upstream(status, body)
}

fn transport_err(err: reqwest::Error) -> frame_core::Error {
    frame_core::Error::unavailable(format!("远端 Gateway 连接失败: {err}"))
}

fn target(instance: &InstanceId, method: &MethodId) -> String {
    format!("{instance}.{method}")
}

/// 连接远端 Leaf HTTP 端口的 Gateway。`base_url` 形如 `http://127.0.0.1:8080`。
pub struct RemoteHttpGateway {
    base_url: String,
    client: reqwest::Client,
    catalog_cache: Arc<tokio::sync::RwLock<Vec<CatalogEntry>>>,
}

impl RemoteHttpGateway {
    /// 仅记录地址，不拉取目录（目录为空，需随后 `refresh` 或用 `connect_async`）。
    pub fn connect(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            catalog_cache: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    /// 连接并预取远端目录。
    pub async fn connect_async(base_url: impl Into<String>) -> Result<Self, frame_core::Error> {
        let gateway = Self::connect(base_url);
        gateway.refresh().await?;
        Ok(gateway)
    }

    /// 重新拉取远端目录缓存。
    pub async fn refresh(&self) -> Result<Vec<CatalogEntry>, frame_core::Error> {
        let resp = self
            .client
            .get(format!("{}/v1/abilities", self.base_url))
            .send()
            .await
            .map_err(transport_err)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(transport_err)?;
        if status >= 400 {
            return Err(remote_error_from_body(status, &text));
        }
        let catalog: Vec<CatalogEntry> = serde_json::from_str(&text)
            .map_err(|err| frame_core::Error::upstream(status, format!("响应解码失败: {err}")))?;
        *self.catalog_cache.write().await = catalog.clone();
        Ok(catalog)
    }

    fn find_in_cache(
        &self,
        catalog: &[CatalogEntry],
        instance: &InstanceId,
        method: &MethodId,
    ) -> Option<CatalogEntry> {
        catalog
            .iter()
            .find(|e| e.instance.as_str() == instance.as_str() && e.method.as_str() == method.as_str())
            .cloned()
    }
}

#[async_trait]
impl Gateway for RemoteHttpGateway {
    async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResponse, frame_core::Error> {
        let url = format!(
            "{}/v1/abilities/{}/invoke",
            self.base_url,
            target(&request.instance, &request.method)
        );
        let body = serde_json::json!({
            "args": request.args,
            "session_id": request.locals_scope,
            "invoke_mask": request.invoke_mask,
        });
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(transport_err)?;
        let status = response.status().as_u16();
        let text = response.text().await.map_err(transport_err)?;
        if status >= 400 {
            return Err(remote_error_from_body(status, &text));
        }
        serde_json::from_str(&text)
            .map_err(|err| frame_core::Error::upstream(status, format!("响应解码失败: {err}")))
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        // 同步方法：仅读缓存；异步上下文可短暂进入运行时读取。
        tokio::task::block_in_place(|| {
            self.catalog_cache
                .try_read()
                .map(|c| c.clone())
                .unwrap_or_default()
        })
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        tokio::task::block_in_place(|| {
            self.catalog_cache
                .try_read()
                .ok()
                .and_then(|c| self.find_in_cache(&c, instance, method))
        })
    }
}
