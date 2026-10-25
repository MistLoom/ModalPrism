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

/// 长任务句柄的轮询预算：采集进程返回 pending 句柄后，这里轮询到完成为止。
const POLL_BUDGET: Duration = Duration::from_secs(15 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(800);

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
    ///
    /// 长任务由采集进程的门面切面立即返回句柄，这里轮询到完成；
    /// 调用方不再需要为不同方法维护多套超时。
    pub async fn call(&self, instance: &str, method: &str, args: Value) -> Result<Value, OpsError> {
        let started = Instant::now();
        let data = self.raw(instance, method, args).await?;
        let handle = pending_handle(&data);
        if handle.is_none() {
            return Ok(data);
        }
        let deadline = started + POLL_BUDGET;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let poll = self
                .raw(
                    "committer",
                    "handle",
                    json!({ "handle": handle.as_ref().expect("handle checked") }),
                )
                .await?;
            match poll.get("status").and_then(Value::as_str) {
                Some("done") => {
                    return Ok(poll.get("data").cloned().unwrap_or(Value::Null));
                }
                Some("failed") => {
                    let message = poll
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("长任务失败")
                        .to_string();
                    return Err(OpsError::Rejected(message));
                }
                Some("unknown") => {
                    return Err(OpsError::Shape("句柄不存在或已过期".into()));
                }
                _ if Instant::now() >= deadline => {
                    return Err(OpsError::Rejected("长任务轮询超时".into()));
                }
                _ => {}
            }
        }
    }

    async fn raw(&self, instance: &str, method: &str, args: Value) -> Result<Value, OpsError> {
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

/// 响应是否为待完成的长任务句柄。
fn pending_handle(data: &Value) -> Option<String> {
    if data.get("status").and_then(Value::as_str) == Some("pending") {
        return data
            .get("handle")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    None
}
