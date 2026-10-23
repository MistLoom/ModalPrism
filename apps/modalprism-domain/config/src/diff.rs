//! 两份快照的差异。

use std::collections::BTreeSet;

use serde_json::Value;

use crate::snapshot::Snapshot;

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<Change> {
    let mut changes = Vec::new();
    walk("system", &before.system, &after.system, &mut changes);
    let ids = before
        .modules
        .keys()
        .chain(after.modules.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for id in ids {
        match (before.modules.get(&id), after.modules.get(&id)) {
            (Some(left), Some(right)) => walk(&id, left, right, &mut changes),
            (Some(left), None) => changes.push(Change {
                path: id,
                before: Some(left.clone()),
                after: None,
            }),
            (None, Some(right)) => changes.push(Change {
                path: id,
                before: None,
                after: Some(right.clone()),
            }),
            (None, None) => {}
        }
    }
    changes
}

fn walk(path: &str, before: &Value, after: &Value, changes: &mut Vec<Change>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match (left.get(&key), right.get(&key)) {
                    (Some(before), Some(after)) => walk(&child, before, after, changes),
                    (Some(before), None) => changes.push(Change {
                        path: child,
                        before: Some(before.clone()),
                        after: None,
                    }),
                    (None, Some(after)) => changes.push(Change {
                        path: child,
                        before: None,
                        after: Some(after.clone()),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ => changes.push(Change {
            path: path.to_string(),
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
    }
}
