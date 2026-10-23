//! 字段溯源。

use serde_json::Value;

use crate::error::{Diagnostic, Diagnostics, Error};
use crate::node::Origin;
use crate::scope::{Scope, scope_of};
use crate::snapshot::Snapshot;

#[derive(Clone, Debug, PartialEq)]
pub struct FieldOrigin {
    pub path: String,
    pub value: Value,
    pub file: String,
    pub line: u32,
    pub preset_id: Option<String>,
    pub via_preset: Option<String>,
    pub system_scope: bool,
}

pub fn explain(snapshot: &Snapshot, id: &str) -> Result<Vec<FieldOrigin>, Error> {
    let value = if id == "system" {
        &snapshot.system
    } else {
        snapshot.modules.get(id).ok_or_else(|| {
            Error::Invalid(Diagnostics(vec![Diagnostic {
                file: id.to_string(),
                line: 0,
                message: format!("模块不存在: {id}"),
            }]))
        })?
    };
    let mut fields = Vec::new();
    for ((owner, path), origin) in &snapshot.origins {
        if owner != id || path.is_empty() {
            continue;
        }
        let Some(item) = lookup(value, path) else {
            continue;
        };
        fields.push(field(path, item, origin, is_system_scope(path)));
    }
    if id != "system" {
        for ((owner, path), origin) in &snapshot.origins {
            if owner != "system" || !is_system_scope(path) {
                continue;
            }
            if fields.iter().any(|item| item.path == *path) {
                continue;
            }
            let Some(item) = lookup(&snapshot.system, path) else {
                continue;
            };
            fields.push(field(path, item, origin, true));
        }
    }
    fields.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(fields)
}

fn field(path: &str, value: &Value, origin: &Origin, system_scope: bool) -> FieldOrigin {
    FieldOrigin {
        path: path.to_string(),
        value: value.clone(),
        file: origin.file.clone(),
        line: origin.line,
        preset_id: origin.preset_id.clone(),
        via_preset: origin.via_preset.clone(),
        system_scope,
    }
}

fn is_system_scope(path: &str) -> bool {
    path.strip_prefix("policy.")
        .and_then(scope_of)
        .is_some_and(|scope| scope == Scope::System)
}

fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}
