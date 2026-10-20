//! 策略键的作用域。

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocKind {
    System,
    Preset,
    Profile,
    Accessor,
    Adapter,
    Source,
    Type,
    Defaults,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    System,
    Source,
    Accessor,
    Shared,
}

const PATHS: &[(&str, Scope)] = &[
    ("raw.store", Scope::Source),
    ("raw.sample_rate", Scope::Source),
    ("raw.retain", Scope::Source),
    ("raw.max_part_size", Scope::Source),
    ("raw.redact_headers", Scope::System),
    ("media.mode", Scope::Source),
    ("media.kinds", Scope::Source),
    ("media.max_size", Scope::Source),
    ("versions.facet_keep", Scope::Source),
    ("versions.snapshot_retain", Scope::System),
    ("versions.audit_tag", Scope::System),
    ("commit.flush_interval", Scope::System),
    ("commit.flush_max_rows", Scope::System),
    ("state.snapshot_interval", Scope::System),
    ("robots.mode", Scope::Source),
    ("robots.exemptions", Scope::System),
    ("robots.cache_ttl", Scope::System),
    ("robots.on_unreachable", Scope::System),
    ("robots.crawl_delay", Scope::Source),
    ("robots.exempt_as", Scope::Source),
    ("robots.robots_reason", Scope::Source),
    ("render.max_level", Scope::Source),
    ("render.fallback_engine", Scope::Source),
    ("render.downgrade_probe", Scope::Source),
    ("capture.screenshot", Scope::Source),
    ("capture.network", Scope::Source),
    ("capture.renditions", Scope::Source),
    ("egress.private_networks", Scope::System),
    ("egress.identity", Scope::System),
    ("paid.unknown_receipt", Scope::Accessor),
    ("reprocess.on_adapter_change", Scope::Shared),
    ("identity.merge_primary", Scope::System),
    ("identity.strip_params", Scope::System),
    ("identity.fragment", Scope::System),
    ("dedupe.near_duplicate", Scope::Source),
    ("backfill.initial", Scope::Source),
    ("classify.detector", Scope::Shared),
    ("health.expected_period", Scope::Source),
    ("health.yield_drop_ratio", Scope::Source),
    ("health.failure_threshold", Scope::Source),
];

/// 把系统作用域策略键覆盖到模块策略上，供访问器读取有效策略。
pub fn overlay_system_policy(system: &Value, module_policy: &Value) -> Value {
    let mut policy = match module_policy {
        Value::Object(map) => Value::Object(map.clone()),
        _ => Value::Object(Map::new()),
    };
    if let Some(system_policy) = system.get("policy") {
        copy_system(&mut policy, system_policy, "");
    }
    policy
}

fn copy_system(dst: &mut Value, src: &Value, path: &str) {
    let Some(map) = src.as_object() else {
        return;
    };
    for (key, value) in map {
        if key == "preset" || key == "locks" {
            continue;
        }
        let child = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        if value.is_object() && scope_of(&child).is_none() {
            let entry = dst
                .as_object_mut()
                .expect("策略是对象")
                .entry(key.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            copy_system(entry, value, &child);
            continue;
        }
        if scope_of(&child) == Some(Scope::System) {
            dst.as_object_mut()
                .expect("策略是对象")
                .insert(key.clone(), value.clone());
        }
    }
}

pub fn scope_of(path: &str) -> Option<Scope> {
    let mut found = None;
    let mut found_len = 0;
    for (key, scope) in PATHS {
        if (*key == path
            || path
                .strip_prefix(key)
                .is_some_and(|rest| rest.starts_with('.')))
            && key.len() >= found_len
        {
            found = Some(*scope);
            found_len = key.len();
        }
    }
    found
}

pub fn is_known_lock(path: &str) -> bool {
    PATHS.iter().any(|(key, _)| *key == path)
}

/// 该文档是否允许写出这个作用域的键。
///
/// `defaults_for` 在 `kind` 为 [`DocKind::Defaults`] 时给出目标：`None` 表示 `common.yaml`。
pub fn allows_write(kind: DocKind, scope: Scope, defaults_for: Option<DocKind>) -> bool {
    let kind = if kind == DocKind::Defaults {
        match defaults_for {
            Some(target) => target,
            None => DocKind::Profile,
        }
    } else {
        kind
    };
    match kind {
        DocKind::System | DocKind::Preset => true,
        DocKind::Profile | DocKind::Defaults => scope != Scope::System,
        DocKind::Source => matches!(scope, Scope::Source | Scope::Shared),
        DocKind::Accessor => scope == Scope::Accessor,
        DocKind::Adapter => scope == Scope::Shared,
        DocKind::Type => false,
    }
}

/// 合并进目标 kind 时是否带上该键。系统作用域另由调用方决定。
pub fn accepts_target(kind: DocKind, scope: Scope) -> bool {
    match kind {
        DocKind::System => true,
        DocKind::Source => matches!(scope, Scope::Source | Scope::Shared),
        DocKind::Accessor => scope == Scope::Accessor,
        DocKind::Adapter => scope == Scope::Shared,
        _ => false,
    }
}

pub fn top_level_keys(kind: DocKind) -> &'static [&'static str] {
    match kind {
        DocKind::System => &["kind", "id", "name", "description", "strata", "policy"],
        DocKind::Preset => &["id", "kind", "name", "description", "extends", "policy"],
        DocKind::Profile => &[
            "id",
            "kind",
            "name",
            "description",
            "enabled",
            "labels",
            "extends",
            "policy",
            "spec",
            "egress",
            "limits",
        ],
        DocKind::Accessor => &[
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
        ],
        DocKind::Adapter => &[
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
            "role",
            "consumes",
            "produces",
        ],
        DocKind::Source => &[
            "id",
            "kind",
            "name",
            "description",
            "enabled",
            "labels",
            "extends",
            "policy",
            "spec",
            "egress",
            "paid",
            "access",
            "pipeline",
            "schedule",
            "admission",
        ],
        DocKind::Type => &[
            "id",
            "kind",
            "name",
            "description",
            "enabled",
            "labels",
            "extends",
            "spec",
            "schema",
        ],
        DocKind::Defaults => &[
            "description",
            "enabled",
            "labels",
            "policy",
            "spec",
            "egress",
            "limits",
            "implementation",
            "paid",
            "role",
            "consumes",
            "produces",
            "access",
            "pipeline",
            "schedule",
            "admission",
        ],
    }
}
