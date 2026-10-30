//! 语义字典：把程序值翻译成用户语言。
//!
//! 纯数据，无 IO：领域 API 只在读配置覆盖时碰磁盘，前端只消费 `/dictionary` 的输出。
//! 每个条目是 `(domain, code, label)`，`label` 必须用通用语义而不是行话
//! （例如 `tombstoned` → 「已删除」，不出现「墓碑」）。

use serde_json::{Map, Value, json};

/// 字典版本：条目语义变化时自增，前端据此失效缓存。
pub const VERSION: &str = "2";

pub struct Entry {
    /// 值域，例如 `task.queue`。
    pub domain: &'static str,
    /// 程序值，例如 `free`。
    pub code: &'static str,
    /// 用户可见的文案。
    pub label: &'static str,
    /// 语气（徽标配色）：ok / warn / bad / info / neutral / notice。
    pub tone: Option<&'static str>,
}

const fn entry(domain: &'static str, code: &'static str, label: &'static str) -> Entry {
    Entry {
        domain,
        code,
        label,
        tone: None,
    }
}

const fn toned(
    domain: &'static str,
    code: &'static str,
    label: &'static str,
    tone: &'static str,
) -> Entry {
    Entry {
        domain,
        code,
        label,
        tone: Some(tone),
    }
}

/// 全部内置条目。值域与程序值必须与后端枚举一致，前端缺失时回落原值。
pub const ENTRIES: &[Entry] = &[
    // 任务队列：排队阶段决定处理优先级。
    entry("task.queue", "free", "即时队列"),
    entry("task.queue", "paid", "付费队列"),
    entry("task.queue", "render", "渲染队列"),
    entry("task.queue", "push", "推送队列"),
    // 任务来由：为什么会有这次执行。
    entry("task.reason", "schedule", "按计划"),
    entry("task.reason", "manual", "手动"),
    entry("task.reason", "follow", "跟进"),
    // 任务种类。
    entry("task.kind", "collection", "采集"),
    entry("task.kind", "maintenance", "维护"),
    entry("task.kind", "sourceAnalysis", "信源分析"),
    entry("task.kind", "adapterDraft", "适配器草稿"),
    entry("task.kind", "adapterApprove", "适配器批准"),
    // 任务状态。
    toned("task.status", "queued", "排队", "warn"),
    toned("task.status", "leased", "执行中", "info"),
    // 对外任务视图会把 `leased` 映射成 `running`，两种写法都得能翻。
    toned("task.status", "running", "执行中", "info"),
    toned("task.status", "paused", "已暂停", "warn"),
    toned("task.status", "succeeded", "成功", "ok"),
    toned("task.status", "failed", "失败", "bad"),
    toned("task.status", "cancelled", "已取消", "neutral"),
    toned("task.status", "retrying", "重试中", "warn"),
    // 内容数据面类型。
    entry("facet.kind", "raw", "原始内容"),
    entry("facet.kind", "html", "网页正文"),
    entry("facet.kind", "xml", "XML 数据"),
    entry("facet.kind", "md", "Markdown 正文"),
    entry("facet.kind", "text", "纯文本"),
    entry("facet.kind", "json", "JSON 数据"),
    entry("facet.kind", "yaml", "YAML 数据"),
    entry("facet.kind", "struct", "结构化对象"),
    entry("facet.kind", "links", "链接清单"),
    entry("facet.kind", "image", "图片"),
    entry("facet.kind", "audio", "音频"),
    entry("facet.kind", "video", "视频"),
    entry("facet.kind", "file", "附件"),
    entry("facet.kind", "embedding", "向量"),
    entry("facet.kind", "regions", "结构区域"),
    entry("facet.kind", "csv", "表格数据"),
    entry("facet.kind", "detect", "识别结果"),
    // 数据面变体。
    entry("facet.variant", "article", "文章"),
    entry("facet.variant", "hotlist", "热榜"),
    entry("facet.variant", "item", "条目"),
    entry("facet.variant", "discovery", "采集条目"),
    entry("facet.variant", "page", "页面"),
    entry("facet.variant", "jsonld", "结构化标注"),
    entry("facet.variant", "readable", "已净化"),
    entry("facet.variant", "prism", "识别索引"),
    // 内容状态。
    toned("content.status", "active", "有效", "ok"),
    toned("content.status", "tombstoned", "已删除", "neutral"),
    // 关系类型与方向。
    entry("relation", "discovered_by", "由其他内容发现"),
    entry("relation", "derived_from", "派生自"),
    entry("relation", "mentions", "提及"),
    entry("relation", "links_to", "链接到"),
    entry("relation", "same_as", "同一对象"),
    entry("relation", "parent_of", "上级"),
    entry("relation", "child_of", "下级"),
    entry("relation.direction", "out", "出向"),
    entry("relation.direction", "in", "入向"),
    // 操作与事件种类：日志、事件、审计与后台操作共用一个值域。
    // 同一条操作在「最近事件」、操作抽屉与日志页因此显示同一句中文。
    entry("operation.kind", "http", "接口请求"),
    entry("operation.kind", "operation", "后台操作"),
    entry("operation.kind", "collection.run", "手动采集"),
    entry("operation.kind", "collection.job.create", "批量派发采集"),
    entry("operation.kind", "collection.probe", "连接探测"),
    entry("operation.kind", "collection.plan.create", "新建采集计划"),
    entry("operation.kind", "collection.plan.update", "调整采集计划"),
    entry("operation.kind", "collection.plan.pause", "暂停采集计划"),
    entry("operation.kind", "collection.plan.resume", "恢复采集计划"),
    entry("operation.kind", "collection.plan.stop", "计划触顶暂停"),
    entry("operation.kind", "collection.plan.delete", "删除采集计划"),
    entry("operation.kind", "catalog.probe", "连接探测"),
    entry("operation.kind", "catalog.source.create", "新建信源"),
    entry("operation.kind", "catalog.source.update", "修改信源"),
    entry("operation.kind", "catalog.source.delete", "删除信源"),
    entry("operation.kind", "catalog.source.share", "调整信源共享"),
    entry("operation.kind", "catalog.source.toggle", "启停信源"),
    entry(
        "operation.kind",
        "catalog.sourceGroup.create",
        "新建信源分类",
    ),
    entry(
        "operation.kind",
        "catalog.sourceGroup.update",
        "重命名信源分类",
    ),
    entry(
        "operation.kind",
        "catalog.sourceGroup.delete",
        "删除信源分类",
    ),
    entry(
        "operation.kind",
        "catalog.source.group.assign",
        "调整信源分类",
    ),
    entry("operation.kind", "catalog.sourceAnalysis", "信源分析"),
    entry("operation.kind", "catalog.adapterDraft", "生成适配器草稿"),
    entry("operation.kind", "catalog.adapterApprove", "批准适配器"),
    entry("operation.kind", "catalog.adapter.approve", "批准适配器"),
    entry("operation.kind", "catalog.adapter.reject", "驳回适配器"),
    entry("operation.kind", "catalog.binding.create", "新建绑定"),
    entry("operation.kind", "catalog.binding.update", "修改绑定"),
    entry("operation.kind", "catalog.binding.delete", "删除绑定"),
    entry("operation.kind", "catalog.binding.rebind", "换绑信源"),
    entry("operation.kind", "binding.rebind", "换绑信源"),
    entry("operation.kind", "source.delete", "删除信源"),
    entry("operation.kind", "source.probe", "探测信源"),
    entry("operation.kind", "source.analysis", "分析信源"),
    entry("operation.kind", "adapter.draft", "生成适配器草稿"),
    entry("operation.kind", "adapter.approve", "批准适配器"),
    entry("operation.kind", "content.batch", "批量修改内容"),
    entry("operation.kind", "content.update", "修改内容元数据"),
    entry("operation.kind", "content.purge", "清空内容"),
    entry("operation.kind", "content.replay", "按快照重建"),
    entry("operation.kind", "delivery.manual", "手动投递"),
    entry("operation.kind", "delivery.auto.round", "自动投递轮询"),
    entry("operation.kind", "delivery.channel.create", "新建通知渠道"),
    entry("operation.kind", "delivery.channel.delete", "删除通知渠道"),
    entry("operation.kind", "delivery.channel.test", "测试通知渠道"),
    entry("operation.kind", "derivation.run", "派生加工"),
    entry("operation.kind", "egress.host.reset", "复位出站主机"),
    entry("operation.kind", "health.recompute", "重算健康"),
    entry("operation.kind", "logs.export", "导出日志"),
    entry("operation.kind", "maintenance.reset", "清理数据"),
    entry("operation.kind", "maintenance.factoryReset", "恢复出厂设置"),
    entry("operation.kind", "observability.export", "导出观测流"),
    entry("operation.kind", "project.create", "新建项目"),
    entry("operation.kind", "settings.apply", "应用系统设置"),
    entry("operation.kind", "settings.rollback", "回滚系统设置"),
    entry("operation.kind", "settings.model.create", "登记模型"),
    entry("operation.kind", "settings.model.update", "修改模型"),
    entry("operation.kind", "settings.model.delete", "删除模型"),
    entry("operation.kind", "settings.prompt.create", "新建提示词"),
    entry(
        "operation.kind",
        "settings.prompt.version",
        "新增提示词版本",
    ),
    entry("operation.kind", "settings.skill.create", "新建技能"),
    entry("operation.kind", "settings.skill.version", "新增技能版本"),
    entry("operation.kind", "system.start", "系统启动"),
    entry("operation.kind", "system.notice", "系统通知"),
    entry("operation.kind", "task.run", "执行任务"),
    entry("operation.kind", "tasks.batch", "批量操作任务"),
    entry("operation.kind", "tasks.purge", "清空任务"),
    entry("operation.kind", "tool.save", "工具固化信源"),
    // 操作过程事件：操作抽屉里按时间正序展示的那几条。
    entry("operation.event", "queued", "已受理"),
    entry("operation.event", "cancel", "请求取消"),
    toned("operation.event", "succeeded", "成功", "ok"),
    toned("operation.event", "partiallySucceeded", "部分成功", "warn"),
    toned("operation.event", "failed", "失败", "bad"),
    toned("operation.event", "cancelled", "已取消", "neutral"),
    // 框架监测事件：概览「最近事件」按种类、相位与能力名讲清发生了什么。
    entry("monitor.kind", "invoke", "能力调用"),
    entry("monitor.kind", "lifecycle", "生命周期"),
    entry("monitor.kind", "host", "宿主装载"),
    entry("monitor.phase", "before", "调用开始"),
    toned("monitor.phase", "after", "调用完成", "ok"),
    toned("monitor.phase", "error", "调用失败", "bad"),
    entry("monitor.phase", "discovered", "已发现"),
    entry("monitor.phase", "loaded", "已装载"),
    entry("monitor.phase", "bound", "已绑定"),
    entry("monitor.phase", "started", "已启动"),
    toned("monitor.phase", "ready", "就绪", "ok"),
    entry("monitor.phase", "stopping", "停止中"),
    entry("monitor.phase", "stopped", "已停止"),
    entry("monitor.phase", "unloaded", "已卸载"),
    toned("monitor.phase", "failed", "装载失败", "bad"),
    entry("monitor.phase", "load", "装载"),
    entry("monitor.phase", "unload", "卸载"),
    // 能力实例：监测事件里以 `实例.方法` 出现，拆开后按产品语言展示。
    entry("ability", "committer", "提交器"),
    entry("ability", "planner", "计划器"),
    entry("ability", "runner", "执行器"),
    entry("ability", "content_admin", "内容管理"),
    entry("ability", "catalog_admin", "目录管理"),
    entry("ability", "config_admin", "配置管理"),
    entry("ability", "config_inspect", "配置溯源"),
    entry("ability", "health", "健康检查"),
    entry("ability", "health_record", "健康记录"),
    entry("ability", "egress", "出站治理"),
    entry("ability", "paid_gateway", "付费网关"),
    entry("ability", "accessor_probe", "接入探测"),
    entry("ability", "strata_read", "数据读取"),
    entry("ability", "strata_write", "数据写入"),
    entry("ability", "strata_queue", "任务队列"),
    entry("ability", "tool", "工具面"),
    entry("ability", "type_article", "文章视图"),
    entry("ability", "type_hotlist", "热榜视图"),
    entry("ability", "rawprism", "结构解析"),
    entry("ability", "markdown", "Markdown 处理"),
    entry("ability", "store", "内存存储"),
    entry("ability", "store_persist", "持久存储"),
    entry("ability", "store_temp", "临时存储"),
    entry("ability", "counter", "计数器"),
    entry("ability", "echo", "回显"),
    entry("ability", "shout", "广播"),
    entry("ability", "task", "任务"),
    entry("ability", "todo_write", "待办清单"),
    entry("ability", "bash", "命令执行"),
    entry("ability", "read_file", "读取文件"),
    entry("ability", "write_file", "写入文件"),
    entry("ability", "chat", "对话"),
    entry("ability", "dispatch", "分派"),
    entry("ability", "prompt", "提示词"),
    entry("ability", "session", "会话"),
    entry("ability", "skill", "技能"),
    entry("ability", "turn", "轮次"),
    entry("ability", "claude_mod", "插件适配"),
    entry("ability", "dsh_plugin", "插件适配"),
    // 能力方法：与实例名组合成一句可读的动作。
    entry("ability.method", "flush", "立即提交"),
    entry("ability.method", "sweep", "保留清理"),
    entry("ability.method", "handle", "查询结果"),
    entry("ability.method", "recover", "恢复任务"),
    entry("ability.method", "status", "查看状态"),
    entry("ability.method", "round", "调度一轮"),
    entry("ability.method", "tick", "推进一轮"),
    entry("ability.method", "trigger", "触发计划"),
    entry("ability.method", "run", "执行"),
    entry("ability.method", "invoke", "调用"),
    entry("ability.method", "scan", "识别"),
    entry("ability.method", "load", "装载"),
    entry("ability.method", "unload", "卸载"),
    entry("ability.method", "list", "列出"),
    entry("ability.method", "get", "查看"),
    entry("ability.method", "create", "新建"),
    entry("ability.method", "update", "修改"),
    entry("ability.method", "delete", "删除"),
    entry("ability.method", "start", "开始"),
    entry("ability.method", "stop", "停止"),
    entry("ability.method", "cancel", "取消"),
    entry("ability.method", "retry", "重试"),
    entry("ability.method", "health", "健康评估"),
    entry("ability.method", "stats", "统计"),
    entry("ability.method", "summary", "摘要"),
    entry("ability.method", "query", "查询"),
    entry("ability.method", "search", "检索"),
    entry("ability.method", "fetch", "抓取"),
    entry("ability.method", "parse", "解析"),
    entry("ability.method", "probe", "探测"),
    entry("ability.method", "read", "读取"),
    entry("ability.method", "write", "写入"),
    entry("ability.method", "apply", "应用"),
    entry("ability.method", "rollback", "回滚"),
    entry("ability.method", "export", "导出"),
    entry("ability.method", "replay", "重建"),
    entry("ability.method", "purge", "清空"),
    entry("ability.method", "reset", "重置"),
    entry("ability.method", "enable", "启用"),
    entry("ability.method", "disable", "停用"),
    entry("ability.method", "share", "共享"),
    entry("ability.method", "unshare", "取消共享"),
    entry("ability.method", "analyze", "分析"),
    entry("ability.method", "approve", "批准"),
    entry("ability.method", "reject", "驳回"),
    entry("ability.method", "commit", "提交"),
    entry("ability.method", "publish", "发布"),
    entry("ability.method", "deliver", "投递"),
    entry("ability.method", "sync", "同步"),
    entry("ability.method", "ping", "连通检测"),
    entry("ability.method", "batch", "批量操作"),
    entry("ability.method", "proxy", "代理请求"),
    entry("ability.method", "remove", "移除"),
    entry("ability.method", "toggle", "启停切换"),
    entry("ability.method", "templates", "模板清单"),
    entry("ability.method", "generation", "配置代际"),
    entry("ability.method", "count", "数量统计"),
    entry("ability.method", "label", "打标"),
    entry("ability.method", "diff", "差异对比"),
    entry("ability.method", "explain", "来源解释"),
    entry("ability.method", "modules", "模块清单"),
    entry("ability.method", "validate", "校验"),
    entry("ability.method", "check", "出站检查"),
    entry("ability.method", "report", "上报"),
    entry("ability.method", "evaluate", "评估"),
    entry("ability.method", "complete", "补全调用"),
    entry("ability.method", "manual", "手动入队"),
    entry("ability.method", "next", "下一个计划"),
    entry("ability.method", "pause", "暂停计划"),
    entry("ability.method", "queues", "队列视图"),
    entry("ability.method", "enqueue", "入队"),
    entry("ability.method", "extend", "续租"),
    entry("ability.method", "lease", "获取租约"),
    entry("ability.method", "nack", "退回队列"),
    entry("ability.method", "transition", "推进状态"),
    entry("ability.method", "changes", "变更列表"),
    entry("ability.method", "has", "存在判断"),
    entry("ability.method", "history", "历史记录"),
    entry("ability.method", "kinds", "种类清单"),
    entry("ability.method", "lineage", "血缘追踪"),
    entry("ability.method", "related", "关联查询"),
    entry("ability.method", "resolve", "解析标识"),
    entry("ability.method", "slice", "区间读取"),
    entry("ability.method", "catalog", "工具清单"),
    entry("ability.method", "once", "一次跑通"),
    entry("ability.method", "save", "固化为信源"),
    entry("ability.method", "on_commit", "提交后处理"),
    entry("ability.method", "project", "派生投影"),
    // 日志级别。
    entry("log.level", "trace", "跟踪"),
    entry("log.level", "debug", "调试"),
    toned("log.level", "info", "信息", "ok"),
    toned("log.level", "warn", "警告", "warn"),
    toned("log.level", "error", "错误", "bad"),
    // 观测流。
    entry("log.stream", "logs", "运行日志"),
    entry("log.stream", "events", "业务事件"),
    entry("log.stream", "audit", "审计事件"),
    // 操作关联资源的种类。
    entry("ref.kind", "source", "信源"),
    entry("ref.kind", "content", "内容"),
    entry("ref.kind", "plan", "采集计划"),
    entry("ref.kind", "task", "任务"),
    entry("ref.kind", "taskBatch", "任务批处理"),
    entry("ref.kind", "contentBatch", "内容批处理"),
    entry("ref.kind", "sourceAnalysis", "信源分析"),
    entry("ref.kind", "adapterDraft", "适配器草稿"),
    entry("ref.kind", "collectionRun", "采集运行"),
    entry("ref.kind", "delivery", "投递记录"),
    entry("ref.kind", "derivationRun", "派生加工"),
    entry("ref.kind", "egressHost", "出站主机"),
    entry("ref.kind", "export", "导出任务"),
    entry("ref.kind", "factoryReset", "出厂重置"),
    entry("ref.kind", "reset", "数据清理"),
    entry("ref.kind", "health", "健康状态"),
    entry("ref.kind", "settings", "系统设置"),
    entry("ref.kind", "snapshot", "快照"),
    // 运行记录。
    entry("run.mode", "manual", "手动"),
    entry("run.mode", "scheduled", "按计划"),
    toned("run.status", "queued", "排队", "warn"),
    toned("run.status", "running", "执行中", "info"),
    toned("run.status", "succeeded", "成功", "ok"),
    toned("run.status", "failed", "失败", "bad"),
    toned("run.status", "cancelled", "已取消", "neutral"),
    // 信源。
    entry("source.type", "feed", "订阅"),
    entry("source.type", "search", "搜索"),
    entry("source.type", "web", "网页"),
    entry("source.type", "api", "接口"),
    entry("source.type", "push", "推送"),
    entry("source.type", "custom", "自定义"),
    toned("source.health", "healthy", "正常", "ok"),
    toned("source.health", "degraded", "能力受限", "warn"),
    toned("source.health", "failed", "失败", "bad"),
    toned("source.health", "paused", "暂停", "warn"),
    toned("source.health", "unknown", "未知", "neutral"),
    // 计划停止原因：信源页「命中限额/停止」后的括号值。
    entry("source.stopReason", "quota-paused", "额度用尽后暂停"),
    entry("source.stopReason", "quota-exhausted", "额度耗尽"),
    entry("source.stopReason", "budget", "预算耗尽"),
    entry("source.stopReason", "daily-cap", "已达每日上限"),
    // 计划与投递。
    entry("plan.trigger", "schedule", "按计划"),
    entry("plan.trigger", "manual", "手动"),
    entry("plan.trigger", "retry", "重试"),
    toned("delivery.status", "pending", "待投递", "warn"),
    toned("delivery.status", "delivering", "投递中", "info"),
    toned("delivery.status", "delivered", "已投递", "ok"),
    toned("delivery.status", "failed", "失败", "bad"),
    toned("delivery.status", "skipped", "已跳过", "neutral"),
    // 错误类别：告诉用户「能不能重试」。
    toned("error.class", "transient", "可重试", "warn"),
    toned("error.class", "permanent", "不可恢复", "bad"),
    toned("error.class", "config", "配置错误", "bad"),
    toned("error.class", "validation", "参数无效", "bad"),
    toned("error.class", "dependency", "依赖不可用", "warn"),
    // 媒体类型：键值渲染时把 MIME 换成可读形态。
    entry("media.type", "text/html", "网页"),
    entry("media.type", "application/json", "JSON"),
    entry("media.type", "text/markdown", "Markdown"),
    entry("media.type", "text/plain", "纯文本"),
    entry("media.type", "application/xml", "XML"),
    entry("media.type", "text/csv", "表格"),
    entry("media.type", "image/jpeg", "JPEG 图片"),
    entry("media.type", "image/png", "PNG 图片"),
    entry("media.type", "image/webp", "WebP 图片"),
    entry("media.type", "video/mp4", "MP4 视频"),
    entry("media.type", "audio/mpeg", "MP3 音频"),
    entry("media.type", "application/pdf", "PDF 文档"),
];

/// 前端按域分组的查表结构：`domains[domain][code] = {label, tone}`。
pub fn document() -> Value {
    let mut domains: Map<String, Value> = Map::new();
    for item in ENTRIES {
        let bucket = domains
            .entry(item.domain.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = bucket {
            let mut entry = json!({ "label": item.label });
            if let Some(tone) = item.tone {
                entry["tone"] = json!(tone);
            }
            map.insert(item.code.to_string(), entry);
        }
    }
    json!({ "version": VERSION, "domains": domains })
}

/// 单个查询：CLI / 工具目录用，找不到返回 None。
pub fn lookup(domain: &str, code: &str) -> Option<&'static Entry> {
    ENTRIES
        .iter()
        .find(|item| item.domain == domain && item.code == code)
}

/// 配置覆盖条目：来自 `config/dictionary.yaml`，字段必须齐全（label 必填）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Override {
    pub domain: String,
    pub code: String,
    pub label: String,
    #[serde(default)]
    pub tone: Option<String>,
}

/// 用覆盖条目改写内置字典：按 `(domain, code)` 替换，未出现的保持原样。
pub fn merge(overrides: &[Override]) -> Value {
    let mut document = document();
    let Some(domains) = document.get_mut("domains").and_then(Value::as_object_mut) else {
        return document;
    };
    for item in overrides {
        let bucket = domains
            .entry(item.domain.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(map) = bucket.as_object_mut() else {
            continue;
        };
        let mut entry = json!({ "label": item.label });
        if let Some(tone) = &item.tone {
            entry["tone"] = json!(tone);
        }
        map.insert(item.code.clone(), entry);
    }
    document
}
