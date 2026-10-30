//! 信源类型分类（v3 R12–R14）：意图层闭集、注册表、派生与校验。
//!
//! 类型是「用户意图」维度的显式化——订阅一个流 / 跑一个查询 / 盯一个页面 / 调一个接口 /
//! 等待推送。它不新增采集能力、不改变派发形态：类型只收窄声明表达、提供创建期默认剧本、
//! 承载类型化健康预期。传输（accessor）与解析（adapter/binding）仍各自独立，类型对它们是
//! 软约束（允许矩阵 + `custom` 逃生舱），不强绑定。
//!
//! 本模块是类型的唯一事实源：校验器、catalog 写路径、契约端点、前端表单都从这里消费，
//! 禁止第二处定义枚举或矩阵（对应 v3 S5）。

use serde_json::{Value, json};

/// 信源类型闭集。序列化名与控制台字段名统一为小写词 `sourceType`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceType {
    /// RSS / Atom 订阅：一个持续追加的条目流。
    Feed,
    /// 关键词搜索：以查询为核心，随时间获取新命中。
    Search,
    /// 网页监测：盯住页面/脚本包，从结构中提取条目。
    Web,
    /// 接口查询：固定参数的结构化接口调用。
    Api,
    /// 外部推送：事件驱动，等待外部推进（不入 tick）。
    Push,
    /// 自定义：矩阵外的过渡形态，接受任意 accessor/adapter 组合。
    Custom,
}

impl SourceType {
    /// 全部闭集值，用于报错文案与契约端点顺序。
    pub const ALL: [SourceType; 6] = [
        SourceType::Feed,
        SourceType::Search,
        SourceType::Web,
        SourceType::Api,
        SourceType::Push,
        SourceType::Custom,
    ];

    /// 序列化/契约用的小写词。
    pub fn as_str(self) -> &'static str {
        match self {
            SourceType::Feed => "feed",
            SourceType::Search => "search",
            SourceType::Web => "web",
            SourceType::Api => "api",
            SourceType::Push => "push",
            SourceType::Custom => "custom",
        }
    }

    /// 解析声明值；未知返回 `None`（由校验器产硬错误）。
    pub fn parse(text: &str) -> Option<SourceType> {
        SourceType::ALL
            .into_iter()
            .find(|item| item.as_str() == text)
    }

    /// 闭集文案，供错误信息复用。
    pub fn closed_set_text() -> String {
        SourceType::ALL
            .iter()
            .map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// 类型注册表条目：矩阵、locator 形态、创建期默认剧本与健康预期。
struct KindSpec {
    kind: SourceType,
    /// 控制台产品命名（中文，面向用户，不暴露 accessor 词汇）。
    label: &'static str,
    /// 期望的 locator 形态：url / query / apiEndpoint / pushChannel / any。
    locator_shape: &'static str,
    /// 允许的访问器 stem（`accessor/rss` → `rss`）；`None` 表示任意。
    allowed_accessors: Option<&'static [&'static str]>,
    /// 控制台简易表单实际能表达的访问器 stem（上限见 `allowed_accessors`）。
    ///
    /// 表单只有地址与频道两种输入，写不出 `paid-api` 的 `params.query` 或 `locator.endpoint`；
    /// 若照 `allowed_accessors` 全量给选项，用户会选到"保存必被派生不一致拒绝"的组合。
    /// 这里声明的是表单可落盘的子集，前端只渲染不判断，避免第二套矩阵。
    form_accessors: Option<&'static [&'static str]>,
    /// 允许的适配器 stem（`adapter/feed-items` → `feed-items`）；`None` 表示任意。
    allowed_adapters: Option<&'static [&'static str]>,
    /// 是否禁止声明 `paid`（feed/web/push 类不该计费）。
    forbid_paid: bool,
    /// 连续多少轮空命中升级为告警（v3 R18）；`None` 表示不做停滞升级。
    stagnation_cycles: Option<u32>,
}

const REGISTRY: [KindSpec; 6] = [
    KindSpec {
        kind: SourceType::Feed,
        label: "RSS 订阅",
        locator_shape: "url",
        allowed_accessors: Some(&["rss", "http"]),
        form_accessors: Some(&["rss", "http"]),
        allowed_adapters: Some(&["feed-items"]),
        forbid_paid: true,
        stagnation_cycles: None,
    },
    KindSpec {
        kind: SourceType::Search,
        label: "关键词搜索",
        locator_shape: "query",
        allowed_accessors: Some(&["search", "paid-api", "http"]),
        // 关键词与接口地址都不在简易表单里，只有内置 search 访问器可直接落盘。
        form_accessors: Some(&["search"]),
        allowed_adapters: Some(&["json-list", "html-list"]),
        forbid_paid: false,
        stagnation_cycles: None,
    },
    KindSpec {
        kind: SourceType::Web,
        label: "网页监测",
        locator_shape: "url",
        allowed_accessors: Some(&["http", "browser", "bundle-list"]),
        // bundle-list 需要列表地址组，简易表单只有单个地址，故不提供。
        form_accessors: Some(&["http", "browser"]),
        allowed_adapters: Some(&["html-list", "json-list", "page-fields"]),
        forbid_paid: true,
        stagnation_cycles: Some(6),
    },
    KindSpec {
        kind: SourceType::Api,
        label: "接口查询",
        locator_shape: "apiEndpoint",
        allowed_accessors: Some(&["api", "paid-api"]),
        // paid-api 需要 `locator.endpoint` 与鉴权头，简易表单写不出。
        form_accessors: Some(&["api"]),
        allowed_adapters: Some(&["json-list"]),
        forbid_paid: false,
        stagnation_cycles: None,
    },
    KindSpec {
        kind: SourceType::Push,
        label: "外部推送",
        locator_shape: "pushChannel",
        allowed_accessors: Some(&["push"]),
        form_accessors: Some(&["push"]),
        allowed_adapters: Some(&["json-list"]),
        forbid_paid: true,
        stagnation_cycles: None,
    },
    KindSpec {
        kind: SourceType::Custom,
        label: "自定义",
        locator_shape: "any",
        allowed_accessors: None,
        form_accessors: None,
        allowed_adapters: None,
        forbid_paid: false,
        stagnation_cycles: None,
    },
];

fn spec_of(kind: SourceType) -> &'static KindSpec {
    REGISTRY
        .iter()
        .find(|item| item.kind == kind)
        .expect("注册表覆盖全部闭集值")
}

/// 连续空命中升级为告警的周期数（v3 R18）；`None` 表示该类型不做停滞升级。
/// 采集进程以它作 per-plan 阈值消费，不写类型分支。
pub fn stagnation_cycles(kind: SourceType) -> Option<u32> {
    spec_of(kind).stagnation_cycles
}

/// 创建期计划默认剧本（v3 R17）。仅供新建 plan/向导路径物化，不回写既有计划。
pub fn plan_defaults(kind: SourceType) -> Value {
    match kind {
        SourceType::Feed => json!({
            "trigger": { "kind": "recurring", "interval": "1h" },
            "quota": { "perRunItems": 8, "maxAge": "365d" },
            "stopPolicy": { "onFailureThreshold": 5 }
        }),
        SourceType::Search => json!({
            "trigger": { "kind": "recurring", "interval": "30m" },
            "quota": {},
            "stopPolicy": { "onBudgetExhausted": "pause" }
        }),
        SourceType::Web => json!({
            "trigger": { "kind": "recurring", "interval": "2h" },
            "quota": { "perRunItems": 30 },
            "stopPolicy": { "onFailureThreshold": 5 }
        }),
        SourceType::Api => json!({
            "trigger": { "kind": "recurring", "interval": "1h" },
            "quota": {},
            "stopPolicy": { "onBudgetExhausted": "pause" }
        }),
        // 推送为事件驱动：不产生 recurring trigger，计划退化为入口 + 准入。
        SourceType::Push => json!({ "trigger": Value::Null, "quota": {}, "stopPolicy": {} }),
        SourceType::Custom => json!({ "trigger": Value::Null, "quota": {}, "stopPolicy": {} }),
    }
}

/// 类型化健康预期（v3 R18）：声明阈值意图与用户语言文案，collector 侧只消费不分支。
fn health_expectation(kind: SourceType) -> Value {
    match kind {
        SourceType::Feed => {
            json!({ "signal": "yieldDrop", "dropRatio": 0.3, "alertText": "该订阅疑似停更" })
        }
        SourceType::Search => json!({ "signal": "budget", "alertText": "查询预算/限流告急" }),
        SourceType::Web => {
            json!({ "signal": "stagnation", "idleCycles": 6, "alertText": "该源疑似改版失效" })
        }
        SourceType::Api => json!({ "signal": "budget", "alertText": "接口预算/限流告急" }),
        SourceType::Push => json!({ "signal": "arrival", "alertText": "长时间无外部推送到达" }),
        SourceType::Custom => json!({ "signal": "generic", "alertText": "健康异常" }),
    }
}

/// 取 `accessor/rss` 之类 id 的 stem；无斜杠时原样返回。
fn stem_of(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

fn pipeline_adapter_stems(module: &Value) -> Vec<String> {
    module
        .get("pipeline")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .flat_map(|step| {
                    ["run", "run_if_accepts"]
                        .into_iter()
                        .flat_map(|key| step.get(key).and_then(Value::as_array))
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|id| stem_of(id).to_string())
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 从信源声明按有序规则推导类型（v3 §5.4）。首个命中即返回，输入全部取自信源本地字段。
pub fn derive(module: &Value) -> SourceType {
    let locator = module.pointer("/access/locator");
    // 规则 1：locator.channel 存在 → push。
    if locator.and_then(|item| item.get("channel")).is_some() {
        return SourceType::Push;
    }
    let accessor = module
        .pointer("/access/accessor")
        .and_then(Value::as_str)
        .map(stem_of)
        .unwrap_or_default();
    match accessor {
        // 规则 2/3/4：按访问器形态直接定性。
        "rss" => return SourceType::Feed,
        "search" => return SourceType::Search,
        "bundle-list" | "browser" => return SourceType::Web,
        // 规则 5：paid-api / api 视是否带查询定 search 或 api。
        "paid-api" | "api" => {
            let has_query = locator.and_then(|item| item.get("query")).is_some()
                || module
                    .pointer("/access/spec/params/query")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty());
            return if has_query {
                SourceType::Search
            } else {
                SourceType::Api
            };
        }
        // 规则 6：http 视 pipeline 是否含 feed-items 定 feed 或 web。
        "http" => {
            return if pipeline_adapter_stems(module)
                .iter()
                .any(|item| item == "feed-items")
            {
                SourceType::Feed
            } else {
                SourceType::Web
            };
        }
        _ => {}
    }
    // 规则 7：无法归类 → custom（等待人工定类）。
    SourceType::Custom
}

/// 有效类型：显式声明优先，否则按规则推导。行渲染、契约与校验都以它为准。
pub fn effective(module: &Value) -> SourceType {
    module
        .get("sourceType")
        .and_then(Value::as_str)
        .and_then(SourceType::parse)
        .unwrap_or_else(|| derive(module))
}

/// 是否缺失显式 `sourceType`（供回填清点，不作为校验硬错误，见 v3 W1.5/W1.6）。
pub fn is_missing(module: &Value) -> bool {
    module.get("sourceType").is_none()
}

/// 类型校验（v3 R12/R14）：返回应阻断写前校验与重载的硬问题。
///
/// - 未知 `sourceType` 值 → 拒绝；
/// - 显式声明与派生结果不一致（`custom` 作为逃生舱除外）→ 拒绝；
/// - 有效类型与 accessor/adapter/paid 的组合违反矩阵 → 拒绝。
///
/// 缺失 `sourceType` 不在此产错：回填前用派生生效，保证整库校验始终绿色（S1 前提）。
pub fn validate(module: &Value) -> Vec<String> {
    let mut issues = Vec::new();
    let declared = module.get("sourceType").and_then(Value::as_str);
    if let Some(text) = declared
        && SourceType::parse(text).is_none()
    {
        issues.push(format!(
            "未知 sourceType: {text}（闭集: {}）",
            SourceType::closed_set_text()
        ));
        return issues;
    }
    let kind = effective(module);
    // 一致性：显式 concrete 值必须等于派生结果；custom 是有意逃生舱，不比对。
    if let Some(text) = declared
        && text != "custom"
    {
        let derived = derive(module);
        if derived != kind {
            issues.push(format!(
                "sourceType={text} 与配置形状推导的 {} 不一致，请改正声明或改用 custom",
                derived.as_str()
            ));
        }
    }
    let spec = spec_of(kind);
    let accessor = module
        .pointer("/access/accessor")
        .and_then(Value::as_str)
        .map(stem_of)
        .unwrap_or_default();
    if let Some(allowed) = spec.allowed_accessors
        && !accessor.is_empty()
        && !allowed.contains(&accessor)
    {
        issues.push(format!(
            "{} 类不允许访问器 {accessor}（允许: {}）",
            spec.label,
            allowed.join(", ")
        ));
    }
    if let Some(allowed) = spec.allowed_adapters {
        for adapter in pipeline_adapter_stems(module) {
            if !allowed.contains(&adapter.as_str()) {
                issues.push(format!(
                    "{} 类不允许适配器 {adapter}（允许: {}）",
                    spec.label,
                    allowed.join(", ")
                ));
            }
        }
    }
    if spec.forbid_paid && module.get("paid").is_some() {
        issues.push(format!("{} 类不应声明 paid 预算", spec.label));
    }
    issues
}

/// 契约端点投影：类型注册表的只读 JSON（v3 R13）。前端表单、校验、健康共享同源。
pub fn registry_json() -> Value {
    let items: Vec<Value> = REGISTRY
        .iter()
        .map(|spec| {
            json!({
                "id": spec.kind.as_str(),
                "label": spec.label,
                "locatorShape": spec.locator_shape,
                "allowedAccessors": spec.allowed_accessors.map(|list| list.to_vec()),
                "formAccessors": spec.form_accessors.map(|list| list.to_vec()),
                "allowedAdapters": spec.allowed_adapters.map(|list| list.to_vec()),
                "forbidPaid": spec.forbid_paid,
                "stagnationCycles": spec.stagnation_cycles,
                "planDefaults": plan_defaults(spec.kind),
                "healthExpectation": health_expectation(spec.kind),
            })
        })
        .collect();
    json!({ "items": items })
}

// ---------------------------------------------------------------------------
// 内置信源分类
// ---------------------------------------------------------------------------
// 分类与 `sourceType` 正交：类型由信源形状决定且是闭集，分类是用户自己的管理
// 口径。这里固定「内置信源归到哪一类」，标识一经发布不再改动——信源正文的
// `groupId` 引用它，改名只改显示名，不迁移成员。

/// 一条内置分类的声明。
pub struct BuiltinGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// 内置分类按领域归档：AI 拆成厂商、研究、媒体三档，金融与医疗按监管/期刊分，
/// 科技单列，系统自带的接入类信源单独一组，避免大量未分组信源混在一起。
pub const BUILTIN_GROUPS: &[BuiltinGroup] = &[
    BuiltinGroup {
        id: "grp_builtin_ai_labs",
        name: "AI 厂商与实验室",
        description: "模型厂商、开源组织与实验室的官方博客和产品更新。",
    },
    BuiltinGroup {
        id: "grp_builtin_ai_research",
        name: "AI 研究与论文",
        description: "论文预印本与学术期刊中的人工智能研究。",
    },
    BuiltinGroup {
        id: "grp_builtin_ai_media",
        name: "AI 媒体与观点",
        description: "科技媒体、独立作者与周刊的人工智能报道和分析。",
    },
    BuiltinGroup {
        id: "grp_builtin_finance",
        name: "金融监管与市场",
        description: "央行、证券监管与主流财经媒体的公开信息。",
    },
    BuiltinGroup {
        id: "grp_builtin_health",
        name: "医疗与期刊",
        description: "监管机构、顶级医学期刊与医疗媒体的公开信息。",
    },
    BuiltinGroup {
        id: "grp_builtin_tech",
        name: "科技媒体",
        description: "综合科技媒体与工程媒体的深度报道。",
    },
    BuiltinGroup {
        id: "grp_builtin_access",
        name: "自有与接入",
        description: "系统自身的推送入口、状态检索与公众号接入，默认停用或按需开启。",
    },
];

/// 按标识取内置分类；用户自建分类返回 `None`。
pub fn builtin_group(id: &str) -> Option<&'static BuiltinGroup> {
    BUILTIN_GROUPS.iter().find(|group| group.id == id)
}

// ---------------------------------------------------------------------------
// 内置信源归属
// ---------------------------------------------------------------------------
// 初始数据不只在仓库的 YAML 里，也在程序这一侧：`slug → 内置分类` 的清单既回答
// 「哪些信源是自带的」，又作为出厂基线的指纹来源。新增或迁移内置信源时同步这里，
// 出厂基线才会随版本重建，而不是被首次安装时的副本长期冻住。

const AI_LABS: &str = "grp_builtin_ai_labs";
const AI_RESEARCH: &str = "grp_builtin_ai_research";
const AI_MEDIA: &str = "grp_builtin_ai_media";
const FINANCE: &str = "grp_builtin_finance";
const HEALTH: &str = "grp_builtin_health";
const TECH: &str = "grp_builtin_tech";
const ACCESS: &str = "grp_builtin_access";

/// 一条内置信源的归属。
pub struct BuiltinSource {
    pub slug: &'static str,
    pub group_id: &'static str,
}

const fn source(slug: &'static str, group_id: &'static str) -> BuiltinSource {
    BuiltinSource { slug, group_id }
}

/// 随仓库提供的全部信源及其内置分类，与 `config/sources/` 一一对应（测试护栏）。
pub const BUILTIN_SOURCES: &[BuiltinSource] = &[
    source("aws-machine-learning", AI_LABS),
    source("deepseek-updates", AI_LABS),
    source("github-ai", AI_LABS),
    source("google-ai-blog", AI_LABS),
    source("google-deepmind", AI_LABS),
    source("google-research", AI_LABS),
    source("hugging-face", AI_LABS),
    source("microsoft-research", AI_LABS),
    source("mimo-blog", AI_LABS),
    source("mistral", AI_LABS),
    source("nvidia-blog", AI_LABS),
    source("openai-news", AI_LABS),
    source("arxiv-cs-ai", AI_RESEARCH),
    source("berkeley-ai-research", AI_RESEARCH),
    source("nature-machine-intelligence", AI_RESEARCH),
    source("ars-technica-ai", AI_MEDIA),
    source("import-ai", AI_MEDIA),
    source("latent-space", AI_MEDIA),
    source("mit-technology-review-ai", AI_MEDIA),
    source("simon-willison", AI_MEDIA),
    source("techcrunch-ai", AI_MEDIA),
    source("the-decoder", AI_MEDIA),
    source("the-verge-ai", AI_MEDIA),
    source("ecb-press", FINANCE),
    source("federal-reserve-press", FINANCE),
    source("marketwatch-top-stories", FINANCE),
    source("sec-press-releases", FINANCE),
    source("fda-press-releases", HEALTH),
    source("jama-current", HEALTH),
    source("nature-medicine", HEALTH),
    source("nejm-current", HEALTH),
    source("stat-news", HEALTH),
    source("the-lancet", HEALTH),
    source("ieee-spectrum", TECH),
    source("mit-technology-review", TECH),
    source("wired", TECH),
    source("inbound-items", ACCESS),
    source("status-search", ACCESS),
    source("wechat-account", ACCESS),
];

/// 内置信源应归属的内置分类；用户自建信源返回 `None`。
pub fn builtin_source_group(slug: &str) -> Option<&'static str> {
    BUILTIN_SOURCES
        .iter()
        .find(|source| source.slug == slug)
        .map(|source| source.group_id)
}

/// 初始数据的声明形态版本：本版本把内置信源的节律从 `schedule` 迁到 2-3 个
/// 默认关闭的示例计划（其余信源需自建计划）。形态变化必须让旧出厂基线重算，
/// 否则恢复出厂又会带回旧的「隐式计划」。
const INITIAL_DATA_REVISION: u64 = 3;

/// 初始数据指纹：内置分类与信源清单的任何变化都会改变它。
///
/// 出厂基线用它判断自己是否还代表当前版本：指纹不同就整体重建，避免把首次
/// 安装时的旧配置（少几个信源、没有分类归属）一直当成「出厂设置」。
pub fn builtin_fingerprint() -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    fn feed(hash: &mut u64, text: &str) {
        for byte in text.as_bytes() {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(PRIME);
        }
    }
    let mut hash = OFFSET ^ INITIAL_DATA_REVISION;
    for group in BUILTIN_GROUPS {
        feed(&mut hash, group.id);
        feed(&mut hash, group.name);
        feed(&mut hash, group.description);
    }
    for source in BUILTIN_SOURCES {
        feed(&mut hash, source.slug);
        feed(&mut hash, source.group_id);
    }
    hash
}

/// 公开入口：摘掉内置信源遗留的 `schedule`（不新建计划）。
///
/// 实现属于配置迁移，详见 [`crate::migration::strip_builtin_schedules`]。
pub fn strip_builtin_schedules(
    dir: &std::path::Path,
    options: &crate::LoadOptions,
    slugs: &[&str],
) -> Result<usize, crate::Error> {
    crate::migration::strip_builtin_schedules(dir, options, slugs)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use serde_json::json;

    fn source(accessor: &str, locator: Value, pipeline: Value) -> Value {
        json!({
            "kind": "source",
            "access": { "accessor": accessor, "locator": locator },
            "pipeline": pipeline,
        })
    }

    #[test]
    fn builtin_groups_are_unique_and_named() {
        let mut seen = std::collections::BTreeSet::new();
        for group in BUILTIN_GROUPS {
            assert!(seen.insert(group.id), "内置分类标识重复: {}", group.id);
            assert!(!group.name.trim().is_empty(), "{} 缺名称", group.id);
            assert!(!group.description.trim().is_empty(), "{} 缺说明", group.id);
        }
    }

    /// 仓库内置信源必须与清单一一对应：不漏、不多，且分类归属与清单一致。
    #[test]
    fn shipped_sources_match_builtin_manifest() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../config")
            .canonicalize()
            .expect("仓库 config 目录应存在");
        let entries = crate::author::entries(&dir, "source").expect("读取内置信源");
        assert!(!entries.is_empty(), "内置信源不应为空");
        // 仓库里的初始数据必须整库通过校验：信源 + 计划 + 七个内置分类。
        crate::validate(&dir, &crate::LoadOptions::default()).expect("仓库配置应通过校验");
        let mut slugs: Vec<&str> = entries.iter().map(|entry| entry.slug.as_str()).collect();
        let mut listed: Vec<&str> = BUILTIN_SOURCES.iter().map(|source| source.slug).collect();
        slugs.sort_unstable();
        listed.sort_unstable();
        assert_eq!(
            slugs, listed,
            "内置信源清单应与 config/sources 目录一一对应；新增信源需同步 BUILTIN_SOURCES"
        );
        // 自带计划只留 2-3 个示例，全部默认关闭；信源不再带 schedule，未被示例
        // 计划覆盖的信源不参与调度，需用户自建计划。
        let plans = crate::author::entries(&dir, "plan").expect("读取内置计划");
        assert!(
            (2..=3).contains(&plans.len()),
            "自带计划只保留 2-3 个示例，实际 {} 个",
            plans.len()
        );
        let mut covered: std::collections::BTreeMap<&str, Vec<&str>> =
            std::collections::BTreeMap::new();
        for plan in &plans {
            assert_eq!(
                plan.doc["enabled"],
                json!(false),
                "{} 应默认关闭",
                plan.slug
            );
            let targets = plan.doc["targets"]
                .as_array()
                .unwrap_or_else(|| panic!("{} 缺少 targets", plan.slug));
            assert!(!targets.is_empty(), "{} 没有采集目标", plan.slug);
            for target in targets {
                let slug = target
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches("source/");
                assert!(
                    entries.iter().any(|entry| entry.slug == slug),
                    "{} 的计划引用了不存在的信源 {slug}",
                    plan.slug
                );
                covered.entry(slug).or_default().push(plan.slug.as_str());
            }
        }
        for entry in entries {
            let group_id = entry
                .doc
                .get("groupId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                builtin_group(group_id).is_some(),
                "{} 的分类不在内置清单里",
                entry.slug
            );
            assert_eq!(
                Some(group_id),
                builtin_source_group(&entry.slug),
                "信源 {} 的分类与清单不一致",
                entry.slug
            );
            assert!(
                entry.doc.get("schedule").is_none(),
                "{} 仍把节律写在信源里，应在计划中表达",
                entry.slug
            );
            if let Some(covering) = covered.get(entry.slug.as_str()) {
                assert_eq!(
                    covering.len(),
                    1,
                    "{} 被多个计划覆盖，会重复派发: {covering:?}",
                    entry.slug
                );
            }
        }
    }

    /// 迁移兜底：只把内置信源遗留的 schedule/回填额度摘掉，**不新建计划**。
    #[test]
    fn strip_builtin_schedules_only_strips() {
        let dir = tempfile::tempdir().unwrap();
        let options = crate::LoadOptions::default();
        std::fs::create_dir_all(dir.path().join("presets")).unwrap();
        for preset in crate::builtin_presets() {
            std::fs::write(
                dir.path().join("presets").join(preset.file_name),
                preset.source,
            )
            .unwrap();
        }
        std::fs::write(
            dir.path().join("system.yaml"),
            "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("accessors")).unwrap();
        std::fs::write(
            dir.path().join("accessors/rss.yaml"),
            "id: accessor/rss\nkind: accessor\nname: rss\nimplementation: builtin:rss\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
        )
        .unwrap();
        crate::author::create(
            dir.path(),
            &options,
            &crate::author::Draft {
                kind: "source".into(),
                slug: "legacy".into(),
                doc: json!({
                    "name": "Legacy",
                    "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
                    "pipeline": [],
                    "schedule": {"interval": "2h"},
                    "admission": {"initialBackfill": {"limit": 8, "maxAge": "365d"}},
                }),
            },
        )
        .unwrap();
        assert_eq!(
            strip_builtin_schedules(dir.path(), &options, &["legacy"]).expect("首次剥离"),
            1
        );
        // 只剥离、不补计划：自带示例计划之外不批量生成。
        assert!(
            crate::author::entries(dir.path(), "plan")
                .unwrap()
                .is_empty()
        );
        let source = crate::author::entries(dir.path(), "source").unwrap();
        assert!(source[0].doc.get("schedule").is_none());
        assert!(
            source[0]
                .doc
                .pointer("/admission/initialBackfill")
                .is_none()
        );
        assert_eq!(
            strip_builtin_schedules(dir.path(), &options, &["legacy"]).expect("幂等"),
            0
        );
    }

    #[test]
    fn builtin_fingerprint_is_deterministic() {
        assert_ne!(builtin_fingerprint(), 0);
        assert_eq!(builtin_fingerprint(), builtin_fingerprint());
    }

    #[test]
    fn derive_matches_the_ordered_rules() {
        assert_eq!(
            derive(&source(
                "accessor/rss",
                json!({"url": "u"}),
                json!([{"run": ["adapter/feed-items"]}])
            )),
            SourceType::Feed
        );
        assert_eq!(
            derive(&source("accessor/search", json!({"query": "q"}), json!([]))),
            SourceType::Search
        );
        assert_eq!(
            derive(&source(
                "accessor/bundle-list",
                json!({"url": "u"}),
                json!([])
            )),
            SourceType::Web
        );
        assert_eq!(
            derive(&source(
                "accessor/push",
                json!({"channel": "items"}),
                json!([])
            )),
            SourceType::Push
        );
        // paid-api 带 query → search，无 query → api。
        let with_query = json!({
            "kind": "source",
            "access": { "accessor": "accessor/paid-api", "locator": {"endpoint": "e"}, "spec": {"params": {"query": "from:x"}} },
            "pipeline": [{"run": ["adapter/json-list"]}],
        });
        assert_eq!(derive(&with_query), SourceType::Search);
        let no_query = json!({
            "kind": "source",
            "access": { "accessor": "accessor/paid-api", "locator": {"endpoint": "e"}, "spec": {"method": "POST"} },
            "pipeline": [{"run": ["adapter/json-list"]}],
        });
        assert_eq!(derive(&no_query), SourceType::Api);
        // http：含 feed-items → feed，否则 web。
        assert_eq!(
            derive(&source(
                "accessor/http",
                json!({"url": "u"}),
                json!([{"run": ["adapter/feed-items"]}])
            )),
            SourceType::Feed
        );
        assert_eq!(
            derive(&source(
                "accessor/http",
                json!({"url": "u"}),
                json!([{"run": ["adapter/html-list"]}])
            )),
            SourceType::Web
        );
    }

    #[test]
    fn the_shipped_sources_all_validate() {
        // 仓库自带的信源覆盖六个代表形状，都应零问题通过矩阵。
        let cases = [
            source(
                "accessor/rss",
                json!({"url": "u"}),
                json!([{"run": ["adapter/feed-items"]}]),
            ),
            source(
                "accessor/bundle-list",
                json!({"url": "u"}),
                json!([{"run": ["adapter/json-list"]}, {"run": ["adapter/page-fields"]}]),
            ),
            json!({
                "kind": "source",
                "access": {"accessor": "accessor/paid-api", "locator": {"endpoint": "e"}, "spec": {"params": {"query": "x"}}},
                "pipeline": [{"run": ["adapter/json-list"]}],
                "paid": {"budget": {"requests": 48, "window": "1d"}},
            }),
            source(
                "accessor/push",
                json!({"channel": "items"}),
                json!([{"run": ["adapter/json-list"]}]),
            ),
            source(
                "accessor/http",
                json!({"url": "u"}),
                json!([{"run": ["adapter/html-list"]}]),
            ),
        ];
        for case in cases {
            assert!(
                validate(&case).is_empty(),
                "不应有硬问题: {:?}",
                validate(&case)
            );
        }
    }

    #[test]
    fn unknown_value_and_mismatch_are_rejected() {
        let mut doc = source(
            "accessor/rss",
            json!({"url": "u"}),
            json!([{"run": ["adapter/feed-items"]}]),
        );
        doc["sourceType"] = json!("nonsense");
        assert!(
            validate(&doc)
                .iter()
                .any(|issue| issue.contains("未知 sourceType"))
        );

        // 声明 web 但形状是 rss → 不一致。
        let mut doc = source(
            "accessor/rss",
            json!({"url": "u"}),
            json!([{"run": ["adapter/feed-items"]}]),
        );
        doc["sourceType"] = json!("web");
        assert!(validate(&doc).iter().any(|issue| issue.contains("不一致")));
    }

    #[test]
    fn matrix_blocks_bad_combinations() {
        // feed 类塞入 page-fields 适配器。
        let mut doc = source(
            "accessor/http",
            json!({"url": "u"}),
            json!([{"run": ["adapter/feed-items"]}]),
        );
        doc["pipeline"] = json!([{"run": ["adapter/feed-items", "adapter/page-fields"]}]);
        doc["sourceType"] = json!("feed");
        assert!(
            validate(&doc)
                .iter()
                .any(|issue| issue.contains("不允许适配器"))
        );

        // feed 类声明 paid。
        let mut doc = source(
            "accessor/rss",
            json!({"url": "u"}),
            json!([{"run": ["adapter/feed-items"]}]),
        );
        doc["paid"] = json!({"budget": {"requests": 1, "window": "1d"}});
        assert!(validate(&doc).iter().any(|issue| issue.contains("paid")));
    }

    #[test]
    fn custom_is_an_escape_hatch() {
        let mut doc = source(
            "accessor/browser",
            json!({"url": "u"}),
            json!([{"run": ["adapter/html-list"]}]),
        );
        doc["sourceType"] = json!("custom");
        assert!(
            validate(&doc).is_empty(),
            "custom 应放行任意组合: {:?}",
            validate(&doc)
        );
    }

    #[test]
    fn missing_is_not_a_hard_error_but_is_inventory() {
        let doc = source(
            "accessor/rss",
            json!({"url": "u"}),
            json!([{"run": ["adapter/feed-items"]}]),
        );
        assert!(is_missing(&doc));
        assert!(validate(&doc).is_empty(), "缺失不产硬错误，用派生生效");
        assert_eq!(effective(&doc), SourceType::Feed);
    }

    #[test]
    fn registry_exposes_all_six_with_defaults() {
        let registry = registry_json();
        let items = registry["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        assert_eq!(
            plan_defaults(SourceType::Web)["trigger"]["interval"],
            json!("2h")
        );
        assert!(plan_defaults(SourceType::Push)["trigger"].is_null());
    }

    /// 表单可选项必须是校验允许集的子集：否则前端会渲染出"选了必被拒"的组合。
    #[test]
    fn form_accessors_stay_within_allowed() {
        for spec in &REGISTRY {
            let Some(form) = spec.form_accessors else {
                continue;
            };
            let allowed = spec
                .allowed_accessors
                .expect("声明 formAccessors 必须同时给上限");
            for item in form {
                assert!(
                    allowed.contains(item),
                    "{:?} 的 {item} 超出允许集",
                    spec.kind
                );
            }
        }
        assert_eq!(
            spec_of(SourceType::Search).form_accessors,
            Some(&["search"][..])
        );
        assert_eq!(
            spec_of(SourceType::Push).form_accessors,
            Some(&["push"][..])
        );
        // 注册表把子集原样下发，前端不自行推断。
        let registry = registry_json();
        let search = registry["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == json!("search"))
            .unwrap();
        assert_eq!(search["formAccessors"], json!(["search"]));
    }
}
