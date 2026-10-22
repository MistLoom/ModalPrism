//! 运行态查询。不属于反馈层。

use serde::{Deserialize, Serialize};

/// 一条在途调用链。结束后查询为 `None`。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainStatus {
    pub chain_id: String,
    pub instance: String,
    pub method: Option<String>,
    pub span: u64,
    pub running: bool,
}
