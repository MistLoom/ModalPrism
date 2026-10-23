//! 运维端口客户端。控制台不打开数据目录，只读取采集进程公开的运维面。

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

/// 调用失败的三类原因，映射为不同的界面处置。
#[derive(Debug, Error)]
pub enum OpsError {
    #[error("连接失败: {0}")]
    Unreachable(String),
    #[error("{0}")]
    Rejected(String),
    #[error("响应无法解析: {0}")]
    Shape(String),
}

#[derive(Debug, Deserialize)]
struct Envelope {
    data: Value,
}

#[derive(Debug, Deserialize)]
struct Failure {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: String,
}

/// 一个运维端口的调用句柄。
#[derive(Clone)]
pub struct Ops {
    base: String,
    http: reqwest::Client,
}

impl Ops {
    pub fn new(base: &str) -> Result<Self, OpsError> {
        let base = base.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(OpsError::Unreachable("运维端口为空".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| OpsError::Unreachable(error.to_string()))?;
        Ok(Self {
            base: base.to_string(),
            http,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// 调用 `instance.method`。`args` 必须是对象。
    pub async fn call(&self, instance: &str, method: &str, args: Value) -> Result<Value, OpsError> {
        let started = Instant::now();
        let url = format!("{}/v1/abilities/{instance}/invoke", self.base);
        let response = self
            .http
            .post(url)
            .query(&[("method", method)])
            .json(&json!({ "args": args }))
            .send()
            .await
            .map_err(|error| OpsError::Unreachable(format!("{}: {error}", self.base)))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| OpsError::Shape(error.to_string()))?;
        if !status.is_success() {
            let message = serde_json::from_str::<Failure>(&body)
                .map(|failure| failure.error.message)
                .unwrap_or_else(|_| format!("{instance}.{method} 返回 {status}"));
            return Err(OpsError::Rejected(message));
        }
        let envelope: Envelope = serde_json::from_str(&body)
            .map_err(|error| OpsError::Shape(format!("{instance}.{method}: {error}")))?;
        tracing::debug!(
            instance,
            method,
            ms = started.elapsed().as_millis() as u64,
            "运维调用"
        );
        Ok(envelope.data)
    }

    /// 探活：调用只读的 `planner.status`。
    pub async fn probe(&self) -> (bool, u128, Option<String>) {
        let started = Instant::now();
        match self.call("planner", "status", json!({})).await {
            Ok(_) => (true, started.elapsed().as_millis(), None),
            Err(OpsError::Rejected(message)) => {
                (true, started.elapsed().as_millis(), Some(message))
            }
            Err(error) => (
                false,
                started.elapsed().as_millis(),
                Some(error.to_string()),
            ),
        }
    }
}
