//! 运维门面：把组合根已链接的 Ability 收敛成受白名单约束的 Gateway，并以 HTTP 暴露。
//!
//! 长任务（派发、加工、清理）走提交—轮询切面：首次调用立即返回句柄，执行在后台
//! 继续，调用方轮询 `committer.handle`。客户端因此不需要为不同方法维护多套超时。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ability_core::AbilityHost;
use ability_support::{Error as InvokeError, InstanceId, MethodId};
use async_trait::async_trait;
use frame_core::{CatalogEntry, Gateway, InvokeRequest, InvokeResponse};
use serde_json::{Value, json};

/// 需要异步句柄的长任务：它们在进程内同步执行到完成才返回。
/// `workflow.run` 只处理单篇文章、很快，不在此列；`workflow.tick` 才扫一批。
const LONG_CALLS: &[(&str, &str)] = &[
    ("planner", "round"),
    ("planner", "tick"),
    ("planner", "trigger"),
    ("workflow", "tick"),
    ("committer", "sweep"),
];

/// 句柄结果保留时长：过期即可丢弃，调用方轮询不到就当失败。
const HANDLE_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
enum Outcome {
    Running,
    Done(Value),
    Failed(String),
}

#[derive(Debug)]
struct Entry {
    at: Instant,
    outcome: Outcome,
}

/// 运维面白名单来自 `surface`，此处不重复声明。
fn ops_allowed(instance: &str, method: &str) -> bool {
    surface::exposes(surface::COLLECTOR_OPS, instance, method)
}

fn is_long_call(instance: &str, method: &str) -> bool {
    LONG_CALLS.contains(&(instance, method))
}

pub fn gateway(host: Arc<AbilityHost>) -> Arc<dyn Gateway> {
    Arc::new(HandleGateway {
        inner: Arc::new(OpsGateway { inner: host }),
        handles: Arc::new(Mutex::new(HashMap::new())),
        seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
    })
}

struct OpsGateway {
    inner: Arc<AbilityHost>,
}

#[async_trait]
impl Gateway for OpsGateway {
    async fn invoke(&self, request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        if !ops_allowed(request.instance.as_str(), request.method.as_str()) {
            return Err(InvokeError::unavailable("实例未开放"));
        }
        self.inner.invoke(request).await
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner
            .catalog()
            .into_iter()
            .filter(|entry| ops_allowed(entry.instance.as_str(), entry.method.as_str()))
            .collect()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        if !ops_allowed(instance.as_str(), method.as_str()) {
            return None;
        }
        self.inner.get_entry(instance, method)
    }
}

/// 提交—轮询切面：长任务立即返回句柄，其余调用原样透传。
struct HandleGateway {
    inner: Arc<dyn Gateway>,
    handles: Arc<Mutex<HashMap<String, Entry>>>,
    seq: Arc<std::sync::atomic::AtomicU64>,
}

impl HandleGateway {
    fn poll(&self, id: &str) -> Value {
        let guard = self.handles.lock().expect("handles");
        match guard.get(id) {
            Some(Entry {
                outcome: Outcome::Running,
                ..
            }) => json!({ "status": "pending" }),
            Some(Entry {
                outcome: Outcome::Done(data),
                ..
            }) => json!({ "status": "done", "data": data }),
            Some(Entry {
                outcome: Outcome::Failed(message),
                ..
            }) => json!({ "status": "failed", "message": message }),
            None => json!({ "status": "unknown" }),
        }
    }

    fn prune(&self) {
        let mut guard = self.handles.lock().expect("handles");
        guard.retain(|_, entry| entry.at.elapsed() < HANDLE_TTL);
    }
}

#[async_trait]
impl Gateway for HandleGateway {
    async fn invoke(&self, request: InvokeRequest) -> ability_support::Result<InvokeResponse> {
        let (instance, method) = (
            request.instance.as_str().to_string(),
            request.method.as_str().to_string(),
        );
        if instance == "committer" && method == "handle" {
            if !ops_allowed("committer", "handle") {
                return Err(InvokeError::unavailable("实例未开放"));
            }
            let id = request
                .args
                .get("handle")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Ok(InvokeResponse::new(self.poll(&id)));
        }
        if !is_long_call(&instance, &method) {
            return self.inner.invoke(request).await;
        }
        self.prune();
        let id = format!(
            "op-{}-{}",
            std::process::id(),
            self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        self.handles.lock().expect("handles").insert(
            id.clone(),
            Entry {
                at: Instant::now(),
                outcome: Outcome::Running,
            },
        );
        let inner = Arc::clone(&self.inner);
        let handles = Arc::clone(&self.handles);
        let tracked = id.clone();
        tokio::spawn(async move {
            let outcome = match inner.invoke(request).await {
                Ok(response) => Outcome::Done(response.data),
                Err(error) => Outcome::Failed(error.to_string()),
            };
            if let Some(entry) = handles.lock().expect("handles").get_mut(&tracked) {
                entry.outcome = outcome;
                entry.at = Instant::now();
            }
        });
        Ok(InvokeResponse::new(json!({
            "status": "pending",
            "handle": id,
        })))
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner.catalog()
    }

    fn get_entry(&self, instance: &InstanceId, method: &MethodId) -> Option<CatalogEntry> {
        self.inner.get_entry(instance, method)
    }
}
