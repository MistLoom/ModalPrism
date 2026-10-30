//! 内联 pipeline → 独立 binding 的迁移（R9 / R11.3 / R11.4）。
//!
//! 迁移把信源里「命令式装配」的 pipeline 步骤（单适配器 + `spec` 参数）拆成对 `binding`
//! 的引用，参数原样搬进 `config/bindings/*.yaml`。搬移**不改动取值**，因此：
//! - `config_hash` 稳定：R11.1 已把哈希输入收窄到 access/schedule/admission/paid，pipeline/binding 不在其中；
//! - `params_hash` 稳定：R11.2 规范化字段序，逻辑等值 ⇒ 哈希等值，历史 facet 不被误判重解析（R11.4）。
//!
//! 保守边界（不迁移，保持内联，运行期照常工作）：
//! - 零参数步骤（`run: [adapter]` 无 `spec`）——正是 H6「默认推断仅适用零参数单阶段」的形态；
//! - 探针步骤（`run_if_accepts`）与多适配器步骤——语义更复杂，留待显式声明。
//!
//! 迁移一次性、单代完成：调用方在规划器侧以「迁移代」标记跳过游标重置（R11.3）。

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::author;
use crate::error::Error;
use crate::load::{LoadOptions, validate};
use crate::source_type;

/// 一份信源的迁移结果：改写后的信源正文 + 需要新建的绑定。
pub struct SourceMigration {
    pub source_file: String,
    pub new_source_doc: Value,
    /// `(绑定文件相对路径, 绑定文档, 绑定 id)`。
    pub bindings: Vec<(String, Value, String)>,
}

/// 全量迁移计划（dry-run 产物，不含任何写盘）。
pub struct MigrationPlan {
    pub sources: Vec<SourceMigration>,
}

impl MigrationPlan {
    /// 计划里将要新建的绑定总数，供 dry-run 汇报与门禁断言。
    pub fn binding_count(&self) -> usize {
        self.sources.iter().map(|item| item.bindings.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// 扫描 `dir/sources/*` 生成迁移计划，不落盘（R11.5 dry-run 的第一步）。
pub fn plan_migration(dir: &Path) -> Result<MigrationPlan, Error> {
    let entries = author::entries(dir, "source")?;
    let mut out = Vec::new();
    for entry in entries {
        let Some(migration) = plan_one(&entry) else {
            continue;
        };
        out.push(migration);
    }
    Ok(MigrationPlan { sources: out })
}

fn plan_one(entry: &crate::author::Entry) -> Option<SourceMigration> {
    let steps = entry.doc.get("pipeline").and_then(Value::as_array)?;
    let stem = entry.slug.clone();
    let mut new_steps = Vec::with_capacity(steps.len());
    let mut bindings = Vec::new();
    let mut changed = false;
    for (index, step) in steps.iter().enumerate() {
        let Some(obj) = step.as_object() else {
            new_steps.push(step.clone());
            continue;
        };
        // 探针步骤不迁移。
        if obj.contains_key("run_if_accepts") {
            new_steps.push(step.clone());
            continue;
        }
        let run = obj
            .get("run")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        // 只迁移「单适配器 + 带 spec 参数」的步骤。
        let [adapter] = run.as_slice() else {
            new_steps.push(step.clone());
            continue;
        };
        let params = obj
            .get("spec")
            .and_then(|spec| spec.get(*adapter))
            .cloned()
            .filter(Value::is_object);
        let Some(params) = params else {
            new_steps.push(step.clone());
            continue;
        };
        let binding_slug = format!("{stem}-{index}-{}", adapter_stem(adapter));
        let binding_id = format!("binding/{binding_slug}");
        let binding_file = format!("bindings/{binding_slug}.yaml");
        let binding_doc = json!({
            "id": binding_id,
            "kind": "binding",
            "name": format!("{} 绑定 #{index}", entry.slug),
            "adapter": adapter,
            "params": params,
        });
        bindings.push((binding_file, binding_doc, binding_id.clone()));
        // 改写：pipeline 步骤改为对 binding 的引用，保留 when/for/after。
        let mut reference = Map::new();
        for key in ["when", "for", "after"] {
            if let Some(value) = obj.get(key) {
                reference.insert(key.to_string(), value.clone());
            }
        }
        reference.insert("binding".into(), Value::String(binding_id));
        new_steps.push(Value::Object(reference));
        changed = true;
    }
    if !changed {
        return None;
    }
    let mut new_source_doc = entry.doc.clone();
    new_source_doc["pipeline"] = Value::Array(new_steps);
    Some(SourceMigration {
        source_file: entry.file.clone(),
        new_source_doc,
        bindings,
    })
}

fn adapter_stem(adapter: &str) -> String {
    adapter
        .split('/')
        .next_back()
        .unwrap_or(adapter)
        .to_string()
}

/// 由信源的 `schedule` + `admission.initialBackfill` 合成一份等价采集计划文档。
/// 节律逐字搬进 `trigger`，回填额度搬进 `quota`，其余信源字段（access/pipeline/paid/admission
/// 的 allowUrlPrefixes 等）留在信源。规划器兼容路径据同一份信源合成同 id 计划，取值一致。
fn synthesize_plan(entry: &crate::author::Entry) -> Option<Value> {
    let schedule = entry.doc.get("schedule")?.as_object()?;
    let mut trigger = Map::new();
    trigger.insert("kind".into(), json!("recurring"));
    for key in [
        "interval",
        "floor",
        "ceiling",
        "adaptive",
        "expectedPeriod",
        "batchKey",
    ] {
        if let Some(value) = schedule.get(key) {
            trigger.insert(key.to_string(), value.clone());
        }
    }
    let mut plan = Map::new();
    plan.insert("id".into(), json!(format!("plan/{}", entry.slug)));
    plan.insert("kind".into(), json!("plan"));
    let name = entry
        .doc
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&entry.slug)
        .to_string();
    plan.insert("name".into(), json!(format!("{name} · 采集计划")));
    plan.insert("targets".into(), json!([format!("source/{}", entry.slug)]));
    plan.insert("trigger".into(), Value::Object(trigger));
    if let Some(backfill) = entry
        .doc
        .pointer("/admission/initialBackfill")
        .and_then(Value::as_object)
    {
        let mut quota = Map::new();
        if let Some(limit) = backfill.get("limit") {
            quota.insert("perRunItems".into(), limit.clone());
        }
        if let Some(max_age) = backfill.get("maxAge") {
            quota.insert("maxAge".into(), max_age.clone());
        }
        if !quota.is_empty() {
            plan.insert("quota".into(), Value::Object(quota));
        }
    }
    if let Some(priority) = entry.doc.pointer("/schedule/priority") {
        plan.insert("priority".into(), priority.clone());
    }
    Some(Value::Object(plan))
}

/// 把信源里的 `schedule` 与 `admission.initialBackfill` 摘除，其余原样保留。
fn strip_schedule_fields(doc: &Value) -> Value {
    let mut new_doc = doc.clone();
    if let Some(map) = new_doc.as_object_mut() {
        map.remove("schedule");
        if let Some(admission) = map.get_mut("admission").and_then(Value::as_object_mut) {
            admission.remove("initialBackfill");
        }
    }
    new_doc
}

/// 采集节律迁出信源 → 一等计划（阶段 2c）。扫描 `dir/sources/*`，为仍带 `schedule` 的
/// 信源生成同名 `dir/plans/<slug>.yaml`，并从信源摘除 `schedule` 与 `admission.initialBackfill`。
/// 复用 [`apply_atomic`]：计划先、信源后，整库校验，全有或全无。幂等——无 `schedule` 的信源跳过。
pub fn migrate_schedules_to_plans(dir: &Path, options: &LoadOptions) -> Result<usize, Error> {
    let entries = author::entries(dir, "source")?;
    let mut writes: Vec<AtomicWrite> = Vec::new();
    for entry in &entries {
        if entry.doc.get("schedule").is_none() {
            continue;
        }
        let Some(plan_doc) = synthesize_plan(entry) else {
            continue;
        };
        let plan_file = format!("plans/{}.yaml", entry.slug);
        writes.push(AtomicWrite::write(plan_file, author::to_yaml(&plan_doc)));
        writes.push(AtomicWrite::write(
            entry.file.clone(),
            author::to_yaml(&strip_schedule_fields(&entry.doc)),
        ));
    }
    let moved = writes.len() / 2;
    if writes.is_empty() {
        return Ok(0);
    }
    apply_atomic(dir, options, &writes)?;
    Ok(moved)
}

/// 初始数据兜底：把内置信源里遗留的 `schedule` 摘掉，**不新建计划**。
///
/// 自带计划只保留少量示例（见 `config/plans`），节律归计划表达：没被示例计划
/// 覆盖的信源不参与调度，用户按需在计划页自建。老配置（或旧出厂基线）里仍带
/// `schedule` 的内置源如果原样留着，规划器会走兼容路径就地合成隐藏计划、按源
/// 启用状态自动采集——正是「没有计划却在采集」的来源，所以这里只做剥离。
pub(crate) fn strip_builtin_schedules(
    dir: &Path,
    options: &LoadOptions,
    slugs: &[&str],
) -> Result<usize, Error> {
    let entries = author::entries(dir, "source")?;
    let mut writes: Vec<AtomicWrite> = Vec::new();
    for entry in &entries {
        if !slugs.contains(&entry.slug.as_str()) || entry.doc.get("schedule").is_none() {
            continue;
        }
        writes.push(AtomicWrite::write(
            entry.file.clone(),
            author::to_yaml(&strip_schedule_fields(&entry.doc)),
        ));
    }
    let stripped = writes.len();
    if writes.is_empty() {
        return Ok(0);
    }
    apply_atomic(dir, options, &writes)?;
    Ok(stripped)
}

/// dry-run：不写盘，返回将要生成的计划文档（供迁移前后计划视图等价性断言）。
pub fn plan_schedule_migration(dir: &Path) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    for entry in author::entries(dir, "source")? {
        if let Some(plan) = synthesize_plan(&entry) {
            out.push(plan);
        }
    }
    Ok(out)
}

/// 缺失 `sourceType` 的信源清单 `(文件, 拟回填类型)`。回填前的清点用途，不作为校验硬错误。
pub fn plan_source_type_backfill(dir: &Path) -> Result<Vec<(String, String)>, Error> {
    let mut out = Vec::new();
    for entry in author::entries(dir, "source")? {
        if source_type::is_missing(&entry.doc) {
            out.push((
                entry.file.clone(),
                source_type::derive(&entry.doc).as_str().to_string(),
            ));
        }
    }
    Ok(out)
}

/// 声明回填（v3 R16）：为缺失 `sourceType` 的信源按派生规则写入显式类型，只增字段、
/// 不动其余取值（写出走 [`author::to_yaml`] 的既有键序约定，产物仍可读回）。复用
/// [`apply_atomic`]：全部改写一次性提交并整库校验，任一失败回滚到写入前状态。幂等——
/// 已带 `sourceType` 的信源跳过。`config_hash` 不受影响（v3 S1：节律输入不含 sourceType）。
pub fn backfill_source_types(dir: &Path, options: &LoadOptions) -> Result<usize, Error> {
    let mut writes: Vec<AtomicWrite> = Vec::new();
    for entry in author::entries(dir, "source")? {
        if !source_type::is_missing(&entry.doc) {
            continue;
        }
        let kind = source_type::derive(&entry.doc);
        let mut doc = entry.doc.clone();
        if let Some(map) = doc.as_object_mut() {
            map.insert("sourceType".to_string(), json!(kind.as_str()));
        }
        writes.push(AtomicWrite::write(
            entry.file.clone(),
            author::to_yaml(&doc),
        ));
    }
    let count = writes.len();
    if writes.is_empty() {
        return Ok(0);
    }
    apply_atomic(dir, options, &writes)?;
    Ok(count)
}

/// 应用迁移：先写绑定，再改写信源；任一失败把已写文件全部回滚（消解 H9 的部分成功）。
///
/// 落盘前用整库 `validate` 兜底：若计划产出的某份绑定畸形，本次不会留下半提交状态。
/// 复用 [`apply_atomic`] 作为跨文件原子原语——绑定先、信源后，全有或全无。
pub fn apply_migration(dir: &Path, options: &LoadOptions) -> Result<MigrationPlan, Error> {
    let plan = plan_migration(dir)?;
    if plan.is_empty() {
        return Ok(plan);
    }
    let mut writes: Vec<AtomicWrite> = Vec::new();
    // 先绑定后信源：顺序让「引用方」在被引用方就绪后才落地。
    for migration in &plan.sources {
        for (file, doc, _) in &migration.bindings {
            writes.push(AtomicWrite::write(file.clone(), author::to_yaml(doc)));
        }
    }
    for migration in &plan.sources {
        writes.push(AtomicWrite::write(
            migration.source_file.clone(),
            author::to_yaml(&migration.new_source_doc),
        ));
    }
    apply_atomic(dir, options, &writes)?;
    Ok(plan)
}

/// 一次跨文件原子写：`content=Some(text)` 落文件，`None` 删除文件。
pub struct AtomicWrite {
    pub rel_path: String,
    pub content: Option<String>,
}

impl AtomicWrite {
    pub fn write(rel_path: impl Into<String>, text: String) -> Self {
        Self {
            rel_path: rel_path.into(),
            content: Some(text),
        }
    }

    pub fn remove(rel_path: impl Into<String>) -> Self {
        Self {
            rel_path: rel_path.into(),
            content: None,
        }
    }
}

/// 跨文件原子作业（R5.4 / H9）：一次性写全部目标，整库校验；任一失败按原顺序反向回滚到写入前状态。
///
/// 调用方据此把「先 binding 后 source」这类多文件变更收敛成全有或全无，消解并发/崩溃导致的版本偏斜与半提交。
pub fn apply_atomic(
    dir: &Path,
    options: &LoadOptions,
    writes: &[AtomicWrite],
) -> Result<(), Error> {
    // 先记录每份目标的原文（或标记为新建），供失败回滚。
    let mut prior: Vec<(std::path::PathBuf, Option<String>)> = Vec::with_capacity(writes.len());
    for write in writes {
        let path = dir.join(&write.rel_path);
        let original = std::fs::read_to_string(&path).ok();
        prior.push((path, original));
    }
    for write in writes {
        let path = dir.join(&write.rel_path);
        match &write.content {
            Some(text) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&path, text).map_err(|source| Error::Read {
                    path: path.display().to_string(),
                    source,
                })?;
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    if let Err(error) = validate(dir, options) {
        // 反向回滚：新建文件删除、既有文件恢复原文。
        for (path, original) in prior.iter().rev() {
            match original {
                Some(text) => {
                    let _ = std::fs::write(path, text);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plans_single_adapter_spec_step_into_binding_reference() {
        let dir = tempfile::tempdir().unwrap();
        let sources = dir.path().join("sources");
        fs::create_dir_all(&sources).unwrap();
        fs::write(
            sources.join("demo.yaml"),
            "id: source/demo\nkind: source\nname: Demo\naccess:\n  accessor: accessor/http\n  locator:\n    url: https://example.com/feed\npipeline:\n  - run:\n      - adapter/json-list\n    spec:\n      adapter/json-list:\n        items: /data\n        url: link\nschedule:\n  interval: 1h\nadmission: {}\n",
        )
        .unwrap();

        let plan = plan_migration(dir.path()).unwrap();
        assert_eq!(plan.binding_count(), 1);
        let migration = &plan.sources[0];
        // 信源 pipeline 改为引用绑定，不再内嵌 spec。
        let step = &migration.new_source_doc["pipeline"][0];
        assert_eq!(step["binding"], json!("binding/demo-0-json-list"));
        assert!(step.get("spec").is_none(), "内联 spec 应被移入绑定");
        // 绑定承载同一取值，params 与原 spec 逐字相等（R11.2 保 params_hash）。
        let (_, binding, id) = &migration.bindings[0];
        assert_eq!(id, "binding/demo-0-json-list");
        assert_eq!(binding["adapter"], json!("adapter/json-list"));
        assert_eq!(binding["params"]["items"], json!("/data"));
        assert_eq!(binding["params"]["url"], json!("link"));
    }

    #[test]
    fn leaves_zero_param_and_probe_steps_inline() {
        let dir = tempfile::tempdir().unwrap();
        let sources = dir.path().join("sources");
        fs::create_dir_all(&sources).unwrap();
        fs::write(
            sources.join("demo.yaml"),
            "id: source/demo\nkind: source\nname: Demo\npipeline:\n  - run:\n      - adapter/feed-items\n  - run_if_accepts:\n      - adapter/probe\n",
        )
        .unwrap();
        // 零参数与探针步骤都不迁移：计划为空。
        let plan = plan_migration(dir.path()).unwrap();
        assert!(plan.is_empty());
    }

    /// R5.4 / H9：原子作业任一文件校验失败时，已写目标全部回滚（新建文件被移除）。
    #[test]
    fn apply_atomic_rolls_back_when_validation_fails() {
        let dir = tempfile::tempdir().unwrap();
        let preset = crate::builtin_presets()
            .into_iter()
            .find(|item| item.id == "preset/standard")
            .unwrap();
        fs::create_dir_all(dir.path().join("presets")).unwrap();
        fs::write(dir.path().join("presets/standard.yaml"), preset.source).unwrap();
        fs::write(
            dir.path().join("system.yaml"),
            "id: system\nkind: system\nname: s\npolicy:\n  preset: standard\n",
        )
        .unwrap();

        // 写入一份缺 access 的畸形信源，validate 必失败。
        let bad = AtomicWrite::write(
            "sources/ghost.yaml",
            "id: source/ghost\nkind: source\nname: Ghost\n".to_string(),
        );
        assert!(
            apply_atomic(
                dir.path(),
                &LoadOptions::default(),
                std::slice::from_ref(&bad)
            )
            .is_err(),
            "畸形写入应被拒绝"
        );
        // 回滚：新建文件不应残留。
        assert!(
            !dir.path().join("sources/ghost.yaml").exists(),
            "失败后新建文件必须被回滚删除"
        );
    }

    /// v3 W1 验收：真实仓库 `config/` 的每个信源都带显式 `sourceType`，且整库校验通过
    /// （类型缺失不作为硬错误，故显式声明由回填保证，本测试是回归护栏）。
    #[test]
    fn real_config_sources_all_carry_explicit_type() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../config")
            .canonicalize()
            .expect("仓库 config 目录应存在");
        assert!(
            dir.join("system.yaml").is_file(),
            "未找到 config/system.yaml"
        );
        let options = LoadOptions::default();
        validate(&dir, &options).expect("真实 config 应通过含类型的整库校验");
        let missing = plan_source_type_backfill(&dir).expect("清点缺失类型");
        assert!(
            missing.is_empty(),
            "所有信源应已显式声明 sourceType，缺失: {missing:?}"
        );
    }

    /// 回填幂等且不改其余取值：对临时副本重复回填，第二次应 0 改写，且类型与派生一致。
    #[test]
    fn backfill_is_idempotent_and_preserves_values() {
        let dir = tempfile::tempdir().unwrap();
        copy_fixture_sources(dir.path());
        let options = LoadOptions::default();
        let first = backfill_source_types(dir.path(), &options).expect("首次回填");
        assert!(first > 0, "样本源应至少回填一份");
        let second = backfill_source_types(dir.path(), &options).expect("二次回填应幂等");
        assert_eq!(second, 0, "已带类型的信源不应被再次改写");
        for entry in author::entries(dir.path(), "source").unwrap() {
            let declared = entry.doc["sourceType"].as_str().unwrap();
            assert_eq!(declared, source_type::derive(&entry.doc).as_str());
        }
    }

    /// 复制两份最小可校验的样本信源到临时目录（含 rss 与 http 两种形态）。
    fn copy_fixture_sources(dir: &Path) {
        let preset = crate::builtin_presets()
            .into_iter()
            .find(|item| item.id == "preset/standard")
            .expect("内置预设");
        fs::create_dir_all(dir.join("presets")).unwrap();
        fs::write(dir.join("presets/standard.yaml"), preset.source).unwrap();
        fs::write(
            dir.join("system.yaml"),
            "id: system\nkind: system\nname: s\npolicy:\n  preset: standard\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("accessors")).unwrap();
        for (stem, implementation) in [("rss", "builtin:rss"), ("http", "builtin:http")] {
            fs::write(
                dir.join("accessors").join(format!("{stem}.yaml")),
                format!(
                    "id: accessor/{stem}\nkind: accessor\nname: {stem}\nimplementation: {implementation}\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n"
                ),
            )
            .unwrap();
        }
        fs::create_dir_all(dir.join("adapters")).unwrap();
        fs::write(
            dir.join("adapters/feed-items.yaml"),
            "id: adapter/feed-items\nkind: adapter\nname: Feed\nimplementation: builtin:feed-items\nrole: search\nconsumes:\n  - {kind: raw, part_role: document}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: exponential\n    base: 1s\n    max: 30s\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("sources")).unwrap();
        fs::write(
            dir.join("sources/feed.yaml"),
            "id: source/feed\nkind: source\nname: Feed\naccess:\n  accessor: accessor/rss\n  locator:\n    url: https://example.com/feed\npipeline:\n  - run:\n      - adapter/feed-items\nadmission: {}\n",
        )
        .unwrap();
        fs::write(
            dir.join("sources/page.yaml"),
            "id: source/page\nkind: source\nname: Page\naccess:\n  accessor: accessor/http\n  locator:\n    url: https://example.com/list\npipeline:\n  - run:\n      - adapter/feed-items\nadmission: {}\n",
        )
        .unwrap();
    }
}
