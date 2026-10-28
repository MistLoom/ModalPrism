//! Runtime status query types (shared with query surfaces).

use serde::{Deserialize, Serialize};

/// One in-flight call chain. Absent from the map once the root returns.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainStatus {
    pub chain_id: String,
    pub instance: String,
    pub method: Option<String>,
    pub span: u64,
    pub running: bool,
    /// When this chain (root span) was registered (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Root (or current registered) span id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    /// Parent span id if this registration is nested (usually None for root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Ability id when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_id: Option<String>,
}
