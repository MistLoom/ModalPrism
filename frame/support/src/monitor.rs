//! Bottom-layer monitor primitives: event DTO / Sink / global registry.
//!
//! Contract only — no concrete App. Enabled by Cargo feature `monitor`
//! (pulled in by apps/monitor). When the feature is off, `monitor_emit!`
//! expands to an empty block (zero hot-path cost).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

static EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

/// Next process-local event sequence number.
pub fn next_seq() -> u64 {
    EVENT_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Unified monitor event envelope for invoke / lifecycle / host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorEvent {
    /// Event timestamp (ms).
    pub at: u64,
    /// Event class: `invoke` / `lifecycle` / `host`.
    pub kind: String,
    /// Phase: invoke `before`/`after`/`error`; lifecycle/host phase name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Instance name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Method name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Call-chain id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    /// Nesting depth (root = 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<u64>,
    /// Unique span id for this invoke layer (graph node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    /// Parent span id (nested call); absent for root — do not invent edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Ability id when known (distinct from instance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_id: Option<String>,
    /// Span/segment start time (ms); typically set on before / open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Span/segment end time (ms); set on after / error / close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Previous lifecycle/host state (transition edge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// New lifecycle/host state (transition edge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Process-local monotonic sequence (optional, for ordering).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Error message when phase = error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Extra detail (JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl MonitorEvent {
    pub fn new(kind: impl Into<String>, at: u64) -> Self {
        Self {
            at,
            kind: kind.into(),
            phase: None,
            instance: None,
            method: None,
            chain_id: None,
            span: None,
            span_id: None,
            parent_span_id: None,
            ability_id: None,
            started_at: None,
            ended_at: None,
            from: None,
            to: None,
            seq: Some(next_seq()),
            error: None,
            detail: None,
        }
    }

    pub fn phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }

    pub fn instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn chain(mut self, chain_id: impl Into<String>, span: u64) -> Self {
        self.chain_id = Some(chain_id.into());
        self.span = Some(span);
        self
    }

    pub fn span_id(mut self, span_id: impl Into<String>) -> Self {
        self.span_id = Some(span_id.into());
        self
    }

    pub fn parent_span_id(mut self, parent: impl Into<String>) -> Self {
        self.parent_span_id = Some(parent.into());
        self
    }

    pub fn ability_id(mut self, ability_id: impl Into<String>) -> Self {
        self.ability_id = Some(ability_id.into());
        self
    }

    pub fn started_at(mut self, started_at: u64) -> Self {
        self.started_at = Some(started_at);
        self
    }

    pub fn ended_at(mut self, ended_at: u64) -> Self {
        self.ended_at = Some(ended_at);
        self
    }

    pub fn transition(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self.to = Some(to.into());
        self
    }

    pub fn from_state(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    pub fn to_state(mut self, to: impl Into<String>) -> Self {
        self.to = Some(to.into());
        self
    }

    pub fn error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// Monitor event sink. Implemented by apps/monitor recorder.
/// Must not block the call path.
pub trait MonitorSink: Send + Sync {
    fn record(&self, event: MonitorEvent);
}

static SINK: OnceLock<RwLock<Option<Arc<dyn MonitorSink>>>> = OnceLock::new();

fn sink_slot() -> &'static RwLock<Option<Arc<dyn MonitorSink>>> {
    SINK.get_or_init(|| RwLock::new(None))
}

/// Register the global monitor sink (last write wins).
pub fn set_monitor_sink(sink: Arc<dyn MonitorSink>) {
    *sink_slot().write().expect("monitor sink") = Some(sink);
}

/// Current sink, or `None` if unset.
pub fn monitor_sink() -> Option<Arc<dyn MonitorSink>> {
    sink_slot().read().expect("monitor sink").clone()
}

/// Clear the sink (turn off recording).
pub fn clear_monitor_sink() {
    *sink_slot().write().expect("monitor sink") = None;
}

/// Emit one event to the registered sink (no-op if unset).
pub fn emit(event: MonitorEvent) {
    if let Some(sink) = monitor_sink() {
        sink.record(event);
    }
}

/// Hot-path macro. Empty when `monitor` feature is off.
#[cfg(feature = "monitor")]
#[macro_export]
macro_rules! monitor_emit {
    ($event:expr) => {
        $crate::monitor::emit($event)
    };
}

/// Empty when feature is off: `$event` is not evaluated.
#[cfg(not(feature = "monitor"))]
#[macro_export]
macro_rules! monitor_emit {
    ($event:expr) => {{}};
}
