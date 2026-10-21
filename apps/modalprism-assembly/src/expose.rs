use frame_core::AbilityExpose;
use serde_json::json;

/// 运维 HTTP 端口登记了 `AccessKind::Http` 的实例 id。
pub const OPS_HTTP_INSTANCES: &[&str] = &[
    "strata_read",
    "planner",
    "health",
    "config_inspect",
];

/// 只读进程 MCP 端口登记了 `AccessKind::Mcp` 的实例 id。
pub const READ_AGENT_MCP_INSTANCES: &[&str] = &[
    "strata_read",
    "health",
    "config_inspect",
    "type_article",
    "type_hotlist",
];

/// 控制台经 Gateway 可调用的实例（不含 runner、strata_write 等内部面）。
pub fn console_expose() -> AbilityExpose {
    AbilityExpose::from_value(&json!({
        "unknown": "deny",
        "abilities": {
            "config": { "enabled": true },
            "config_inspect": { "enabled": true },
            "strata_read": { "enabled": true },
            "planner": { "enabled": true },
            "health": { "enabled": true },
            "egress": { "enabled": true },
            "committer": { "enabled": true },
            "control_state": { "enabled": true },
            "strata_queue": { "enabled": true }
        }
    }))
    .expect("expose")
}

/// 运维 HTTP `/v1/abilities` 可见实例。
pub fn ops_http_expose() -> AbilityExpose {
    AbilityExpose::from_value(&json!({
        "unknown": "deny",
        "abilities": {
            "strata_read": { "enabled": true },
            "planner": { "enabled": true },
            "health": { "enabled": true },
            "config_inspect": { "enabled": true }
        }
    }))
    .expect("expose")
}

/// 只读进程 MCP / Tools 表面。
pub fn read_agent_expose() -> AbilityExpose {
    AbilityExpose::from_value(&json!({
        "unknown": "deny",
        "abilities": {
            "strata_read": { "enabled": true },
            "health": { "enabled": true },
            "config_inspect": { "enabled": true },
            "type_article": { "enabled": true },
            "type_hotlist": { "enabled": true },
            "planner": { "enabled": true },
            "config": { "enabled": true },
            "strata_queue": { "enabled": true },
            "control_state": { "enabled": true }
        }
    }))
    .expect("expose")
}
