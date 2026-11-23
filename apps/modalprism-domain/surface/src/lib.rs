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
            ToolParam { name: "url", kind: "string", required: false, summary: "要采集的地址（仅 http/https）" },
            ToolParam { name: "text", kind: "string", required: false, summary: "直接给出的文本" },
            ToolParam { name: "content", kind: "string", required: false, summary: "直接给出的内容本体（base64）；url/text/content 三选一" },
            ToolParam { name: "mediaType", kind: "string", required: false, summary: "text/content 的媒体类型，缺省按内容推导" },
            ToolParam { name: "mode", kind: "string", required: false, summary: "preview（默认）或 store" },
            ToolParam { name: "hint", kind: "string", required: false, summary: "content（默认）或 discover" },
            ToolParam { name: "sourceId", kind: "string", required: false, summary: "归属信源，缺省为临时信源" },
            ToolParam { name: "selectors", kind: "array", required: false, summary: "识别类型过滤" },
            ToolParam { name: "pipeline", kind: "array", required: false, summary: "显式适配器流水线" },
            ToolParam { name: "vectorize", kind: "boolean", required: false, summary: "写入后触发向量派生（缺省 false）" },
            ToolParam { name: "fromSearch", kind: "string", required: false, summary: "从持久化检索记录（searchId）取 URL 逐条执行" },
            ToolParam { name: "index", kind: "number", required: false, summary: "fromSearch 的第 N 条（0 起）；缺省按序全部" },
        ],
    },
    ToolSpec {
        id: "probe",
        summary: "只识别不落库：类型分布、区域与建议声明",
        write: false,
        params: &[
            ToolParam { name: "url", kind: "string", required: false, summary: "要探测的地址（仅 http/https）" },
            ToolParam { name: "text", kind: "string", required: false, summary: "直接给出的文本" },
            ToolParam { name: "content", kind: "string", required: false, summary: "直接给出的内容本体（base64）；url/text/content 三选一" },
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
            ToolParam { name: "schedule", kind: "object", required: false, summary: "调度间隔；缺省不写，节律交由采集计划表达" },
            ToolParam { name: "sourceType", kind: "string", required: false, summary: "信源类型，缺省按访问器与定位形状推导" },
        ],
    },
    ToolSpec {
        id: "ls",
        summary: "发现信源与可挂载组件；可按组件 kind 与标签过滤目录",
        write: false,
        params: &[
            ToolParam { name: "kind", kind: "string", required: false, summary: "组件类型过滤：accessor / adapter / type" },
            ToolParam { name: "tag", kind: "string", required: false, summary: "标签过滤目录，仅用于发现，不用于调度" },
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
            ToolParam { name: "scope", kind: "string", required: false, summary: "web（默认）检索网页；content 检索已采集内容" },
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
    ToolSpec {
        id: "plan.ls",
        summary: "列出采集计划（含 pipeline、vectorize 与运行态摘要）",
        write: false,
        params: &[
            ToolParam { name: "status", kind: "string", required: false, summary: "按状态过滤：active / paused" },
        ],
    },
    ToolSpec {
        id: "plan.get",
        summary: "读取采集计划详情（含 pipeline、vectorize 与进度）",
        write: false,
        params: &[
            ToolParam { name: "planId", kind: "string", required: true, summary: "计划标识" },
        ],
    },
    ToolSpec {
        id: "plan.new",
        summary: "新建采集计划（策略的状态实例：引用策略并携带状态）",
        write: true,
        params: &[
            ToolParam { name: "name", kind: "string", required: false, summary: "计划名称" },
            ToolParam { name: "strategy", kind: "string", required: true, summary: "引用的策略（有计划必有策略）" },
            ToolParam { name: "enabled", kind: "boolean", required: false, summary: "创建后是否立即启用采集" },
            ToolParam { name: "description", kind: "string", required: false, summary: "说明" },
        ],
    },
    ToolSpec {
        id: "plan.set",
        summary: "修改采集计划的状态与元数据（策略引用、启停、一次性 URL 数据）",
        write: true,
        params: &[
            ToolParam { name: "planId", kind: "string", required: true, summary: "计划标识" },
            ToolParam { name: "strategy", kind: "string", required: false, summary: "改引其他策略（有计划必有策略）" },
            ToolParam { name: "enabled", kind: "boolean", required: false, summary: "启停采集" },
            ToolParam { name: "urls", kind: "array", required: false, summary: "一次性 URL 目标（整体替换）" },
            ToolParam { name: "appendUrls", kind: "array", required: false, summary: "追加一次性 URL 目标" },
            ToolParam { name: "name", kind: "string", required: false, summary: "计划名称" },
            ToolParam { name: "description", kind: "string", required: false, summary: "说明" },
        ],
    },
    ToolSpec {
        id: "plan.pause",
        summary: "暂停采集计划",
        write: true,
        params: &[ToolParam { name: "planId", kind: "string", required: true, summary: "计划标识" }],
    },
    ToolSpec {
        id: "plan.resume",
        summary: "恢复采集计划",
        write: true,
        params: &[ToolParam { name: "planId", kind: "string", required: true, summary: "计划标识" }],
    },
    ToolSpec {
        id: "plan.run",
        summary: "立即触发一次信源采集，返回异步操作",
        write: true,
        params: &[
            ToolParam { name: "sourceId", kind: "string", required: true, summary: "信源标识" },
            ToolParam { name: "mode", kind: "string", required: false, summary: "normal（默认）/ dryRun / singleLocator" },
            ToolParam { name: "locator", kind: "string", required: false, summary: "singleLocator 模式的地址" },
            ToolParam { name: "reason", kind: "string", required: false, summary: "触发原因" },
        ],
    },
    ToolSpec {
        id: "strategy.ls",
        summary: "列出采集策略（可复用的节律/额度/停止策略定义）",
        write: false,
        params: &[],
    },
    ToolSpec {
        id: "strategy.get",
        summary: "读取采集策略详情",
        write: false,
        params: &[ToolParam { name: "strategyId", kind: "string", required: true, summary: "策略标识" }],
    },
    ToolSpec {
        id: "strategy.new",
        summary: "新建采集策略，供多个计划引用复用",
        write: true,
        params: &[
            ToolParam { name: "name", kind: "string", required: false, summary: "策略名称" },
            ToolParam { name: "targets", kind: "array", required: false, summary: "目标信源" },
            ToolParam { name: "trigger", kind: "object", required: true, summary: "节律定义（kind: recurring/oneShot）" },
            ToolParam { name: "pipeline", kind: "array", required: false, summary: "适配器流水线" },
            ToolParam { name: "vectorize", kind: "boolean", required: false, summary: "采集后写向量索引" },
            ToolParam { name: "deliveryTargets", kind: "array", required: false, summary: "投递目标" },
            ToolParam { name: "deliveryEnabled", kind: "boolean", required: false, summary: "是否启用投递" },
            ToolParam { name: "queue", kind: "string", required: false, summary: "队列覆盖" },
            ToolParam { name: "priority", kind: "number", required: false, summary: "优先级" },
            ToolParam { name: "quota", kind: "object", required: false, summary: "额度" },
            ToolParam { name: "stopPolicy", kind: "object", required: false, summary: "停止条件" },
            ToolParam { name: "storage", kind: "object", required: false, summary: "存储根声明" },
            ToolParam { name: "description", kind: "string", required: false, summary: "说明" },
        ],
    },
    ToolSpec {
        id: "strategy.set",
        summary: "修改采集策略；引用它的计划在下次解析时生效",
        write: true,
        params: &[
            ToolParam { name: "strategyId", kind: "string", required: true, summary: "策略标识" },
            ToolParam { name: "targets", kind: "array", required: false, summary: "目标信源" },
            ToolParam { name: "trigger", kind: "object", required: false, summary: "节律定义" },
            ToolParam { name: "pipeline", kind: "array", required: false, summary: "适配器流水线" },
            ToolParam { name: "vectorize", kind: "boolean", required: false, summary: "采集后写向量索引" },
            ToolParam { name: "deliveryTargets", kind: "array", required: false, summary: "投递目标" },
            ToolParam { name: "deliveryEnabled", kind: "boolean", required: false, summary: "是否启用投递" },
            ToolParam { name: "quota", kind: "object", required: false, summary: "额度" },
            ToolParam { name: "stopPolicy", kind: "object", required: false, summary: "停止条件" },
            ToolParam { name: "queue", kind: "string", required: false, summary: "队列覆盖" },
            ToolParam { name: "priority", kind: "number", required: false, summary: "优先级" },
            ToolParam { name: "storage", kind: "object", required: false, summary: "存储根声明" },
            ToolParam { name: "enabled", kind: "boolean", required: false, summary: "启停" },
            ToolParam { name: "name", kind: "string", required: false, summary: "策略名称" },
            ToolParam { name: "description", kind: "string", required: false, summary: "说明" },
        ],
    },
    ToolSpec {
        id: "strategy.remove",
        summary: "删除采集策略；仍被计划引用时拒绝",
        write: true,
        params: &[ToolParam { name: "strategyId", kind: "string", required: true, summary: "策略标识" }],
    },
];

/// 按动词名取工具声明。
pub fn tool(id: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|item| item.id == id)
}

/// 对外默认宣传的工具短名：MCP Tools 与 CLI 短命令共用这一份。
///
/// 兼容别名（`tool.*` 长名、`plan.list`）不在宣传面里，但仍可调用（见 [`resolve_alias`]）。
pub const ADVERTISED_TOOLS: &[&str] = &[
    "once", "search", "ls", "read", "fetch", "plan.ls", "plan.get", "plan.new", "plan.set",
    "plan.pause", "plan.resume", "plan.run",
    "strategy.ls", "strategy.get", "strategy.new", "strategy.set", "strategy.remove",
];

/// 兼容别名解析：`tool.*` 长名剥前缀，计划长名归一到短名（list→ls 等）。
/// 解析结果如果仍是工具动词就照常分发；旧名是否用 profile 收敛是未决设计，这里不做。
pub fn resolve_alias(name: &str) -> &str {
    let stripped = name.strip_prefix("tool.").unwrap_or(name);
    match stripped {
        "plan.list" => "plan.ls",
        "plan.create" => "plan.new",
        "plan.update" => "plan.set",
        "plan.continue" => "plan.run",
        other => other,
    }
}

/// 计划面的动词契约（M-03）：短名是宣传名，别名是兼容写法（语义不变）。
/// CLI 的 `mp plan <短名|别名>` 与 MCP 的 `plan.<短名>` 全部由这里派生。
pub struct PlanVerbSpec {
    /// 短名（CLI 副命令，也是 MCP `plan.<短名>` 的后缀）。
    pub short: &'static str,
    /// 对应的工具动词（`plan.<short>`）。
    pub tool: &'static str,
    /// 兼容别名（长名），语义与短名相同。
    pub aliases: &'static [&'static str],
    /// 是否可能写存储。
    pub write: bool,
    pub summary: &'static str,
}

/// 计划面的唯一事实源：与 [`TOOLS`] 的 `plan.*` 条目、[`ADVERTISED_TOOLS`]、
/// CLI `mp plan --help` 同源，由契约测试守住。
pub const PLANS: &[PlanVerbSpec] = &[
    PlanVerbSpec { short: "ls", tool: "plan.ls", aliases: &["list"], write: false, summary: "列出计划" },
    PlanVerbSpec { short: "get", tool: "plan.get", aliases: &[], write: false, summary: "计划详情与进度游标" },
    PlanVerbSpec { short: "new", tool: "plan.new", aliases: &["create"], write: true, summary: "新建计划" },
    PlanVerbSpec { short: "set", tool: "plan.set", aliases: &["update"], write: true, summary: "更新计划" },
    PlanVerbSpec { short: "pause", tool: "plan.pause", aliases: &[], write: true, summary: "主动中断：不丢游标与已落盘数据" },
    PlanVerbSpec { short: "resume", tool: "plan.resume", aliases: &[], write: true, summary: "从上一已接受游标继续" },
    PlanVerbSpec { short: "run", tool: "plan.run", aliases: &["continue"], write: true, summary: "立即跑一轮（不重置游标）" },
];

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
            "catalog", "fetch", "ls", "once", "parse", "plan.get", "plan.ls", "plan.new",
            "plan.pause", "plan.resume", "plan.run", "plan.set", "probe", "read", "save",
            "search", "strategy.get", "strategy.ls", "strategy.new", "strategy.remove",
            "strategy.set",
        ],
        read_only: &[
            "catalog", "fetch", "ls", "parse", "plan.get", "plan.ls", "probe", "read",
            "search", "strategy.get", "strategy.ls",
        ],
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
            "catalog", "fetch", "ls", "once", "parse", "plan.get", "plan.ls", "plan.new",
            "plan.pause", "plan.resume", "plan.run", "plan.set", "probe", "read", "save",
            "search",
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
            "catalog", "fetch", "ls", "once", "parse", "plan.get", "plan.ls", "plan.new",
            "plan.pause", "plan.resume", "plan.run", "plan.set", "probe", "read", "save",
            "search", "strategy.get", "strategy.ls", "strategy.new", "strategy.remove",
            "strategy.set",
        ],
        read_only: &[
            "catalog", "fetch", "ls", "parse", "plan.get", "plan.ls", "probe", "read",
            "search", "strategy.get", "strategy.ls",
        ],
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
            "catalog", "fetch", "ls", "once", "parse", "plan.get", "plan.ls", "plan.new",
            "plan.pause", "plan.resume", "plan.run", "plan.set", "probe", "read", "save",
            "search",
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
        methods: &[
            "catalog", "fetch", "ls", "parse", "plan.get", "plan.ls", "probe", "read",
            "search", "strategy.get", "strategy.ls",
        ],
    },
];

/// MCP 面的白名单写方法。只收可恢复写（声明文件草稿：新建 / 改写 / 启停，落盘前校验），
/// 破坏性删除（remove）、系统设置类写（proxy）与采集驱动类写一律不在此面。
pub const AGENT_MCP_WRITES: &[Exposure] = &[
    Exposure {
        id: "catalog_admin",
        methods: &["create", "toggle", "update"],
    },
    // 单次采集、声明固化与计划生命周期：可恢复写（内容追加、声明草稿），
    // 且必须先通过回环 API 的项目校验。
    Exposure {
        id: "tool",
        methods: &[
            "once", "plan.new", "plan.pause", "plan.resume", "plan.run", "plan.set", "save",
            "strategy.new", "strategy.remove", "strategy.set",
        ],
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

    /// 验收：默认宣传面（MCP Tools 与 CLI 短命令）必须是既定短名。
    #[test]
    fn default_surface_is_the_accepted_short_names() {
        for name in ADVERTISED_TOOLS {
            assert!(tool(name).is_some(), "默认宣传短名 {name} 不在 TOOLS 表");
        }
    }

    /// 验收：once 与策略动词承载 pipeline 与 vectorize；计划动词不得再声明行为字段。
    #[test]
    fn pipeline_and_vectorize_are_carried_by_the_accepted_verbs() {
        for verb in ["once", "strategy.new", "strategy.set"] {
            let spec = tool(verb).expect("动词应存在");
            for field in ["pipeline", "vectorize"] {
                assert!(
                    spec.params.iter().any(|param| param.name == field),
                    "{verb} 缺少 {field} 字段"
                );
            }
        }
        // 单一语义：行为字段上收策略，计划动词不再宣传任何覆写入口。
        for verb in ["plan.new", "plan.set"] {
            let spec = tool(verb).expect("动词应存在");
            for field in ["pipeline", "vectorize", "trigger", "quota", "stopPolicy", "targets"] {
                assert!(
                    !spec.params.iter().any(|param| param.name == field),
                    "{verb} 不应声明行为字段 {field}"
                );
            }
        }
    }

    /// 验收：不为分层或派生新增或宣传内部方法与新工具。
    #[test]
    fn forbidden_tools_stay_out_of_the_surface() {
        for name in [
            "processor.run",
            "factor.compute",
            "adapter.validate",
            "pipeline.test",
        ] {
            assert!(tool(name).is_none(), "内部方法 {name} 不得成为工具动词");
            assert!(
                !name.split('.').next().is_some_and(|head| head == "vector"),
                "vector.* 不得进入工具面"
            );
        }
        assert!(
            tool_ids().iter().all(|id| !id.starts_with("vector.")),
            "vector.* 工具不得进入 TOOLS 表"
        );
        // 工具名与必填参数不得是模块路径 / crate 名 / 文件系统路径。
        for spec in TOOLS {
            assert!(
                !spec.id.contains('/') && !spec.id.contains('\\'),
                "工具名 {} 不得携带路径分隔符",
                spec.id
            );
            for param in spec.params {
                assert!(
                    param.kind != "path",
                    "{} 的参数 {} 不得要求文件系统路径",
                    spec.id,
                    param.name
                );
                assert!(
                    param.name != "file" && param.name != "path",
                    "{} 的参数 {} 不得进入工具宣传面：本机文件读取只开放给 CLI 入口",
                    spec.id,
                    param.name
                );
            }
        }
        // 开发 SKILL 不进主路径：没有任何动词以 skill 命名。
        assert!(tool_ids().iter().all(|id| !id.starts_with("skill")));
    }

    /// 验收：兼容别名按设计保留——`tool.*` 长名与 `plan.list` 归一到默认短名，
    /// 归一结果仍是可用动词；profile 收敛是未决项，这里不锁定。
    #[test]
    fn compat_aliases_resolve_to_accepted_verbs() {
        assert_eq!(resolve_alias("tool.once"), "once");
        assert_eq!(resolve_alias("tool.plan.new"), "plan.new");
        assert_eq!(resolve_alias("plan.list"), "plan.ls");
        // 非 tool.* 前缀的名字不受影响。
        assert_eq!(resolve_alias("catalog_admin.list"), "catalog_admin.list");
        for alias in ["tool.once", "plan.list"] {
            assert!(tool(resolve_alias(alias)).is_some(), "别名 {alias} 应仍可用");
        }
    }
}
