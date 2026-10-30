//! 合并之后的语义检查。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use crate::error::Diagnostic;
use crate::scope::{DocKind, Scope, accepts_target, is_known_lock, scope_of};
use crate::security::security_issues;

const RESERVED_SPEC: &[&str] = &[
    "id",
    "kind",
    "name",
    "description",
    "enabled",
    "labels",
    "project",
    "shared",
    "extends",
    "policy",
    "spec",
    "implementation",
    "limits",
    "egress",
    "paid",
    "role",
    "consumes",
    "produces",
    "access",
    "pipeline",
    "schedule",
    "admission",
    "sourceType",
    "strata",
    "raw",
    "media",
    "versions",
    "commit",
    "state",
    "robots",
    "render",
    "capture",
    "reprocess",
    "identity",
    "dedupe",
    "backfill",
    "classify",
    "health",
    "preset",
    "locks",
];

pub fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    if path.is_empty() {
        return Some(current);
    }
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

pub fn check_modules(
    modules: &BTreeMap<String, Value>,
    system_policy: &Value,
    baseline_policy: &Value,
    locks: &[String],
    schemas: &BTreeMap<String, Value>,
    root: &Path,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (id, module) in modules {
        let kind = module.get("kind").and_then(Value::as_str).unwrap_or("");
        check_spec(id, module, &mut diags);
        check_schema_ref(id, module, schemas, root, &mut diags);
        match kind {
            "source" => {
                check_source(id, module, modules, system_policy, &mut diags);
            }
            "accessor" => check_accessor(id, module, &mut diags),
            "adapter" => check_adapter(id, module, &mut diags),
            "binding" => check_binding(id, module, modules, &mut diags),
            "plan" => check_plan(id, module, modules, &mut diags),
            "preset" | "profile" | "type" => {}
            _ => diags.push(bare(format!("未知 kind: {kind}"))),
        }
    }
    check_locks(modules, system_policy, baseline_policy, locks, &mut diags);
    diags
}

fn check_source(
    id: &str,
    module: &Value,
    modules: &BTreeMap<String, Value>,
    system_policy: &Value,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(access) = module.get("access") else {
        diags.push(of(id, "信源缺少 access"));
        return;
    };
    match access.get("accessor").and_then(Value::as_str) {
        Some(accessor) => match modules.get(accessor) {
            Some(target) if target.get("kind").and_then(Value::as_str) == Some("accessor") => {}
            Some(_) => diags.push(of(id, format!("引用的模块不是访问器: {accessor}"))),
            None => diags.push(of(id, format!("引用不存在: {accessor}"))),
        },
        None => diags.push(of(id, "信源缺少 access.accessor")),
    }
    if module.get("pipeline").is_none() {
        diags.push(of(id, "信源缺少 pipeline"));
    }
    if let Some(steps) = module.get("pipeline").and_then(Value::as_array) {
        for step in steps {
            // R3：步骤可改为引用独立 binding，而非内嵌可执行 spec。source→binding 存在性。
            if let Some(binding) = step.get("binding").and_then(Value::as_str) {
                match modules.get(binding) {
                    Some(target)
                        if target.get("kind").and_then(Value::as_str) == Some("binding") => {}
                    Some(_) => {
                        diags.push(of(id, format!("引用的模块不是绑定: {binding}")));
                    }
                    None => diags.push(of(id, format!("引用不存在: {binding}"))),
                }
            }
            let run = string_array(step.get("run"));
            let probe = string_array(step.get("run_if_accepts"));
            if run.is_empty() && probe.is_empty() && step.get("binding").is_none() {
                diags.push(of(id, "管道步骤缺少适配器或绑定"));
            }
            for adapter in run.into_iter().chain(probe) {
                match modules.get(&adapter) {
                    Some(target)
                        if target.get("kind").and_then(Value::as_str) == Some("adapter") => {}
                    Some(_) => diags.push(of(id, format!("引用的模块不是适配器: {adapter}"))),
                    None => diags.push(of(id, format!("引用不存在: {adapter}"))),
                }
            }
        }
    }
    // 顶层 `bindings` 便捷清单同样要存在（R10.3 双向存在性）。
    if let Some(bindings) = module.get("bindings").and_then(Value::as_array) {
        for binding in bindings.iter().filter_map(Value::as_str) {
            match modules.get(binding) {
                Some(target) if target.get("kind").and_then(Value::as_str) == Some("binding") => {}
                Some(_) => diags.push(of(id, format!("引用的模块不是绑定: {binding}"))),
                None => diags.push(of(id, format!("引用不存在: {binding}"))),
            }
        }
    }
    check_robots(id, module.get("policy"), system_policy, diags);
    check_source_budget(id, module, diags);
    // R12/R14：类型闭集、声明一致性与 accessor/adapter/paid 组合矩阵。
    for issue in crate::source_type::validate(module) {
        diags.push(of(id, issue));
    }
    if lookup(module, "policy.raw.store").and_then(Value::as_str) == Some("sampled")
        && lookup(module, "policy.raw.sample_rate").is_none()
    {
        diags.push(of(id, "raw.store 为 sampled 时必须写出 raw.sample_rate"));
    }
}

// 采集节律已迁出信源，改由计划的 trigger 承载；信源若仍写 schedule 也不再校验其形态。
// 保留此说明以对照旧 `check_schedule` 的去留。

/// 采集计划：targets 必须都存在且是信源；trigger 形态合法（recurring 带节律，oneShot 允许一次性）。
fn check_plan(
    id: &str,
    module: &Value,
    modules: &BTreeMap<String, Value>,
    diags: &mut Vec<Diagnostic>,
) {
    let targets = string_array(module.get("targets"));
    if targets.is_empty() {
        diags.push(of(id, "计划缺少 targets"));
    }
    for target in &targets {
        match modules.get(target) {
            Some(source) if source.get("kind").and_then(Value::as_str) == Some("source") => {}
            Some(_) => diags.push(of(id, format!("计划 target 不是信源: {target}"))),
            None => diags.push(of(id, format!("计划引用不存在的信源: {target}"))),
        }
    }
    let Some(trigger) = module.get("trigger") else {
        diags.push(of(id, "计划缺少 trigger"));
        return;
    };
    match trigger.get("kind").and_then(Value::as_str) {
        Some("oneShot") => {}
        Some("recurring") | None => {
            if trigger.get("adaptive").and_then(Value::as_bool) == Some(true) {
                for key in ["floor", "ceiling", "expectedPeriod"] {
                    if trigger.get(key).is_none() {
                        diags.push(of(id, format!("adaptive 为 true 时缺少 trigger.{key}")));
                    }
                }
            } else if trigger.get("interval").is_none() {
                diags.push(of(id, "recurring 计划 trigger 缺少 interval"));
            }
        }
        Some(other) => diags.push(of(id, format!("未知 trigger.kind: {other}"))),
    }
}

fn check_robots(
    id: &str,
    policy: Option<&Value>,
    system_policy: &Value,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(robots) = policy.and_then(|policy| policy.get("robots")) else {
        return;
    };
    let mode = robots.get("mode").and_then(Value::as_str);
    let reason = robots.get("robots_reason").and_then(Value::as_str);
    let exempt = robots.get("exempt_as").and_then(Value::as_str);
    match mode {
        Some("ignore") => {
            if reason.is_none() {
                diags.push(of(id, "robots.mode 为 ignore 时必须写出 robots_reason"));
            }
            if exempt.is_some() {
                diags.push(of(id, "robots.mode 为 ignore 时不写 exempt_as"));
            }
        }
        Some("obey-with-exemptions") => {
            if let Some(category) = exempt {
                if reason.is_none() {
                    diags.push(of(id, "使用 robots 豁免时必须写出 robots_reason"));
                }
                let known = system_policy
                    .pointer("/robots/exemptions")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                if !known.contains(category) {
                    diags.push(of(id, format!("豁免类别未定义: {category}")));
                }
            }
        }
        Some(other) if exempt.is_some() => {
            diags.push(of(id, format!("robots.mode 为 {other} 时不写 exempt_as")));
        }
        _ => {}
    }
}

fn check_accessor(id: &str, module: &Value, diags: &mut Vec<Diagnostic>) {
    if module
        .get("implementation")
        .and_then(Value::as_str)
        .is_none()
    {
        diags.push(of(id, "访问器缺少 implementation"));
    }
    if module.get("limits").is_none() {
        diags.push(of(id, "访问器缺少 limits"));
    }
    if module.get("egress").is_none() {
        diags.push(of(id, "访问器缺少 egress"));
    }
    let implementation = module
        .get("implementation")
        .and_then(Value::as_str)
        .unwrap_or("");
    if implementation.starts_with("builtin:paid")
        && module
            .pointer("/paid/service")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        diags.push(of(id, "付费访问器缺少 paid.service"));
    }
}

fn check_source_budget(id: &str, module: &Value, diags: &mut Vec<Diagnostic>) {
    let paid_accessor =
        module.pointer("/access/accessor").and_then(Value::as_str) == Some("accessor/paid-api");
    if !paid_accessor && module.get("paid").is_none() {
        return;
    }
    let requests = module
        .pointer("/paid/budget/requests")
        .and_then(Value::as_u64);
    let window = module
        .pointer("/paid/budget/window")
        .and_then(Value::as_str);
    if requests.is_none() || window.is_none() {
        diags.push(of(id, "付费信源缺少 paid.budget"));
    }
}

fn check_adapter(id: &str, module: &Value, diags: &mut Vec<Diagnostic>) {
    if module.get("implementation").is_none() {
        diags.push(of(id, "适配器缺少 implementation"));
    }
    if module.get("limits").is_none() {
        diags.push(of(id, "适配器缺少 limits"));
    }
    let role = module.get("role").and_then(Value::as_str);
    let produces = module.get("produces").and_then(Value::as_array);
    match (role, produces) {
        (Some("search"), Some(items)) => {
            if items.is_empty() || items.iter().any(|item| item.as_str() != Some("discovery")) {
                diags.push(of(id, "search 适配器的 produces 必须是 discovery"));
            }
        }
        (Some("content"), Some(items)) => {
            if items.is_empty()
                || items
                    .iter()
                    .any(|item| item.get("kind").and_then(Value::as_str).is_none())
            {
                diags.push(of(id, "content 适配器的 produces 必须带 kind"));
            }
        }
        _ => diags.push(of(id, "适配器缺少 role 或 produces")),
    }
    if module.get("consumes").and_then(Value::as_array).is_none() {
        diags.push(of(id, "适配器缺少 consumes"));
    }
    // R10.2 / H4：适配器落盘前走公共安全校验器。
    for issue in security_issues(module) {
        diags.push(of(id, format!("安全检查失败: {issue}")));
    }
}

/// 校验 AdapterBinding（R5/R10.2/R10.3）。
///
/// - binding→adapter：`adapter` 必填且指向存在的适配器（悬空即失败）。
/// - 承载参数：至少给出 `params` 或 `spec` 之一，否则等价于空壳。
/// - R10.2：与适配器同一套公共安全检查（禁用串 / 回环 / SSRF / `javascript:` / 环境变量泄露）。
fn check_binding(
    id: &str,
    module: &Value,
    modules: &BTreeMap<String, Value>,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(adapter) = module.get("adapter").and_then(Value::as_str) else {
        diags.push(of(id, "绑定缺少 adapter 引用"));
        return;
    };
    match modules.get(adapter) {
        Some(target) if target.get("kind").and_then(Value::as_str) == Some("adapter") => {}
        Some(_) => diags.push(of(id, format!("引用的模块不是适配器: {adapter}"))),
        None => diags.push(of(id, format!("引用不存在: {adapter}"))),
    }
    let has_params = module
        .get("params")
        .and_then(Value::as_object)
        .is_some_and(|map| !map.is_empty())
        || module
            .get("spec")
            .and_then(Value::as_object)
            .is_some_and(|map| !map.is_empty());
    if !has_params {
        diags.push(of(id, "绑定未提供任何装配参数（params 或 spec）"));
    }
    for issue in security_issues(module) {
        diags.push(of(id, format!("安全检查失败: {issue}")));
    }
}

fn check_spec(id: &str, module: &Value, diags: &mut Vec<Diagnostic>) {
    let Some(spec) = module.get("spec").and_then(Value::as_object) else {
        return;
    };
    for key in spec.keys() {
        if key == "$schema" {
            continue;
        }
        if RESERVED_SPEC.contains(&key.as_str()) {
            diags.push(of(id, format!("spec 使用了基础字段名: {key}")));
        }
    }
}

fn check_schema_ref(
    id: &str,
    module: &Value,
    schemas: &BTreeMap<String, Value>,
    root: &Path,
    diags: &mut Vec<Diagnostic>,
) {
    if let Some(path) = module.get("schema").and_then(Value::as_str) {
        if !root.join(path).is_file() {
            diags.push(of(id, format!("schema 文件不存在: {path}")));
        }
    }
    let Some(pointer) = module.pointer("/spec/$schema").and_then(Value::as_str) else {
        return;
    };
    if !schemas.contains_key(pointer) {
        diags.push(of(id, format!("spec schema 不存在: {pointer}")));
    }
}

fn check_locks(
    modules: &BTreeMap<String, Value>,
    system_policy: &Value,
    baseline_policy: &Value,
    locks: &[String],
    diags: &mut Vec<Diagnostic>,
) {
    for path in locks {
        if !is_known_lock(path) {
            diags.push(bare(format!("锁定路径未知: {path}")));
            continue;
        }
        let Some(baseline) = lookup(baseline_policy, path) else {
            diags.push(bare(format!("锁定路径没有基准值: {path}")));
            continue;
        };
        let Some(scope) = scope_of(path) else {
            continue;
        };
        if scope == Scope::System {
            match lookup(system_policy, path) {
                Some(effective) if effective == baseline => {}
                Some(_) => diags.push(bare(format!("锁定冲突: {path}"))),
                None => diags.push(bare(format!("锁定冲突: {path}"))),
            }
            continue;
        }
        for (id, module) in modules {
            let kind = match module.get("kind").and_then(Value::as_str) {
                Some("source") => DocKind::Source,
                Some("accessor") => DocKind::Accessor,
                Some("adapter") => DocKind::Adapter,
                Some("binding") => DocKind::AdapterBinding,
                _ => continue,
            };
            if !accepts_target(kind, scope) {
                continue;
            }
            let effective = lookup(module, &format!("policy.{path}"));
            if effective != Some(baseline) {
                diags.push(of(id, format!("锁定冲突: {path}")));
            }
        }
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn of(id: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        file: id.to_string(),
        line: 0,
        message: message.into(),
    }
}

fn bare(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        file: String::new(),
        line: 0,
        message: message.into(),
    }
}
