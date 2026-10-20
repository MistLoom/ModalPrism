//! 健康信号。预算和停机不计入失败。

use serde::{Deserialize, Serialize};
use strata_model::ErrorClass;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Health {
    pub status: String,
    pub transient: u32,
    pub permanent: u32,
    pub config: u32,
    pub last_ok_at: Option<String>,
    pub last_error: Option<String>,
    pub selector_miss: u32,
    pub blocked_hits: u32,
    pub blocked_total: u32,
    pub render_level: Option<String>,
    pub cost_7d: u64,
    pub alert: bool,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            status: "ok".into(),
            transient: 0,
            permanent: 0,
            config: 0,
            last_ok_at: None,
            last_error: None,
            selector_miss: 0,
            blocked_hits: 0,
            blocked_total: 0,
            render_level: None,
            cost_7d: 0,
            alert: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Thresholds {
    pub expected_period_ms: Option<u64>,
    pub yield_drop_ratio: f64,
    pub failure_threshold: u32,
}

pub fn counts_toward_health(class: ErrorClass, message: &str) -> bool {
    if class == ErrorClass::Budget {
        return false;
    }
    !matches!(
        message,
        "robots" | "unknown-receipt" | "unknown-wait" | "shutdown"
    )
}

pub fn consecutive(health: &Health) -> u32 {
    health
        .transient
        .saturating_add(health.permanent)
        .saturating_add(health.config)
}
