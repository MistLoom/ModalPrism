//! 合并之后的语义检查。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use crate::error::Diagnostic;
use crate::scope::{DocKind, Scope, accepts_target, is_known_lock, scope_of};

const RESERVED_SPEC: &[&str] = &[
    "id",
    "kind",
    "name",
    "description",
    "enabled",
    "labels",
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
            let run = string_array(step.get("run"));
            let probe = string_array(step.get("run_if_accepts"));
            if run.is_empty() && probe.is_empty() {
                diags.push(of(id, "管道步骤缺少适配器"));
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
    check_schedule(id, module.get("schedule"), diags);
    if module.get("admission").is_none() {
        diags.push(of(id, "信源缺少 admission"));
    }
    check_robots(id, module.get("policy"), system_policy, diags);
    if lookup(module, "policy.raw.store").and_then(Value::as_str) == Some("sampled")
        && lookup(module, "policy.raw.sample_rate").is_none()
    {
        diags.push(of(id, "raw.store 为 sampled 时必须写出 raw.sample_rate"));
    }
}

fn check_schedule(id: &str, schedule: Option<&Value>, diags: &mut Vec<Diagnostic>) {
    let Some(schedule) = schedule else {
        diags.push(of(id, "信源缺少 schedule"));
        return;
    };
    if schedule.get("adaptive").and_then(Value::as_bool) == Some(true) {
        for key in ["floor", "ceiling", "expectedPeriod"] {
            if schedule.get(key).is_none() {
                diags.push(of(id, format!("adaptive 为 true 时缺少 schedule.{key}")));
            }
        }
    } else if schedule.get("interval").is_none() {
        diags.push(of(id, "schedule 缺少 interval"));
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
    if implementation.starts_with("builtin:paid") && module.get("paid").is_none() {
        diags.push(of(id, "付费访问器缺少 paid"));
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
