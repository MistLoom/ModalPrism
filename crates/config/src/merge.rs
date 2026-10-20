//! 深度合并、列表操作与按作用域过滤。

use crate::node::{Data, Node};
use crate::scope::{DocKind, Scope, accepts_target, scope_of};

pub fn merge(base: Node, over: Node) -> Result<Node, String> {
    if over.is_null() {
        return Ok(over);
    }
    match (&base.data, &over.data) {
        (Data::Map(_), Data::Map(_)) => merge_maps(base, over),
        (Data::Array(_), Data::Map(_)) if is_list_op(&over) => apply_list_op(base, &over),
        _ => Ok(over),
    }
}

fn merge_maps(base: Node, over: Node) -> Result<Node, String> {
    let Data::Map(base_entries) = base.data else {
        return Ok(over);
    };
    let Data::Map(over_entries) = over.data else {
        return Ok(over);
    };
    let mut entries = base_entries;
    for (key, value) in over_entries {
        if value.is_null() {
            entries.retain(|(name, _)| name != &key);
            continue;
        }
        if let Some(index) = entries.iter().position(|(name, _)| name == &key) {
            let previous = entries[index].1.clone();
            entries[index].1 = merge(previous, value)?;
        } else {
            entries.push((key, value));
        }
    }
    Ok(Node {
        file: over.file,
        line: over.line,
        preset_id: over.preset_id,
        data: Data::Map(entries),
    })
}

fn is_list_op(node: &Node) -> bool {
    let Some(entries) = node.as_map() else {
        return false;
    };
    if entries.len() != 1 {
        return false;
    }
    matches!(entries[0].0.as_str(), "$append" | "$prepend" | "$remove")
}

fn apply_list_op(base: Node, op: &Node) -> Result<Node, String> {
    let Data::Array(mut items) = base.data else {
        return Err("列表操作的原值不是列表".into());
    };
    let Some(entries) = op.as_map() else {
        return Err("列表操作必须是对象".into());
    };
    let (name, payload) = &entries[0];
    let Data::Array(extra) = &payload.data else {
        return Err(format!("{name} 的值必须是列表"));
    };
    match name.as_str() {
        "$append" => items.extend(extra.iter().cloned()),
        "$prepend" => {
            let mut next = extra.clone();
            next.extend(items);
            items = next;
        }
        "$remove" => {
            items.retain(|item| !extra.iter().any(|candidate| same_value(item, candidate)));
        }
        _ => return Err(format!("未知的列表操作 {name}")),
    }
    Ok(Node {
        file: op.file.clone(),
        line: op.line,
        preset_id: op.preset_id.clone(),
        data: Data::Array(items),
    })
}

fn same_value(left: &Node, right: &Node) -> bool {
    match (&left.data, &right.data) {
        (Data::Null, Data::Null) => true,
        (Data::Bool(a), Data::Bool(b)) => a == b,
        (Data::Int(a), Data::Int(b)) => a == b,
        (Data::Float(a), Data::Float(b)) => a == b,
        (Data::String(a), Data::String(b)) => a == b,
        (Data::Array(a), Data::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_value(x, y))
        }
        (Data::Map(a), Data::Map(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .any(|(other, rhs)| other == key && same_value(value, rhs))
                })
        }
        _ => false,
    }
}

/// 把 `src` 的策略键合并进 `dst`。`include_system` 为假时跳过系统作用域。
pub fn merge_policy(
    dst: &mut Node,
    src: &Node,
    target: DocKind,
    include_system: bool,
) -> Result<(), String> {
    if !matches!(dst.data, Data::Map(_)) {
        *dst = Node::map(src.file.clone(), src.line);
    }
    merge_policy_at(dst, src, "", target, include_system)
}

fn merge_policy_at(
    dst: &mut Node,
    src: &Node,
    prefix: &str,
    target: DocKind,
    include_system: bool,
) -> Result<(), String> {
    let Data::Map(entries) = &src.data else {
        return Err("策略必须是映射".into());
    };
    for (key, value) in entries {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if path == "preset" || path == "locks" {
            continue;
        }
        if value.is_null() {
            if keep_leaf(&path, target, include_system) {
                remove_key(dst, key);
            }
            continue;
        }
        if is_list_op(value) {
            if keep_leaf(&path, target, include_system) {
                insert_merged(dst, key, value.clone())?;
            }
            continue;
        }
        if matches!(value.data, Data::Map(_)) && scope_of(&path).is_none() {
            if !matches!(dst.get(key).map(|node| &node.data), Some(Data::Map(_))) {
                let created = Node::map(value.file.clone(), value.line);
                insert_child(dst, key, created);
            }
            let child = dst_child_mut(dst, key);
            merge_policy_at(child, value, &path, target, include_system)?;
            continue;
        }
        if keep_leaf(&path, target, include_system) {
            insert_merged(dst, key, value.clone())?;
        }
    }
    Ok(())
}

fn keep_leaf(path: &str, target: DocKind, include_system: bool) -> bool {
    match scope_of(path) {
        Some(Scope::System) => include_system,
        Some(scope) => accepts_target(target, scope),
        None => true,
    }
}

fn remove_key(node: &mut Node, key: &str) {
    if let Data::Map(entries) = &mut node.data {
        entries.retain(|(name, _)| name != key);
    }
}

fn insert_child(node: &mut Node, key: &str, child: Node) {
    if let Data::Map(entries) = &mut node.data {
        if let Some(index) = entries.iter().position(|(name, _)| name == key) {
            entries[index].1 = child;
        } else {
            entries.push((key.to_string(), child));
        }
    }
}

fn dst_child_mut<'a>(node: &'a mut Node, key: &str) -> &'a mut Node {
    let Data::Map(entries) = &mut node.data else {
        unreachable!("策略目标已保证为映射");
    };
    let index = entries
        .iter()
        .position(|(name, _)| name == key)
        .expect("子键已插入");
    &mut entries[index].1
}

fn insert_merged(dst: &mut Node, key: &str, value: Node) -> Result<(), String> {
    if let Some(index) = dst
        .as_map()
        .and_then(|entries| entries.iter().position(|(name, _)| name == key))
    {
        let Data::Map(entries) = &mut dst.data else {
            return Ok(());
        };
        let previous = entries[index].1.clone();
        entries[index].1 = merge(previous, value)?;
    } else {
        insert_child(dst, key, value);
    }
    Ok(())
}

pub fn without_identity(node: &Node) -> Node {
    let Some(entries) = node.as_map() else {
        return node.clone();
    };
    let kept = entries
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "id" | "kind" | "name" | "extends"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Node {
        file: node.file.clone(),
        line: node.line,
        preset_id: node.preset_id.clone(),
        data: Data::Map(kept),
    }
}
