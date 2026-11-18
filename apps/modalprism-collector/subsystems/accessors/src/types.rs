//! 访问器能力、请求和端口。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use strata_model::{
    ErrorClass, Locator, PartRef, RawEnvelope, ReceiptId, RenderLevel, SourceId, TaskId,
};

use crate::error::AccessError;

/// 网络捕获粒度。与 `policy.capture.network` 的取值对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureNetwork {
    None,
    Trace,
    MatchedBody,
    AllBodies,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaidCapability {
    pub service: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessorCapabilities {
    pub protocols: Vec<String>,
    pub conditional: bool,
    pub render: bool,
    pub layout: bool,
    pub capture_network: CaptureNetwork,
    pub interact: bool,
    pub paid: Option<PaidCapability>,
    pub batchable: bool,
    pub inbound: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessHints {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub render_level: Option<RenderLevel>,
}

#[derive(Clone, Debug)]
pub struct AccessRequest {
    pub task_id: TaskId,
    pub source_id: SourceId,
    pub locator: Locator,
    pub spec: Value,
    pub cursor: Option<Value>,
    pub hints: AccessHints,
    pub policy: Arc<Value>,
    pub deadline: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub final_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaidCall {
    pub service: String,
    pub endpoint: String,
    pub params: Value,
    pub body: Bytes,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaidResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub receipt_id: Option<ReceiptId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub headers: Vec<(String, String)>,
    pub cookie: Option<String>,
}

#[async_trait]
pub trait EgressGateway: Send + Sync {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, AccessError>;
}

#[async_trait]
pub trait PaidGateway: Send + Sync {
    async fn call(&self, request: PaidCall) -> Result<PaidResponse, AccessError>;
}

#[async_trait]
pub trait SessionPool: Send + Sync {
    async fn get(&self, profile: &str) -> Result<Session, AccessError>;
}

pub trait Staging: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<PartRef, AccessError>;
}

pub struct AccessContext {
    pub egress: Arc<dyn EgressGateway>,
    pub paid: Arc<dyn PaidGateway>,
    pub sessions: Arc<dyn SessionPool>,
    pub staging: Arc<dyn Staging>,
    pub cancel: Arc<AtomicBool>,
}

impl AccessContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait]
pub trait Accessor: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn capabilities(&self) -> &AccessorCapabilities;
    fn validate(&self, spec: &Value) -> Result<(), AccessError>;
    async fn access(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
    ) -> Result<Vec<RawEnvelope>, AccessError>;

    /// 按批流式访问：每完成一页（批）即推送给 `batches`，回调成功返回后该批方可丢弃。
    /// 有界内存采集（万级按批落盘）依赖该路径；缺省回退到一次性 `access`（整批单次回调），
    /// 未改造的访问器行为不变。
    async fn access_batched(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
        batches: &mut (dyn BatchHandler + Send),
    ) -> Result<(), AccessError> {
        let envelopes = self.access(request, ctx).await?;
        let cursor = envelopes
            .last()
            .and_then(|item| item.cursor_candidate.clone());
        batches.batch(envelopes, cursor).await
    }
}

/// 每批回调：`envelopes` 是一批（通常一页）的信封，`cursor` 是该批结束时的续取游标
///（取最后一条信封的 `cursor_candidate`，无更多页时为 None）。
#[async_trait]
pub trait BatchHandler {
    async fn batch(
        &mut self,
        envelopes: Vec<RawEnvelope>,
        cursor: Option<Value>,
    ) -> Result<(), AccessError>;
}

/// 把 `BatchHandler` 回调收集进 Vec 的一次性路径适配器（供 `access` 复用）。
pub struct CollectingBatches<'a> {
    pub out: &'a mut Vec<RawEnvelope>,
}

#[async_trait]
impl BatchHandler for CollectingBatches<'_> {
    async fn batch(
        &mut self,
        envelopes: Vec<RawEnvelope>,
        _cursor: Option<Value>,
    ) -> Result<(), AccessError> {
        self.out.extend(envelopes);
        Ok(())
    }
}

pub fn config_error(message: impl Into<String>) -> AccessError {
    AccessError::new(ErrorClass::Config, message)
}
