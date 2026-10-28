//! 从配置快照生成信源计划。推送信源不进入 tick。

use std::time::Duration;

use config::Snapshot;
use planner::SourcePlan;
use serde_json::Value;
use strata_model::{Locator, SourceId, content_hash};

pub const YIELD_DROP_RATIO: f64 = 0.3;
pub const FAILURE_THRESHOLD: u32 = 5;

pub fn plans_of(snapshot: &Snapshot) -> Vec<SourcePlan> {
    snapshot
        .modules
        .iter()
        .filter_map(|(id, module)| one_plan(snapshot, id, module))
        .collect()
}

fn one_plan(snapshot: &Snapshot, id: &str, module: &Value) -> Option<SourcePlan> {
    if module.get("kind").and_then(Value::as_str) != Some("source") {
        return None;
    }
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
    let source_id = SourceId::new(id).ok()?;
    let schedule = module.get("schedule").cloned().unwrap_or(Value::Null);
    let floor = duration_at(schedule.get("floor"), Duration::from_secs(30 * 60))?;
    let interval = duration_at(schedule.get("interval"), floor)?;
    let ceiling = duration_at(schedule.get("ceiling"), interval)?;
    let timeout = duration_at(
        accessor_module.pointer("/limits/timeout"),
        Duration::from_secs(30),
    )?;
    let expected_period = match schedule.get("expectedPeriod") {
        None => None,
        Some(value) => Some(duration_at(Some(value), Duration::ZERO)?),
    };
    let spec = access_spec(accessor_module, module);
    let bytes = plan_hash_input(module);
    let budget = budget_of(module);
    let daily_limit = cap_of(module, "daily");
    Some(SourcePlan {
        source_id,
        locator,
        accessor,
        queue: queue_of(implementation, module),
        spec,
        generation: snapshot.generation,
        config_hash: content_hash(&bytes),
        migration_generation: migration_generation(),
        timeout,
        paid: module.get("paid").is_some(),
        budget_requests: budget.map(|(requests, _)| requests),
        budget_window: budget.map(|(_, window)| window).unwrap_or(Duration::ZERO),
        adaptive: schedule
            .get("adaptive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        interval,
        floor,
        ceiling,
        batch_key: schedule
            .get("batchKey")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        enabled: module
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        expected_period,
        yield_drop_ratio: YIELD_DROP_RATIO,
        failure_threshold: FAILURE_THRESHOLD,
        daily_limit,
    })
}

/// `config_hash` 只覆盖影响抓取计划的字段（access / schedule / admission / paid），
/// **排除** pipeline 与 binding（R11.1 / H1）。
///
/// 把内联 pipeline 拆到独立 binding 后，信源字节会变；若哈希仍吃整份模块，全体
/// `config_hash` 会一次性翻转，规划器据此判定「全部配置变更」，触发批量重排与自适应
/// 调度状态重置——与 N2/N3 冲突。字段序由 serde_json 有序映射固定，序列化即规范化。
fn plan_hash_input(module: &Value) -> Vec<u8> {
    let mut relevant = serde_json::Map::new();
    for key in ["access", "schedule", "admission", "paid"] {
        if let Some(value) = module.get(key) {
            relevant.insert(key.to_string(), value.clone());
        }
    }
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
        assert_eq!(
            content_hash(&plan_hash_input(&before)),
            content_hash(&plan_hash_input(&after)),
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
            content_hash(&plan_hash_input(&after)),
            content_hash(&plan_hash_input(&changed)),
            "access 变化必须翻转 config_hash"
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
            dir.join("accessors/rss.yaml"),
            "id: accessor/rss\nkind: accessor\nname: RSS\nimplementation: builtin:rss\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
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
            "id: source/demo\nkind: source\nname: Demo\naccess:\n  accessor: accessor/rss\n  locator:\n    url: https://example.com/feed\npipeline:\n  - run:\n      - adapter/json-list\n    spec:\n      adapter/json-list:\n        items: /data\n        url: link\nschedule:\n  interval: 1h\nadmission: {}\n",
        )
        .unwrap();
    }
}
