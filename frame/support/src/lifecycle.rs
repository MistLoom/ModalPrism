//! 能力生命周期相位。由 ability-core 推进，能力只实现 `on_lifecycle`。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Discovered,
    Loaded,
    Bound,
    Started,
    Ready,
    Stopping,
    Stopped,
    Unloaded,
    Failed,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Loaded => "loaded",
            Self::Bound => "bound",
            Self::Started => "started",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Unloaded => "unloaded",
            Self::Failed => "failed",
        }
    }
}
