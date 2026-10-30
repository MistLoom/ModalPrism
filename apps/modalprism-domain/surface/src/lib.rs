//! 治理清单的唯一来源。
//!
//! 三个应用各自持有能力白名单：采集进程的运维面、只读进程的 MCP 面、控制台的界面面。
//! 白名单写在三个地方会漂移，所以统一在这里声明，由各组合根派生。
//!
//! 两层结构：
//!
//! - [`Ability`]：应用**安装**了哪些能力、每个能力的完整方法集与只读方法。完整方法集必须与
//!   `ability.yml`、`provider.manifest()`、`src/logic/*.rs` 三处一致，由契约测试守住。
//!   运行时装载的仓颉 Guest（如 rawprism 解析引擎）不在治理表内：它们没有 logic 面，
//!   由组合根内部接线直连 host 调用，契约测试显式豁免。
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

/// 工具面的一个参数。`kind` 取值：string / number / boolean / object / array。
pub struct ToolParam {
    pub name: &'static str,
    pub kind: &'static str,
    pub required: bool,
    pub summary: &'static str,
}

/// 工具面的一个动词。
pub struct ToolSpec {
    /// 动词名，也是 `POST /tools/{verb}` 的路径段与方法名。
    pub id: &'static str,
    pub summary: &'static str,
    /// 是否可能写存储（写方法进 MCP 写白名单）。
    pub write: bool,
    pub params: &'static [ToolParam],
}

/// 工具面的唯一事实源：`ability/tool` 的 `ability.yml`、`GET /tools` 的目录、
/// CLI 的参数解析与 MCP 的方法投影全部由这里派生，一处新增、处处同步。
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: "catalog",
        summary: "列出全部工具动词及其参数",
        write: false,
        params: &[],
    },
    ToolSpec {
        id: "once",
        summary: "一次完成 取数 → 识别 → 适配 → 写入或返回（类型未识别不失败）",
        write: true,
        params: &[
            ToolParam { name: "url", kind: "string", required: false, summary: "要采集的地址" },
            ToolParam { name: "text", kind: "string", required: false, summary: "直接给出的文本" },
            ToolParam { name: "file", kind: "string", required: false, summary: "本地文件路径" },
            ToolParam { name: "mode", kind: "string", required: false, summary: "preview（默认）或 store" },
            ToolParam { name: "hint", kind: "string", required: false, summary: "content（默认）或 discover" },
            ToolParam { name: "sourceId", kind: "string", required: false, summary: "归属信源，缺省为临时信源" },
            ToolParam { name: "selectors", kind: "array", required: false, summary: "识别类型过滤" },
            ToolParam { name: "pipeline", kind: "array", required: false, summary: "显式适配器流水线" },
        ],
    },
    ToolSpec {
        id: "probe",
        summary: "只识别不落库：类型分布、区域与建议声明",
        write: false,
        params: &[
            ToolParam { name: "url", kind: "string", required: false, summary: "要探测的地址" },
            ToolParam { name: "text", kind: "string", required: false, summary: "直接给出的文本" },
            ToolParam { name: "file", kind: "string", required: false, summary: "本地文件路径" },
            ToolParam { name: "selectors", kind: "array", required: false, summary: "识别类型过滤" },
        ],
    },
    ToolSpec {
        id: "save",
        summary: "把探测结论固化为信源声明",
        write: true,
        params: &[
            ToolParam { name: "name", kind: "string", required: true, summary: "信源名称" },
            ToolParam { name: "url", kind: "string", required: true, summary: "信源地址" },
            ToolParam { name: "accessor", kind: "string", required: false, summary: "访问器 id" },
            ToolParam { name: "pipeline", kind: "array", required: false, summary: "适配器流水线" },
            ToolParam { name: "schedule", kind: "object", required: false, summary: "调度间隔" },
        ],
    },
    ToolSpec {
        id: "fetch",
        summary: "按地址取回正文",
        write: false,
        params: &[
            ToolParam { name: "url", kind: "string", required: true, summary: "要取回的地址" },
            ToolParam { name: "maxBytes", kind: "number", required: false, summary: "正文上限" },
        ],
    },
    ToolSpec {
        id: "search",
        summary: "用已配置的搜索引擎检索",
        write: false,
        params: &[
            ToolParam { name: "query", kind: "string", required: true, summary: "关键词" },
            ToolParam { name: "engine", kind: "string", required: false, summary: "引擎 id" },
            ToolParam { name: "limit", kind: "number", required: false, summary: "返回条数" },
        ],
    },
    ToolSpec {
        id: "parse",
        summary: "对文本或地址做结构化识别",
        write: false,
        params: &[
            ToolParam { name: "text", kind: "string", required: false, summary: "待识别文本" },
            ToolParam { name: "url", kind: "string", required: false, summary: "待识别地址" },
            ToolParam { name: "selectors", kind: "array", required: false, summary: "识别类型过滤" },
        ],
    },
    ToolSpec {
        id: "read",
        summary: "读取已入库内容的详情或单面数据",
        write: false,
        params: &[
            ToolParam { name: "contentId", kind: "string", required: true, summary: "内容标识" },
            ToolParam { name: "facet", kind: "string", required: false, summary: "数据面类型" },
            ToolParam { name: "variant", kind: "string", required: false, summary: "数据面变体" },
        ],
    },
];

/// 按动词名取工具声明。
pub fn tool(id: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|item| item.id == id)
}

/// 工具面的全部动词名，与声明顺序一致。
pub fn tool_ids() -> Vec<&'static str> {
    TOOLS.iter().map(|item| item.id).collect()
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
        methods: &["count", "label", "purge"],
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
        id: "tool",
        methods: &[
            "catalog", "fetch", "once", "parse", "probe", "read", "save", "search",
        ],
        read_only: &["catalog", "fetch", "parse", "probe", "read", "search"],
    },
    Ability {
        id: "type_article",
        methods: &["on_commit", "project"],
        read_only: &["on_commit", "project"],
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
        methods: &["count", "label", "purge"],
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
        id: "tool",
        methods: &[
            "catalog", "fetch", "once", "parse", "probe", "read", "save", "search",
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
    // 对外表面的工具投影：一个能力对 CLI / MCP / HTTP 三面，写方法只进写白名单。
    Ability {
        id: "tool",
        methods: &[
            "catalog", "fetch", "once", "parse", "probe", "read", "save", "search",
        ],
        read_only: &["catalog", "fetch", "parse", "probe", "read", "search"],
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
        methods: &["count", "label", "purge"],
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
    // 控制台的调试入口走同一个工具面：试取/试搜/试解析/单次采集都从 `tool` 出。
    Exposure {
        id: "tool",
        methods: &[
            "catalog", "fetch", "once", "parse", "probe", "read", "save", "search",
        ],
    },
];

/// 只读进程的 MCP 面。
///
/// 只读进程装了完整能力（好让 manifest 与其它应用一致），但对外只开查询：
/// 信源清单与模板、健康与配置溯源都在这里，写侧一律不出去。
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
        id: "tool",
        methods: &["catalog", "fetch", "parse", "probe", "read", "search"],
    },
];

/// MCP 面的白名单写方法。只收可恢复写（声明文件草稿：新建 / 改写 / 启停，落盘前校验），
/// 破坏性删除（remove）、系统设置类写（proxy）与采集驱动类写一律不在此面。
pub const AGENT_MCP_WRITES: &[Exposure] = &[
    Exposure {
        id: "catalog_admin",
        methods: &["create", "toggle", "update"],
    },
    // 单次采集与声明固化：可恢复写（内容追加、声明草稿），且必须先通过回环 API 的项目校验。
    Exposure {
        id: "tool",
        methods: &["once", "save"],
    },
];

/// MCP 面写白名单是否放开这个方法。未声明的方法一律不放行。
pub fn mcp_write_allowed(id: &str, method: &str) -> bool {
    exposes(AGENT_MCP_WRITES, id, method)
}

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
        for item in AGENT_MCP_WRITES {
            let ability = ability(AGENT, item.id).expect("MCP 写白名单只能引用只读进程已装能力");
            for method in item.methods {
                assert!(ability.has(method), "{} 未声明 {method}", item.id);
                assert!(
                    !ability.is_read_only(method),
                    "MCP 写白名单 {} {method} 不该是只读方法",
                    item.id
                );
            }
        }
        // 破坏性删除与系统设置类写永远不进 MCP 面（含写白名单）。
        for (id, method) in [
            ("catalog_admin", "remove"),
            ("catalog_admin", "proxy"),
        ] {
            assert!(
                !mcp_write_allowed(id, method),
                "破坏性/设置写 {id}.{method} 不得进入 MCP 写白名单"
            );
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

    #[test]
    fn tool_verbs_come_from_one_table() {
        let declared = ability(COLLECTOR, "tool").expect("采集进程必须安装 tool 能力");
        let ids = tool_ids();
        let mut declared_methods = declared.methods.to_vec();
        let mut ids_sorted = ids.clone();
        declared_methods.sort_unstable();
        ids_sorted.sort_unstable();
        assert_eq!(
            declared_methods, ids_sorted,
            "tool 能力的方法集必须与 TOOLS 表完全一致"
        );
        // 只读声明同样由表派生：写动词不得声明为只读。
        for spec in TOOLS {
            assert_eq!(
                declared.is_read_only(spec.id),
                !spec.write,
                "动词 {} 的读写声明与 TOOLS 表不一致",
                spec.id
            );
        }
    }
}
