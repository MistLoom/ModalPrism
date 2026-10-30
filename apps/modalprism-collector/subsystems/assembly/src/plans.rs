//! 从配置快照生成信源计划。推送信源不进入 tick。

use std::time::Duration;

use config::Snapshot;
use planner::SourcePlan;
use serde_json::{Value, json};
use strata_model::{Locator, SourceId, content_hash};

pub const YIELD_DROP_RATIO: f64 = 0.3;
pub const FAILURE_THRESHOLD: u32 = 5;

pub fn plans_of(snapshot: &Snapshot) -> Vec<SourcePlan> {
    let mut plans = Vec::new();
    let mut covered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // 主路径：遍历一等计划，逐 target 把信源静态声明与计划节律/限制 join 成 SourcePlan。
    for (plan_id, plan_module) in &snapshot.modules {
        if plan_module.get("kind").and_then(Value::as_str) != Some("plan") {
            continue;
        }
        let targets = plan_module
            .get("targets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for target in targets.iter().filter_map(Value::as_str) {
            covered.insert(target.to_string());
            let Some(source_module) = snapshot.modules.get(target) else {
                continue;
            };
            if source_module.get("kind").and_then(Value::as_str) != Some("source") {
                continue;
            }
            if let Some(plan) = join_plan(snapshot, plan_id, plan_module, target, source_module) {
                plans.push(plan);
            }
        }
    }
    // 兼容路径：尚未迁移、仍把 schedule 写在信源里的信源，就地合成同 id 的计划。
    // 迁移把 schedule 逐字搬进 plan.trigger 后，两路解析出的取值一致，计划视图与指纹不变。
    for (id, source_module) in &snapshot.modules {
        if source_module.get("kind").and_then(Value::as_str) != Some("source")
            || covered.contains(id)
            || source_module.get("schedule").is_none()
        {
            continue;
        }
        let plan_id = format!("plan/{}", slug_of(id));
        if let Some(plan) = legacy_plan(snapshot, id, source_module, &plan_id) {
            plans.push(plan);
        }
    }
    plans.sort_by(|left, right| {
        (
            left.plan_id.clone().unwrap_or_default(),
            left.source_id.as_str().to_string(),
        )
            .cmp(&(
                right.plan_id.clone().unwrap_or_default(),
                right.source_id.as_str().to_string(),
            ))
    });
    plans
}

fn slug_of(id: &str) -> &str {
    id.split('/').next_back().unwrap_or(id)
}

/// 从一等计划与信源 join 出一个 SourcePlan：节律/限制取自计划，取数形态取自信源。
fn join_plan(
    snapshot: &Snapshot,
    plan_id: &str,
    plan_module: &Value,
    source_id: &str,
    source_module: &Value,
) -> Option<SourcePlan> {
    let trigger = plan_module.get("trigger")?;
    let cadence = normalize_cadence(trigger);
    let quota = plan_module.get("quota").cloned().unwrap_or(Value::Null);
    let one_shot = trigger.get("kind").and_then(Value::as_str) == Some("oneShot");
    let source_enabled = source_enabled(source_module);
    let plan_enabled = plan_module
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let queue_override = plan_module.get("queue").and_then(Value::as_str);
    let stop_policy = plan_module
        .get("stopPolicy")
        .cloned()
        .unwrap_or(Value::Null);
    build_plan(
        snapshot,
        source_id,
        source_module,
        Some(plan_id),
        &cadence,
        &quota,
        &stop_policy,
        source_enabled && plan_enabled,
        one_shot,
        queue_override,
    )
}

/// 未迁移信源：schedule 当节律、admission.initialBackfill 当 quota，合成等价计划。
fn legacy_plan(
    snapshot: &Snapshot,
    source_id: &str,
    source_module: &Value,
    plan_id: &str,
) -> Option<SourcePlan> {
    let schedule = source_module.get("schedule")?;
    let cadence = normalize_cadence(schedule);
    let backfill = source_module.pointer("/admission/initialBackfill");
    let quota = match backfill {
        Some(b) => json!({
            "perRunItems": b.get("limit"),
            "maxAge": b.get("maxAge"),
        }),
        None => Value::Null,
    };
    build_plan(
        snapshot,
        source_id,
        source_module,
        Some(plan_id),
        &cadence,
        &quota,
        &Value::Null,
        source_enabled(source_module),
        false,
        None,
    )
}

/// 把节律字段收敛成统一形态（interval/floor/ceiling/adaptive/expectedPeriod/batchKey），
/// 使 trigger 与旧 schedule 无论来自哪条路径，同值即同哈希、同指纹。
fn normalize_cadence(value: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "interval",
        "floor",
        "ceiling",
        "adaptive",
        "expectedPeriod",
        "batchKey",
    ] {
        if let Some(field) = value.get(key) {
            out.insert(key.to_string(), field.clone());
        }
    }
    Value::Object(out)
}

fn source_enabled(module: &Value) -> bool {
    module
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

#[allow(clippy::too_many_arguments)]
fn build_plan(
    snapshot: &Snapshot,
    source_id_str: &str,
    module: &Value,
    plan_id: Option<&str>,
    cadence: &Value,
    quota: &Value,
    stop_policy: &Value,
    enabled: bool,
    one_shot: bool,
    queue_override: Option<&str>,
) -> Option<SourcePlan> {
    let locator = locator_of(module.get("access")?.get("locator")?)?;
    if matches!(locator, Locator::PushChannel { .. }) {
        return None;
    }
    let accessor = module
        .pointer("/access/accessor")
        .and_then(Value::as_str)?
        .to_string();
    let accessor_module = snapshot.modules.get(&accessor)?;
    let implementation = accessor_module
        .get("implementation")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !implementation.starts_with("builtin:") {
        return None;
    }
    let source_id = SourceId::new(source_id_str).ok()?;
    let floor = duration_at(cadence.get("floor"), Duration::from_secs(30 * 60))?;
    let interval = duration_at(cadence.get("interval"), floor)?;
    let ceiling = duration_at(cadence.get("ceiling"), interval)?;
    let timeout = duration_at(
        accessor_module.pointer("/limits/timeout"),
        Duration::from_secs(30),
    )?;
    let expected_period = match cadence.get("expectedPeriod") {
        None => None,
        Some(value) => Some(duration_at(Some(value), Duration::ZERO)?),
    };
    let spec = access_spec(accessor_module, module);
    let bytes = plan_hash_input(module, cadence, quota);
    let budget = budget_of(module);
    let daily_limit = cap_of(module, "daily");
    let per_run_items = quota.get("perRunItems").and_then(Value::as_u64);
    let total_items = quota.get("totalItems").and_then(Value::as_u64);
    let on_budget_exhausted = stop_policy
        .get("onBudgetExhausted")
        .and_then(Value::as_str)
        .unwrap_or("skip")
        .to_string();
    let failure_threshold = stop_policy
        .get("onFailureThreshold")
        .and_then(Value::as_u64)
        .map(|value| value as u32)
        .unwrap_or(FAILURE_THRESHOLD);
    let queue = queue_override
        .map(queue_from_label)
        .unwrap_or_else(|| queue_of(implementation, module));
    // v3 R18：停滞升级阈值来自信源类型（per-plan 数据，采集侧不按类型分支）。
    let stagnation_cycles =
        config::source_type::stagnation_cycles(config::source_type::effective(module));
    Some(SourcePlan {
        source_id,
        plan_id: plan_id.map(str::to_string),
        locator,
        accessor,
        queue,
        spec,
        generation: snapshot.generation,
        config_hash: content_hash(&bytes),
        migration_generation: migration_generation(),
        timeout,
        paid: module.get("paid").is_some(),
        budget_requests: budget.map(|(requests, _)| requests),
        budget_window: budget.map(|(_, window)| window).unwrap_or(Duration::ZERO),
        adaptive: cadence
            .get("adaptive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        interval,
        floor,
        ceiling,
        batch_key: cadence
            .get("batchKey")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        enabled,
        expected_period,
        yield_drop_ratio: YIELD_DROP_RATIO,
        failure_threshold,
        daily_limit,
        one_shot,
        per_run_items,
        total_items,
        on_budget_exhausted,
        stagnation_cycles,
    })
}

fn queue_from_label(label: &str) -> strata_model::QueueName {
    match label {
        "paid" => strata_model::QueueName::Paid,
        "render" => strata_model::QueueName::Render,
        "push" => strata_model::QueueName::Push,
        _ => strata_model::QueueName::Free,
    }
}

/// `config_hash` 覆盖影响抓取计划的字段（access / 节律 / quota / paid / admission），
/// **排除** pipeline 与 binding（R11.1 / H1）。节律与 quota 取自解析后的统一形态，
/// 使「schedule 写在信源」与「迁移进计划 trigger/quota」两路得到同一哈希。
fn plan_hash_input(module: &Value, cadence: &Value, quota: &Value) -> Vec<u8> {
    let mut admission = module
        .get("admission")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if let Some(map) = admission.as_object_mut() {
        map.remove("initialBackfill");
    }
    let mut relevant = serde_json::Map::new();
    for key in ["access", "paid"] {
        if let Some(value) = module.get(key) {
            relevant.insert(key.to_string(), value.clone());
        }
    }
    relevant.insert("cadence".to_string(), cadence.clone());
    relevant.insert("quota".to_string(), quota.clone());
    relevant.insert("admission".to_string(), admission);
    serde_json::to_vec(&Value::Object(relevant)).unwrap_or_else(|_| b"null".to_vec())
}

/// 迁移代标记：环境变量置位时，本轮规划把全部信源标记为迁移代，规划器跳过
/// config_hash 变化触发的游标重置（R11.3）。仅在迁移那一代显式开启，用完即撤。
fn migration_generation() -> bool {
    std::env::var("MODALPRISM_CONFIG_MIGRATION_GENERATION")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn access_spec(accessor_module: &Value, source: &Value) -> Value {
    let mut base = match accessor_module.get("spec") {
        Some(Value::Object(map)) => map.clone(),
        _ => Default::default(),
    };
    if let Some(Value::Object(overlay)) = source.pointer("/access/spec") {
        for (key, value) in overlay {
            base.insert(key.clone(), value.clone());
        }
    }
    Value::Object(base)
}

fn locator_of(value: &Value) -> Option<Locator> {
    if let Some(url) = value.get("url").and_then(Value::as_str) {
        return Some(Locator::Url {
            url: url.to_string(),
        });
    }
    if let Some(query) = value.get("query").and_then(Value::as_str) {
        return Some(Locator::Query {
            query: query.to_string(),
        });
    }
    if let Some(endpoint) = value.get("endpoint").and_then(Value::as_str) {
        return Some(Locator::ApiEndpoint {
            endpoint: endpoint.to_string(),
        });
    }
    if let Some(channel) = value.get("channel").and_then(Value::as_str) {
        return Some(Locator::PushChannel {
            channel: channel.to_string(),
        });
    }
    None
}

fn cap_of(module: &Value, key: &str) -> Option<u64> {
    module
        .pointer(&format!("/admission/{key}/limit"))
        .and_then(Value::as_u64)
}

fn budget_of(module: &Value) -> Option<(u64, Duration)> {
    let requests = module.pointer("/paid/budget/requests")?.as_u64()?;
    let window = config::duration(module.pointer("/paid/budget/window")?.as_str()?)?;
    if requests == 0 || window.is_zero() {
        return None;
    }
    Some((requests, window))
}

fn queue_of(implementation: &str, module: &Value) -> strata_model::QueueName {
    if implementation == "builtin:browser" {
        strata_model::QueueName::Render
    } else if module.get("paid").is_some() {
        strata_model::QueueName::Paid
    } else {
        strata_model::QueueName::Free
    }
}

fn duration_at(value: Option<&Value>, default: Duration) -> Option<Duration> {
    let Some(value) = value else {
        return Some(default);
    };
    config::duration(value.as_str()?)
}

pub fn capacities() -> planner::Capacities {
    planner::Capacities {
        free: 4,
        paid: 1,
        render: 2,
        push: 4,
    }
}

pub fn snapshot_interval(snapshot: &Snapshot) -> Duration {
    duration_at(
        snapshot.system.pointer("/policy/state/snapshot_interval"),
        Duration::from_secs(15 * 60),
    )
    .unwrap_or(Duration::from_secs(15 * 60))
}

pub fn flush_interval(snapshot: &Snapshot) -> Duration {
    duration_at(
        snapshot.system.pointer("/policy/commit/flush_interval"),
        Duration::from_secs(30),
    )
    .unwrap_or(Duration::from_secs(30))
}

/// 迁移稳定性 dry-run 报告（R11.5）。
pub struct GateReport {
    /// 迁移后进入计划视图的信源数。
    pub sources: usize,
    /// 迁移将要新建的绑定数。
    pub bindings_added: usize,
    /// 迁移前后不一致的断言项；空即通过门禁。
    pub mismatches: Vec<String>,
}

impl GateReport {
    pub fn stable(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// 影响抓取计划的指纹：除 `config_hash` 外，再钉住由 access/schedule/accessor 派生的
/// 关键字段，确保迁移不改动派发形态（R11.5「计划视图不变」）。
fn plan_fingerprint(plan: &SourcePlan) -> String {
    format!(
        "hash={};accessor={};queue={:?};interval={}ms;floor={}ms;ceiling={}ms;paid={};budget={}@{}ms;enabled={};adaptive={};batch={:?};daily={:?};timeout={}ms;expected={:?}",
        plan.config_hash,
        plan.accessor,
        plan.queue,
        plan.interval.as_millis(),
        plan.floor.as_millis(),
        plan.ceiling.as_millis(),
        plan.paid,
        plan.budget_requests.unwrap_or(0),
        plan.budget_window.as_millis(),
        plan.enabled,
        plan.adaptive,
        plan.batch_key,
        plan.daily_limit,
        plan.timeout.as_millis(),
        plan.expected_period.map(|d| d.as_millis()),
    )
}

/// R11.5：迁移前后计划视图与 `config_hash` 稳定性 dry-run 门禁。
///
/// 在 `staging` 目录（调用方准备的空临时目录，须为 `dir` 的可写副本之外）里复制配置、
/// 应用迁移、重新加载并逐条比对信源计划指纹。**不触碰 `dir` 本身**。任一条不一致即
/// 记入 `mismatches`；生产迁移前必须先跑本门禁且 `stable() == true`（迁移代标记 R11.3）。
pub fn dry_run_migration_gate(
    dir: &std::path::Path,
    staging: &std::path::Path,
) -> Result<GateReport, String> {
    let options = config::LoadOptions::default();
    let before = config::validate(dir, &options).map_err(|error| error.to_string())?;
    let before_index: std::collections::BTreeMap<String, String> = plans_of(&before)
        .iter()
        .map(|plan| (plan.source_id.as_str().to_string(), plan_fingerprint(plan)))
        .collect();
    let sources_before = before_index.len();

    copy_tree(dir, staging).map_err(|error| format!("复制配置副本失败: {error}"))?;
    let plan = config::apply_migration(staging, &options).map_err(|error| error.to_string())?;
    let after = config::validate(staging, &options).map_err(|error| error.to_string())?;
    let after_index: std::collections::BTreeMap<String, String> = plans_of(&after)
        .iter()
        .map(|plan| (plan.source_id.as_str().to_string(), plan_fingerprint(plan)))
        .collect();

    let mut mismatches = Vec::new();
    for (id, fingerprint) in &before_index {
        match after_index.get(id) {
            None => mismatches.push(format!("信源 {id} 迁移后从计划视图消失")),
            Some(after_print) => {
                if after_print != fingerprint {
                    mismatches.push(format!("信源 {id} 迁移前后计划指纹不一致"));
                }
            }
        }
    }
    for id in after_index.keys() {
        if !before_index.contains_key(id) {
            mismatches.push(format!("信源 {id} 迁移后新增（非预期）"));
        }
    }
    // 信源总数守恒（除文档列出的预期项）；此处迁移不增删信源。
    if sources_before != after_index.len() && mismatches.is_empty() {
        mismatches.push(format!(
            "计划信源数变化: {} → {}",
            sources_before,
            after_index.len()
        ));
    }
    Ok(GateReport {
        sources: after_index.len(),
        bindings_added: plan.binding_count(),
        mismatches,
    })
}

/// 递归复制目录树。
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn paid_declaration_enters_the_paid_queue() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "accessor/paid-api".into(),
            json!({
                "kind": "accessor",
                "implementation": "builtin:paid-api",
                "limits": {"timeout": "60s"}
            }),
        );
        modules.insert(
            "source/status-search".into(),
            json!({
                "kind": "source",
                "enabled": false,
                "access": {
                    "accessor": "accessor/paid-api",
                    "locator": {"endpoint": "https://example.com/search"}
                },
                "paid": {"budget": {"requests": 48, "window": "1d"}},
                "schedule": {"interval": "30m"}
            }),
        );
        let snapshot = config::Snapshot {
            generation: 1,
            system: json!({}),
            modules,
            origins: BTreeMap::new(),
        };
        let plans = plans_of(&snapshot);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].queue, strata_model::QueueName::Paid);
        assert!(plans[0].paid);
        assert!(!plans[0].enabled);
        assert_eq!(plans[0].budget_requests, Some(48));
        assert_eq!(plans[0].budget_window, Duration::from_secs(86_400));
    }

    /// R11.1 / H1：把 pipeline 拆成 binding 引用不应翻转 `config_hash`。
    #[test]
    fn config_hash_ignores_pipeline_and_binding() {
        let access = json!({
            "accessor": "accessor/rss",
            "locator": {"url": "https://example.com/feed"}
        });
        let schedule = json!({"interval": "1h"});
        let admission = json!({"initialBackfill": {"limit": 8, "maxAge": "365d"}});

        // 迁移前：内联 pipeline + spec。
        let before = json!({
            "kind": "source",
            "access": access,
            "pipeline": [{"run": ["adapter/feed-items"], "spec": {"adapter/feed-items": {"items": "/a"}}}],
            "schedule": schedule,
            "admission": admission,
        });
        // 迁移后：pipeline 改为对 binding 的引用，pipeline/binding 细节被移出信源字节。
        let after = json!({
            "kind": "source",
            "access": access,
            "pipeline": [{"binding": "binding/feed-items"}],
            "schedule": schedule,
            "admission": admission,
        });
        let cadence = normalize_cadence(&schedule);
        assert_eq!(
            content_hash(&plan_hash_input(&before, &cadence, &Value::Null)),
            content_hash(&plan_hash_input(&after, &cadence, &Value::Null)),
            "pipeline/binding 变化不得影响 config_hash"
        );

        // 但抓取相关字段（access）变化必须翻转。
        let changed = json!({
            "kind": "source",
            "access": {"accessor": "accessor/rss", "locator": {"url": "https://other.example.com/feed"}},
            "pipeline": [{"binding": "binding/feed-items"}],
            "schedule": schedule,
            "admission": admission,
        });
        assert_ne!(
            content_hash(&plan_hash_input(&after, &cadence, &Value::Null)),
            content_hash(&plan_hash_input(&changed, &cadence, &Value::Null)),
            "access 变化必须翻转 config_hash"
        );
    }

    /// v3 S1：`sourceType` 是标签维度，不得进入 `plan_hash_input`。仅追加类型字段，
    /// `config_hash` 必须逐字不变——否则回填代会翻转全体哈希、触发调度重置。
    #[test]
    fn config_hash_ignores_source_type() {
        let base = json!({
            "kind": "source",
            "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
            "pipeline": [{"run": ["adapter/feed-items"]}],
            "schedule": {"interval": "1h"},
            "admission": {},
        });
        let mut typed = base.clone();
        typed["sourceType"] = json!("feed");
        let cadence = normalize_cadence(&base["schedule"]);
        assert_eq!(
            content_hash(&plan_hash_input(&base, &cadence, &Value::Null)),
            content_hash(&plan_hash_input(&typed, &cadence, &Value::Null)),
            "sourceType 变化不得影响 config_hash"
        );
    }

    /// R11.5：迁移前后计划视图与 config_hash 稳定；dry-run 门禁通过。
    #[test]
    fn dry_run_gate_is_stable_across_binding_split() {
        let dir = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        let report = dry_run_migration_gate(dir.path(), staging.path()).expect("dry-run 成功");
        assert!(report.stable(), "迁移门禁应通过: {:?}", report.mismatches);
        assert_eq!(report.sources, 1);
        assert_eq!(report.bindings_added, 1, "内联 spec 步骤应拆出一份绑定");
        // 原目录未被触碰：仍没有 bindings 目录。
        assert!(!dir.path().join("bindings").exists());
    }

    /// 阶段 2c：`schedule → plan` 迁移前后，规划视图逐字等价（兼容路径合成 == join 路径）。
    #[test]
    fn schedule_to_plan_migration_keeps_plan_view_equivalent() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        let before =
            config::validate(dir.path(), &config::LoadOptions::default()).expect("before 校验");
        let before_map: std::collections::BTreeMap<String, String> = plans_of(&before)
            .iter()
            .map(|plan| {
                (
                    format!("{:?}/{}", plan.plan_id, plan.source_id),
                    plan_fingerprint(plan),
                )
            })
            .collect();
        assert_eq!(
            before_map.len(),
            1,
            "兼容路径应为带 schedule 的信源合成一个计划"
        );

        let migrated = tempfile::tempdir().unwrap();
        copy_tree(dir.path(), migrated.path()).unwrap();
        let moved =
            config::migrate_schedules_to_plans(migrated.path(), &config::LoadOptions::default())
                .expect("schedule→plan 迁移应通过整库校验");
        assert_eq!(moved, 1);
        assert!(
            migrated.path().join("plans/demo.yaml").exists(),
            "迁移应生成同名计划文件"
        );
        let after =
            config::validate(migrated.path(), &config::LoadOptions::default()).expect("after 校验");
        let after_map: std::collections::BTreeMap<String, String> = plans_of(&after)
            .iter()
            .map(|plan| {
                (
                    format!("{:?}/{}", plan.plan_id, plan.source_id),
                    plan_fingerprint(plan),
                )
            })
            .collect();
        assert_eq!(
            before_map, after_map,
            "schedule→plan 迁移必须保持计划视图与 config_hash 逐字等价"
        );
    }

    /// 一信源多计划互不吞并：两条计划指向同一信源、不同节律，join 出两条独立 SourcePlan。
    #[test]
    fn multi_plan_on_one_source_are_independent() {
        let mut modules = std::collections::BTreeMap::new();
        modules.insert(
            "accessor/rss".into(),
            json!({"kind": "accessor", "implementation": "builtin:rss", "limits": {"timeout": "45s"}}),
        );
        modules.insert(
            "source/demo".into(),
            json!({
                "kind": "source",
                "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
                "pipeline": [],
                "admission": {},
            }),
        );
        modules.insert(
            "plan/fast".into(),
            json!({"kind": "plan", "targets": ["source/demo"], "trigger": {"kind": "recurring", "interval": "1h"}}),
        );
        modules.insert(
            "plan/slow".into(),
            json!({"kind": "plan", "targets": ["source/demo"], "trigger": {"kind": "recurring", "interval": "6h"}}),
        );
        let snapshot = config::Snapshot {
            generation: 1,
            system: json!({}),
            modules,
            origins: std::collections::BTreeMap::new(),
        };
        let plans = plans_of(&snapshot);
        assert_eq!(plans.len(), 2, "同一信源的两个计划应各自成一条");
        let keys: std::collections::BTreeSet<String> =
            plans.iter().map(|plan| plan.dedup_key()).collect();
        assert_eq!(keys.len(), 2, "去重键带计划维度，互不吞并");
        assert!(plans.iter().all(|plan| plan.plan_id.is_some()));
    }

    /// R11.5：对**真实 `config/` 全量信源**做 dry-run，断言迁移前后计划视图与 config_hash 稳定。
    ///
    /// 复制整份 config 到临时副本、应用迁移、重新加载并逐信源比对；不触碰真实 `config/`。
    #[test]
    fn real_config_dry_run_gate_is_stable() {
        let config_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../config")
            .canonicalize()
            .expect("仓库 config 目录应存在");
        if !config_dir.join("system.yaml").exists() {
            panic!("未找到 config/system.yaml，真实校验前置条件不成立");
        }
        let staging = tempfile::tempdir().unwrap();
        // 先隔离确认：真实 config 本身应可加载（before 校验）。
        let before = config::validate(&config_dir, &config::LoadOptions::default());
        if let Err(error) = &before {
            panic!("真实 config 基线校验失败（非迁移引起）: {error}");
        }
        let report = dry_run_migration_gate(&config_dir, staging.path())
            .expect("真实 config dry-run 不应报错");
        eprintln!(
            "real config dry-run: sources={} bindingsAdded={} mismatches={:?}",
            report.sources, report.bindings_added, report.mismatches
        );
        assert!(
            report.stable(),
            "生产信源迁移前后计划必须稳定: {:?}",
            report.mismatches
        );
    }

    /// 为 dry-run 测试搭一份合法且含单适配器 spec 步骤的配置。
    fn write_fixture(dir: &std::path::Path) {
        let preset = config::builtin_presets()
            .into_iter()
            .find(|item| item.id == "preset/standard")
            .expect("内置预设");
        std::fs::create_dir_all(dir.join("presets")).unwrap();
        std::fs::write(dir.join("presets/standard.yaml"), preset.source).unwrap();
        std::fs::write(
            dir.join("system.yaml"),
            "id: system\nkind: system\nname: s\npolicy:\n  preset: standard\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("accessors")).unwrap();
        std::fs::write(
            dir.join("accessors/http.yaml"),
            "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("adapters")).unwrap();
        std::fs::write(
            dir.join("adapters/json-list.yaml"),
            "id: adapter/json-list\nkind: adapter\nname: JSON\nimplementation: builtin:json-list\nrole: search\nconsumes:\n  - {kind: raw, part_role: document, media_type: application/json}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: exponential\n    base: 1s\n    max: 30s\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("sources")).unwrap();
        std::fs::write(
            dir.join("sources/demo.yaml"),
            "id: source/demo\nkind: source\nname: Demo\naccess:\n  accessor: accessor/http\n  locator:\n    url: https://example.com/feed\npipeline:\n  - run:\n      - adapter/json-list\n    spec:\n      adapter/json-list:\n        items: /data\n        url: link\nschedule:\n  interval: 1h\nadmission: {}\n",
        )
        .unwrap();
    }
}
