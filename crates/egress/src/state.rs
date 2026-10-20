//! 令牌桶、熔断和 robots 缓存。纯计算，持久化由调用方完成。

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::policy::RateLimit;

pub const TOKEN: i64 = 1_000_000;
pub const RATE_FLOOR: i64 = 100_000;
const CIRCUIT_FAILS: u32 = 5;
const CIRCUIT_WINDOW: usize = 20;
const CIRCUIT_WINDOW_FAILS: usize = 10;
const COOLDOWN_START_MS: i64 = 30_000;
const COOLDOWN_CAP_MS: i64 = 30 * 60 * 1000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Db {
    pub rates: BTreeMap<String, RateRow>,
    pub circuits: BTreeMap<String, CircuitRow>,
    pub robots: BTreeMap<String, RobotsRow>,
    pub audits: Vec<AuditRow>,
    pub origin_next: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateRow {
    pub tokens: i64,
    pub updated_ms: i64,
    pub in_flight: u32,
    pub rate_micros: i64,
    pub not_before: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitRow {
    pub consecutive: u32,
    pub recent: Vec<bool>,
    pub open_until: Option<i64>,
    pub cooldown_ms: i64,
    pub openings: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RobotsRow {
    pub status: u16,
    pub body: String,
    pub fetched_ms: i64,
    pub failed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRow {
    pub origin: String,
    pub category: String,
    pub reason: String,
    pub at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acquire {
    Ready,
    Wait(i64),
    Denied(&'static str),
}

pub fn acquire(
    db: &mut Db,
    key: &str,
    limit: &RateLimit,
    now_ms: i64,
    deadline_ms: i64,
) -> Acquire {
    let row = db.rates.entry(key.to_string()).or_insert_with(|| RateRow {
        tokens: i64::from(limit.burst) * TOKEN,
        updated_ms: now_ms,
        in_flight: 0,
        rate_micros: limit.per_sec_micros,
        not_before: None,
    });
    refill(row, limit, now_ms);
    if row.not_before.is_some_and(|at| at > now_ms) {
        return if row.not_before.unwrap_or(now_ms) <= deadline_ms {
            Acquire::Wait(row.not_before.unwrap_or(now_ms) - now_ms)
        } else {
            Acquire::Denied("retry-after")
        };
    }
    if row.in_flight >= limit.concurrency {
        return if now_ms + 50 <= deadline_ms {
            Acquire::Wait(50)
        } else {
            Acquire::Denied("rate-limit")
        };
    }
    if row.tokens >= TOKEN {
        row.tokens -= TOKEN;
        row.in_flight = row.in_flight.saturating_add(1);
        return Acquire::Ready;
    }
    let rate = row.rate_micros.max(1);
    let need = TOKEN - row.tokens.max(0);
    let wait = need.saturating_mul(1000) / rate;
    if wait <= 0 {
        Acquire::Denied("rate-limit")
    } else if now_ms.saturating_add(wait) <= deadline_ms {
        Acquire::Wait(wait.max(1))
    } else {
        Acquire::Denied("rate-limit")
    }
}

pub fn release(db: &mut Db, key: &str) {
    if let Some(row) = db.rates.get_mut(key) {
        row.in_flight = row.in_flight.saturating_sub(1);
    }
}

pub fn note_response(
    db: &mut Db,
    key: &str,
    limit: &RateLimit,
    status: u16,
    elapsed_ms: i64,
    retry_at: Option<i64>,
) {
    let Some(row) = db.rates.get_mut(key) else {
        return;
    };
    if matches!(status, 429 | 503) {
        row.rate_micros = (row.rate_micros / 2).max(RATE_FLOOR);
    } else if (200..300).contains(&status) && elapsed_ms < 1000 {
        row.rate_micros = (row.rate_micros + RATE_FLOOR).min(limit.per_sec_micros.max(RATE_FLOOR));
    }
    if let Some(retry_at) = retry_at {
        row.not_before = Some(
            row.not_before
                .map(|at| at.max(retry_at))
                .unwrap_or(retry_at),
        );
    }
}

pub fn reset_circuit(row: &mut CircuitRow) {
    row.consecutive = 0;
    row.recent.clear();
    row.open_until = None;
    row.cooldown_ms = COOLDOWN_START_MS;
    row.openings = 0;
}

pub fn circuit_open(db: &Db, host: &str, now_ms: i64) -> bool {
    db.circuits
        .get(host)
        .and_then(|row| row.open_until)
        .is_some_and(|until| until > now_ms)
}

pub fn circuit_result(db: &mut Db, host: &str, ok: bool, now_ms: i64) {
    let row = db
        .circuits
        .entry(host.to_string())
        .or_insert_with(|| CircuitRow {
            consecutive: 0,
            recent: Vec::new(),
            open_until: None,
            cooldown_ms: COOLDOWN_START_MS,
            openings: 0,
        });
    if ok {
        row.consecutive = 0;
        row.openings = 0;
        row.cooldown_ms = COOLDOWN_START_MS;
        row.open_until = None;
        push_recent(row, true);
        return;
    }
    row.consecutive = row.consecutive.saturating_add(1);
    push_recent(row, false);
    let window_full = row.recent.len() == CIRCUIT_WINDOW
        && row.recent.iter().filter(|item| !**item).count() >= CIRCUIT_WINDOW_FAILS;
    if row.consecutive >= CIRCUIT_FAILS || window_full {
        row.openings = row.openings.saturating_add(1);
        row.open_until = Some(now_ms.saturating_add(row.cooldown_ms));
        row.cooldown_ms = (row.cooldown_ms.saturating_mul(2)).min(COOLDOWN_CAP_MS);
        row.consecutive = 0;
    }
}

pub fn origin_wait(db: &Db, origin: &str, now_ms: i64, deadline_ms: i64) -> Acquire {
    match db.origin_next.get(origin).copied() {
        Some(at) if at > now_ms && at <= deadline_ms => Acquire::Wait(at - now_ms),
        Some(at) if at > deadline_ms => Acquire::Denied("crawl-delay"),
        _ => Acquire::Ready,
    }
}

pub fn origin_mark(db: &mut Db, origin: &str, now_ms: i64, delay: Duration) {
    let delay_ms = i64::try_from(delay.as_millis()).unwrap_or(i64::MAX);
    if delay_ms > 0 {
        db.origin_next
            .insert(origin.to_string(), now_ms.saturating_add(delay_ms));
    }
}

fn refill(row: &mut RateRow, limit: &RateLimit, now_ms: i64) {
    if row.rate_micros <= 0 {
        row.rate_micros = limit.per_sec_micros.max(RATE_FLOOR);
    }
    let elapsed = now_ms.saturating_sub(row.updated_ms).max(0);
    let add = elapsed.saturating_mul(row.rate_micros) / 1000;
    let cap = i64::from(limit.burst) * TOKEN;
    row.tokens = (row.tokens.saturating_add(add)).min(cap.max(TOKEN));
    row.updated_ms = now_ms;
}

fn push_recent(row: &mut CircuitRow, ok: bool) {
    row.recent.push(ok);
    if row.recent.len() > CIRCUIT_WINDOW {
        let extra = row.recent.len() - CIRCUIT_WINDOW;
        row.recent.drain(0..extra);
    }
}
