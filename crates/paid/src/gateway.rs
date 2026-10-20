//! 逻辑请求键、预算和占位回执。

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use accessors::{AccessError, PaidCall, PaidGateway, PaidResponse};
use strata_model::{ErrorClass, ReceiptId, Timestamp};

use crate::persist::{self, Persist};

const PENDING_MS: i64 = 60_000;
const UNKNOWN_COOLDOWN_MS: i64 = 3_600_000;
const DEFAULT_WINDOW_MS: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownMode {
    RetryOnce,
    Manual,
}

#[derive(Clone, Debug, Default)]
pub struct BudgetLimits {
    pub per_minute: Option<u64>,
    pub per_hour: Option<u64>,
    pub per_day: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct CallPolicy {
    pub unknown: UnknownMode,
    pub window: Duration,
    pub limits: BudgetLimits,
}

impl Default for CallPolicy {
    fn default() -> Self {
        Self {
            unknown: UnknownMode::RetryOnce,
            window: Duration::from_secs(24 * 60 * 60),
            limits: BudgetLimits::default(),
        }
    }
}

#[async_trait]
pub trait PaidTransport: Send + Sync {
    async fn send(&self, call: PaidCall) -> Result<PaidResponse, AccessError>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Db {
    receipts: Vec<Receipt>,
    budget: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receipt {
    id: String,
    logical_key: String,
    service: String,
    status: String,
    updated_ms: i64,
    retry_used: bool,
    http_status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

pub struct Gateway<P, T> {
    persist: P,
    transport: T,
    policy: CallPolicy,
    lock: Mutex<()>,
}

impl<P: Persist, T: PaidTransport> Gateway<P, T> {
    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn new(persist: P, transport: T, policy: CallPolicy) -> Self {
        Self {
            persist,
            transport,
            policy,
            lock: Mutex::new(()),
        }
    }

    pub async fn call_at(
        &self,
        request: PaidCall,
        now_ms: i64,
    ) -> Result<PaidResponse, AccessError> {
        let window_ms = i64::try_from(self.policy.window.as_millis())
            .unwrap_or(DEFAULT_WINDOW_MS)
            .max(1);
        let window_start = now_ms.div_euclid(window_ms) * window_ms;
        let logical_key = logical_key(
            &request.service,
            &request.endpoint,
            &request.params,
            window_start,
        );
        let prepared = {
            let _guard = self.lock.lock().await;
            let mut db = load(&self.persist).await?;
            if let Some(done) = replay(&db, &logical_key) {
                return Ok(done);
            }
            match plan(&mut db, &logical_key, self.policy.unknown, now_ms) {
                Plan::Stop(error) => {
                    save(&self.persist, &db).await?;
                    return Err(error);
                }
                Plan::Go { index } => {
                    if let Some(end) =
                        charge(&mut db, &request.service, &self.policy.limits, now_ms)
                    {
                        save(&self.persist, &db).await?;
                        let mut error = AccessError::new(ErrorClass::Budget, "预算不足");
                        error.retry_at = Timestamp::from_unix_millis(end).ok();
                        return Err(error);
                    }
                    let id = match index {
                        Some(index) => {
                            let row = &mut db.receipts[index];
                            row.retry_used = true;
                            row.status = "pending".into();
                            row.updated_ms = now_ms;
                            row.id.clone()
                        }
                        None => {
                            let id = format!("rcpt_{}", ulid::Ulid::new());
                            db.receipts.push(Receipt {
                                id: id.clone(),
                                logical_key,
                                service: request.service.clone(),
                                status: "pending".into(),
                                updated_ms: now_ms,
                                retry_used: false,
                                http_status: 0,
                                headers: Vec::new(),
                                body: Vec::new(),
                            });
                            id
                        }
                    };
                    save(&self.persist, &db).await?;
                    id
                }
            }
        };
        let sent = self.transport.send(request).await;
        let _guard = self.lock.lock().await;
        let mut db = load(&self.persist).await?;
        if let Some(row) = db.receipts.iter_mut().find(|row| row.id == prepared) {
            match &sent {
                Ok(response) => {
                    row.status = "ready".into();
                    row.http_status = response.status;
                    row.headers = response.headers.clone();
                    row.body = response.body.to_vec();
                    row.updated_ms = now_ms;
                }
                Err(_) => {
                    row.status = "unknown".into();
                    row.updated_ms = now_ms;
                }
            }
        }
        save(&self.persist, &db).await?;
        drop(_guard);
        let mut response = sent?;
        response.receipt_id = ReceiptId::new(prepared).ok();
        Ok(response)
    }

    pub async fn complete(&self, receipt_id: &str) -> Result<(), AccessError> {
        let _guard = self.lock.lock().await;
        let mut db = load(&self.persist).await?;
        if let Some(row) = db.receipts.iter_mut().find(|row| row.id == receipt_id) {
            row.status = "completed".into();
        }
        save(&self.persist, &db).await
    }

    pub async fn status(&self, receipt_id: &str) -> Result<Option<String>, AccessError> {
        let _guard = self.lock.lock().await;
        let db = load(&self.persist).await?;
        Ok(db
            .receipts
            .iter()
            .find(|row| row.id == receipt_id)
            .map(|row| row.status.clone()))
    }
}

#[async_trait]
impl<P: Persist, T: PaidTransport> PaidGateway for Gateway<P, T> {
    async fn call(&self, request: PaidCall) -> Result<PaidResponse, AccessError> {
        self.call_at(request, clock_ms()).await
    }
}

pub fn logical_key(service: &str, endpoint: &str, params: &Value, window_start: i64) -> String {
    let body = serde_json::json!({
        "service": service,
        "endpoint": endpoint,
        "params": canonicalize(params),
        "window": window_start,
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    hex(Sha256::digest(bytes))
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                sorted.insert(key.clone(), canonicalize(child));
            }
            serde_json::to_value(sorted).unwrap_or(Value::Null)
        }
        other => other.clone(),
    }
}

fn replay(db: &Db, key: &str) -> Option<PaidResponse> {
    let row = db.receipts.iter().find(|row| {
        row.logical_key == key && matches!(row.status.as_str(), "ready" | "completed")
    })?;
    Some(PaidResponse {
        status: row.http_status,
        headers: row.headers.clone(),
        body: bytes::Bytes::from(row.body.clone()),
        receipt_id: ReceiptId::new(row.id.clone()).ok(),
    })
}

enum Plan {
    Stop(AccessError),
    Go { index: Option<usize> },
}

fn plan(db: &mut Db, key: &str, mode: UnknownMode, now_ms: i64) -> Plan {
    let Some(index) = db.receipts.iter().position(|row| {
        row.logical_key == key && matches!(row.status.as_str(), "pending" | "unknown" | "manual")
    }) else {
        return Plan::Go { index: None };
    };
    let row = &mut db.receipts[index];
    if row.status == "pending" && now_ms.saturating_sub(row.updated_ms) < PENDING_MS {
        return Plan::Stop(AccessError::new(ErrorClass::Transient, "in-flight"));
    }
    if row.status == "pending" {
        row.status = "unknown".into();
        row.updated_ms = now_ms;
    }
    if row.status == "manual" || mode == UnknownMode::Manual {
        row.status = "manual".into();
        return Plan::Stop(AccessError::new(ErrorClass::Permanent, "unknown-receipt"));
    }
    if row.retry_used {
        row.status = "manual".into();
        return Plan::Stop(AccessError::new(ErrorClass::Permanent, "unknown-receipt"));
    }
    if now_ms.saturating_sub(row.updated_ms) < UNKNOWN_COOLDOWN_MS {
        return Plan::Stop(AccessError::new(ErrorClass::Transient, "unknown-wait"));
    }
    Plan::Go { index: Some(index) }
}

fn charge(db: &mut Db, service: &str, limits: &BudgetLimits, now_ms: i64) -> Option<i64> {
    let windows = windows(service, limits, now_ms);
    if windows
        .iter()
        .any(|(key, _end, cap)| db.budget.get(key).copied().unwrap_or(0) >= *cap)
    {
        return Some(
            windows
                .iter()
                .filter(|(key, _, cap)| db.budget.get(key).copied().unwrap_or(0) >= *cap)
                .map(|(_, end, _)| *end)
                .max()
                .unwrap_or(now_ms),
        );
    }
    for (key, _, _) in &windows {
        *db.budget.entry(key.clone()).or_insert(0) += 1;
    }
    None
}

fn windows(service: &str, limits: &BudgetLimits, now_ms: i64) -> Vec<(String, i64, u64)> {
    let mut out = Vec::new();
    if let Some(cap) = limits.per_minute {
        let start = now_ms.div_euclid(60_000) * 60_000;
        out.push((format!("{service}:m:{start}"), start + 60_000, cap));
    }
    if let Some(cap) = limits.per_hour {
        let start = now_ms.div_euclid(3_600_000) * 3_600_000;
        out.push((format!("{service}:h:{start}"), start + 3_600_000, cap));
    }
    if let Some(cap) = limits.per_day {
        let start = now_ms.div_euclid(86_400_000) * 86_400_000;
        out.push((format!("{service}:d:{start}"), start + 86_400_000, cap));
    }
    out
}

async fn load<P: Persist>(persist: &P) -> Result<Db, AccessError> {
    let Some(bytes) = persist.load("state").await? else {
        return Ok(Db::default());
    };
    serde_json::from_slice(&bytes).map_err(persist::store_error)
}

async fn save<P: Persist>(persist: &P, db: &Db) -> Result<(), AccessError> {
    let bytes = serde_json::to_vec(db).map_err(persist::store_error)?;
    persist.store("state", &bytes).await
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
