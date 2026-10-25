//! 治理清单的唯一来源。
//!
//! 三个应用各自持有能力白名单：采集进程的运维面、只读进程的 MCP 面、控制台的界面面。
//! 白名单写在三个地方会漂移，所以统一在这里声明，由各组合根派生。
//!
//! 两层结构：
//!
//! - [`Ability`]：应用**安装**了哪些能力、每个能力的完整方法集与只读方法。完整方法集必须与
//!   `ability.yml`、`provider.manifest()`、`src/logic/*.rs` 三处一致，由契约测试守住。
//! - [`Exposure`]：应用**对外**只开放其中哪些方法。运维面与控制台出于安全裁剪，只暴露子集。
//!
//! 只读标记只影响目录投影（`stateful`），不改变调用是否被放行。

/// 应用安装的一个能力。
pub struct Ability {
    /// 能力 id，与 `ability.yml` 的 `id` 相同。
    pub id: &'static str,
    /// 完整方法集，升序。
    pub methods: &'static [&'static str],
    /// 其中不写存储的方法。
    pub read_only: &'static [&'static str],
}

/// 应用对外的一个开放面。
pub struct Exposure {
    /// 能力 id。
    pub id: &'static str,
    /// 对外的方法子集，升序。
    pub methods: &'static [&'static str],
}

impl Ability {
    /// 该能力是否声明了这个方法。
    pub fn has(&self, method: &str) -> bool {
        self.methods.contains(&method)
    }

    /// 该方法是否只读。只有声明过的方法才可能只读。
    pub fn is_read_only(&self, method: &str) -> bool {
        self.has(method) && self.read_only.contains(&method)
    }
}

/// 采集进程安装的全部能力。
pub const COLLECTOR: &[Ability] = &[
    Ability {
        id: "accessor_probe",
        methods: &["validate"],
        read_only: &["validate"],
    },
    Ability {
        id: "catalog_admin",
        methods: &[
            "batch",
            "create",
            "list",
            "proxy",
            "remove",
            "templates",
            "toggle",
            "update",
        ],
        read_only: &["list", "templates"],
    },
    Ability {
        id: "committer",
        methods: &["flush", "handle", "recover", "status", "sweep"],
        read_only: &["handle", "status"],
    },
    Ability {
        id: "config_admin",
        methods: &["generation", "load"],
        read_only: &["generation"],
    },
    Ability {
        id: "content_admin",
        methods: &["count", "label", "purge", "restore"],
        read_only: &["count"],
    },
    Ability {
        id: "config_inspect",
        methods: &["diff", "explain", "modules", "validate"],
        read_only: &["diff", "explain", "modules", "validate"],
    },
    Ability {
        id: "egress",
        methods: &["check", "report", "reset", "status"],
        read_only: &["check", "report", "status"],
    },
    Ability {
        id: "health",
        methods: &["report", "status"],
        read_only: &["report", "status"],
    },
    Ability {
        id: "health_record",
        methods: &["evaluate"],
        read_only: &[],
    },
    Ability {
        id: "paid_gateway",
        methods: &["complete", "status"],
        read_only: &["status"],
    },
    Ability {
        id: "planner",
        methods: &[
            "manual", "next", "pause", "queues", "round", "status", "tick", "trigger",
        ],
        read_only: &["next", "queues", "status"],
    },
    Ability {
        id: "runner",
        methods: &["run"],
        read_only: &[],
    },
    Ability {
        id: "strata_queue",
        methods: &[
            "enqueue",
            "extend",
            "lease",
            "list",
            "nack",
            "stats",
            "transition",
        ],
        read_only: &["list", "stats"],
    },
    Ability {
        id: "strata_read",
        methods: &[
            "changes", "get", "has", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
        read_only: &[
            "changes", "get", "has", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
    },
    Ability {
        id: "strata_write",
        methods: &["apply"],
        read_only: &[],
    },
    Ability {
        id: "type_article",
        methods: &["on_commit", "project"],
        read_only: &["on_commit", "project"],
    },
    Ability {
        id: "workflow",
        methods: &[
            "catalog",
            "get",
            "notify_test",
            "remove",
            "remove_model",
            "run",
            "runs",
            "save",
            "save_model",
            "tick",
            "toggle",
        ],
        read_only: &["catalog", "get", "runs"],
    },
];

/// 采集进程的运维面。写侧留在组合根内部，不从这里出去。
pub const COLLECTOR_OPS: &[Exposure] = &[
    Exposure {
        id: "accessor_probe",
        methods: &["validate"],
    },
    Exposure {
        id: "catalog_admin",
        methods: &[
            "batch",
            "create",
            "list",
            "proxy",
            "remove",
            "templates",
            "toggle",
            "update",
        ],
    },
    Exposure {
        id: "committer",
        methods: &["flush", "handle", "recover", "status", "sweep"],
    },
    Exposure {
        id: "config_admin",
        methods: &["generation", "load"],
    },
    Exposure {
        id: "config_inspect",
        methods: &["diff", "explain", "modules", "validate"],
    },
    Exposure {
        id: "content_admin",
        methods: &["count", "label", "purge", "restore"],
    },
    Exposure {
        id: "egress",
        methods: &["check", "report", "reset", "status"],
    },
    Exposure {
        id: "health",
        methods: &["report", "status"],
    },
    Exposure {
        id: "health_record",
        methods: &["evaluate"],
    },
    Exposure {
        id: "planner",
        methods: &[
            "manual", "next", "pause", "queues", "round", "status", "tick", "trigger",
        ],
    },
    Exposure {
        id: "strata_queue",
        methods: &["list", "stats", "transition"],
    },
    Exposure {
        id: "runner",
        methods: &["run"],
    },
    Exposure {
        id: "strata_read",
        methods: &[
            "changes", "get", "has", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
    },
    Exposure {
        id: "type_article",
        methods: &["project"],
    },
    Exposure {
        id: "workflow",
        methods: &[
            "catalog",
            "get",
            "notify_test",
            "remove",
            "remove_model",
            "run",
            "runs",
            "save",
            "save_model",
            "tick",
            "toggle",
        ],
    },
];

/// 只读进程安装的全部能力。
pub const AGENT: &[Ability] = &[
    Ability {
        id: "catalog_admin",
        methods: &[
            "batch",
            "create",
            "list",
            "proxy",
            "remove",
            "templates",
            "toggle",
            "update",
        ],
        read_only: &["list", "templates"],
    },
    Ability {
        id: "config_inspect",
        methods: &["diff", "explain", "modules", "validate"],
        read_only: &["diff", "explain", "modules", "validate"],
    },
    Ability {
        id: "health",
        methods: &["report", "status"],
        read_only: &["report", "status"],
    },
    Ability {
        id: "strata_read",
        methods: &[
            "changes", "get", "has", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
        read_only: &[
            "changes", "get", "has", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
    },
    Ability {
        id: "type_article",
        methods: &["on_commit", "project"],
        read_only: &["on_commit", "project"],
    },
    Ability {
        id: "type_hotlist",
        methods: &["on_commit", "project"],
        read_only: &["on_commit", "project"],
    },
    Ability {
        id: "workflow",
        methods: &[
            "catalog",
            "get",
            "notify_test",
            "remove",
            "remove_model",
            "run",
            "runs",
            "save",
            "save_model",
            "tick",
            "toggle",
        ],
        read_only: &["catalog", "get", "runs"],
    },
];

/// 控制台的界面面。只保留可恢复、无副作用的写方法。
pub const CONSOLE: &[Exposure] = &[
    Exposure {
        id: "catalog_admin",
        methods: &[
            "batch",
            "create",
            "list",
            "proxy",
            "remove",
            "templates",
            "toggle",
            "update",
        ],
    },
    Exposure {
        id: "committer",
        methods: &["flush", "handle", "status", "sweep"],
    },
    Exposure {
        id: "config_admin",
        methods: &["generation", "load"],
    },
    Exposure {
        id: "config_inspect",
        methods: &["diff", "explain", "modules", "validate"],
    },
    Exposure {
        id: "content_admin",
        methods: &["count", "label", "purge", "restore"],
    },
    Exposure {
        id: "egress",
        methods: &["check", "report", "reset", "status"],
    },
    Exposure {
        id: "health",
        methods: &["report", "status"],
    },
    Exposure {
        id: "health_record",
        methods: &["evaluate"],
    },
    Exposure {
        id: "planner",
        methods: &[
            "manual", "next", "pause", "queues", "round", "status", "tick", "trigger",
        ],
    },
    Exposure {
        id: "strata_queue",
        methods: &["list", "stats", "transition"],
    },
    Exposure {
        id: "runner",
        methods: &["run"],
    },
    Exposure {
        id: "strata_read",
        methods: &[
            "changes", "get", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
    },
    Exposure {
        id: "type_article",
        methods: &["project"],
    },
    Exposure {
        id: "workflow",
        methods: &[
            "catalog",
            "get",
            "notify_test",
            "remove",
            "remove_model",
            "run",
            "runs",
            "save",
            "save_model",
            "tick",
            "toggle",
        ],
    },
];

/// 只读进程的 MCP 面。
///
/// 只读进程装了完整能力（好让 manifest 与其它应用一致），但对外只开查询：
/// 信源清单与模板、工作流与运行记录都在这里，写侧一律不出去。
pub const AGENT_MCP: &[Exposure] = &[
    Exposure {
        id: "catalog_admin",
        methods: &["list", "templates"],
    },
    Exposure {
        id: "config_inspect",
        methods: &["diff", "explain", "modules", "validate"],
    },
    Exposure {
        id: "health",
        methods: &["report", "status"],
    },
    Exposure {
        id: "strata_read",
        methods: &[
            "changes", "get", "has", "history", "kinds", "lineage", "query", "related", "resolve",
            "slice", "summary",
        ],
    },
    Exposure {
        id: "type_article",
        methods: &["project"],
    },
    Exposure {
        id: "type_hotlist",
        methods: &["project"],
    },
    Exposure {
        id: "workflow",
        methods: &["catalog", "get", "runs"],
    },
];

/// 按 id 取应用声明的能力。
pub fn ability<'a>(app: &'a [Ability], id: &str) -> Option<&'a Ability> {
    app.iter().find(|item| item.id == id)
}

/// 应用是否安装了这个能力。
pub fn installed(app: &[Ability], id: &str) -> bool {
    ability(app, id).is_some()
}

/// 应用是否对外放开这个方法。未声明的能力与方法一律不放行。
pub fn exposes(app: &[Exposure], id: &str, method: &str) -> bool {
    app.iter()
        .any(|item| item.id == id && item.methods.contains(&method))
}

/// 应用声明的这个方法是否只读。
pub fn is_read_only(app: &[Ability], id: &str, method: &str) -> bool {
    ability(app, id).is_some_and(|item| item.is_read_only(method))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exposure_stays_inside_the_installed_surface() {
        for item in COLLECTOR_OPS {
            let ability = ability(COLLECTOR, item.id).expect("运维面只能引用已安装能力");
            for method in item.methods {
                assert!(ability.has(method), "{} 未声明 {method}", item.id);
            }
        }
        for item in AGENT_MCP {
            let ability = ability(AGENT, item.id).expect("MCP 面只能引用只读进程已装能力");
            for method in item.methods {
                assert!(ability.has(method), "{} 未声明 {method}", item.id);
                assert!(
                    ability.is_read_only(method),
                    "MCP 面 {} {method} 不是只读方法",
                    item.id
                );
            }
        }
        for item in CONSOLE {
            let ability = ability(COLLECTOR, item.id).expect("界面面只能引用采集进程能力");
            for method in item.methods {
                assert!(ability.has(method), "{} 未声明 {method}", item.id);
                assert!(
                    exposes(COLLECTOR_OPS, item.id, method),
                    "界面面 {} {method} 超出运维面",
                    item.id
                );
            }
        }
    }

    #[test]
    fn read_only_methods_are_declared() {
        for ability in COLLECTOR.iter().chain(AGENT.iter()) {
            for method in ability.read_only {
                assert!(ability.has(method), "{} 的只读方法未声明", ability.id);
            }
        }
    }
}
