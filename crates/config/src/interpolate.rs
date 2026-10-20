//! `${env:NAME}`、`${secret:path}` 与 `${ref:id#a.b}`。

use std::collections::BTreeMap;

use crate::error::Diagnostic;
use crate::node::{Data, Node};

pub fn interpolate(
    system: &mut Node,
    modules: &mut BTreeMap<String, Node>,
    env: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, String>,
) -> Vec<Diagnostic> {
    let originals = modules.clone();
    let system_original = system.clone();
    let mut diags = Vec::new();
    let world = World {
        system: &system_original,
        modules: &originals,
        env,
        secrets,
    };
    *system = rewrite("system", system, &world, &mut Vec::new(), &mut diags);
    let ids: Vec<String> = modules.keys().cloned().collect();
    for id in ids {
        let Some(node) = modules.get(&id).cloned() else {
            continue;
        };
        let rewritten = rewrite(&id, &node, &world, &mut Vec::new(), &mut diags);
        modules.insert(id, rewritten);
    }
    diags
}

struct World<'a> {
    system: &'a Node,
    modules: &'a BTreeMap<String, Node>,
    env: &'a BTreeMap<String, String>,
    secrets: &'a BTreeMap<String, String>,
}

fn rewrite(
    owner: &str,
    node: &Node,
    world: &World<'_>,
    stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
) -> Node {
    match &node.data {
        Data::String(text) => rewrite_string(owner, node, text, world, stack, diags),
        Data::Array(items) => {
            let items = items
                .iter()
                .map(|item| rewrite(owner, item, world, stack, diags))
                .collect();
            Node {
                data: Data::Array(items),
                ..node.clone()
            }
        }
        Data::Map(entries) => {
            let entries = entries
                .iter()
                .map(|(key, child)| (key.clone(), rewrite(owner, child, world, stack, diags)))
                .collect();
            Node {
                data: Data::Map(entries),
                ..node.clone()
            }
        }
        _ => node.clone(),
    }
}

fn rewrite_string(
    owner: &str,
    node: &Node,
    text: &str,
    world: &World<'_>,
    stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
) -> Node {
    let parts = match split_interps(text) {
        Ok(parts) => parts,
        Err(message) => {
            push(diags, node, message);
            return node.clone();
        }
    };
    if parts.iter().all(|part| matches!(part, Part::Text(_))) {
        return node.clone();
    }
    if parts.len() == 1 {
        return match &parts[0] {
            Part::Env(name) => string_node(node, lookup_env(node, name, world, diags)),
            Part::Secret(path) => {
                let _ = lookup_secret(node, path, world, diags);
                string_node(node, secret_placeholder(path))
            }
            Part::Ref { id, path } => resolve_ref(owner, node, id, path, world, stack, diags),
            Part::Text(text) => string_node(node, text.clone()),
        };
    }
    let mut combined = String::new();
    for part in parts {
        match part {
            Part::Text(text) => combined.push_str(&text),
            Part::Env(name) => combined.push_str(&lookup_env(node, &name, world, diags)),
            Part::Secret(path) => {
                let _ = lookup_secret(node, &path, world, diags);
                combined.push_str(&secret_placeholder(&path));
            }
            Part::Ref { id, path } => {
                let resolved = resolve_ref(owner, node, &id, &path, world, stack, diags);
                match resolved.as_str() {
                    Some(text) => combined.push_str(text),
                    None => {
                        push(diags, node, format!("插值 {id}#{path} 的值不是字符串"));
                    }
                }
            }
        }
    }
    string_node(node, combined)
}

fn resolve_ref(
    owner: &str,
    at: &Node,
    id: &str,
    path: &str,
    world: &World<'_>,
    stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
) -> Node {
    let mark = format!("{id}#{path}");
    if stack.iter().any(|item| item == &mark) {
        push(diags, at, format!("配置引用成环: {mark}"));
        return at.clone();
    }
    let Some(target_root) = world.get(id) else {
        push(diags, at, format!("配置引用不存在: {id}"));
        return at.clone();
    };
    let Some(target) = target_root.dig(path) else {
        push(diags, at, format!("配置引用不存在: {mark}"));
        return at.clone();
    };
    stack.push(mark);
    let resolved = rewrite(
        if id == "system" { "system" } else { id },
        target,
        world,
        stack,
        diags,
    );
    stack.pop();
    let _ = owner;
    Node {
        file: at.file.clone(),
        line: at.line,
        preset_id: at.preset_id.clone(),
        data: resolved.data,
    }
}

fn lookup_env(at: &Node, name: &str, world: &World<'_>, diags: &mut Vec<Diagnostic>) -> String {
    match world.env.get(name) {
        Some(value) => value.clone(),
        None => {
            push(diags, at, format!("环境变量不存在: {name}"));
            String::new()
        }
    }
}

fn secret_placeholder(path: &str) -> String {
    format!("${{secret:{path}}}")
}

fn lookup_secret(at: &Node, path: &str, world: &World<'_>, diags: &mut Vec<Diagnostic>) -> String {
    match world.secrets.get(path) {
        Some(value) => value.clone(),
        None => {
            push(diags, at, format!("密钥不存在: {path}"));
            String::new()
        }
    }
}

fn string_node(template: &Node, text: String) -> Node {
    Node {
        file: template.file.clone(),
        line: template.line,
        preset_id: template.preset_id.clone(),
        data: Data::String(text),
    }
}

fn push(diags: &mut Vec<Diagnostic>, node: &Node, message: String) {
    diags.push(Diagnostic {
        file: node.file.clone(),
        line: node.line,
        message,
    });
}

impl World<'_> {
    fn get(&self, id: &str) -> Option<&Node> {
        if id == "system" {
            Some(self.system)
        } else {
            self.modules.get(id)
        }
    }
}

enum Part {
    Text(String),
    Env(String),
    Secret(String),
    Ref { id: String, path: String },
}

fn split_interps(text: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut plain_start = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'$' && bytes[index + 1] == b'{' {
            if index > plain_start {
                parts.push(Part::Text(text[plain_start..index].to_string()));
            }
            let body_start = index + 2;
            let Some(rel_end) = text[body_start..].find('}') else {
                return Err("未闭合的插值".into());
            };
            let body = &text[body_start..body_start + rel_end];
            parts.push(parse_body(body)?);
            index = body_start + rel_end + 1;
            plain_start = index;
            continue;
        }
        index += 1;
    }
    if plain_start < text.len() {
        parts.push(Part::Text(text[plain_start..].to_string()));
    }
    Ok(parts)
}

fn parse_body(body: &str) -> Result<Part, String> {
    if let Some(name) = body.strip_prefix("env:") {
        if is_env_name(name) {
            return Ok(Part::Env(name.to_string()));
        }
        return Err(format!("非法环境变量名: {name}"));
    }
    if let Some(path) = body.strip_prefix("secret:") {
        if path.is_empty() || path.contains('}') {
            return Err("密钥路径为空".into());
        }
        return Ok(Part::Secret(path.to_string()));
    }
    if let Some(spec) = body.strip_prefix("ref:") {
        let Some((id, path)) = spec.split_once('#') else {
            return Err(format!("配置引用缺少路径: {spec}"));
        };
        if id.is_empty() || path.is_empty() {
            return Err(format!("配置引用不完整: {spec}"));
        }
        return Ok(Part::Ref {
            id: id.to_string(),
            path: path.to_string(),
        });
    }
    Err(format!("未知插值: {body}"))
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
