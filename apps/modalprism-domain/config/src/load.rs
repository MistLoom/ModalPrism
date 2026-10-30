//! 读取配置目录并生成快照。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::check::check_modules;
use crate::error::{Diagnostic, Diagnostics, Error};
use crate::interpolate::interpolate;
use crate::layout;
use crate::merge::{merge, merge_policy, without_identity};
use crate::node::{Data, Node, leaves};
use crate::schema::{self, SchemaFailure};
use crate::scope::{DocKind, allows_write, scope_of, top_level_keys};
use crate::snapshot::Snapshot;
use crate::yaml;

#[derive(Clone, Debug)]
pub struct LoadOptions {
    pub overlay: Option<String>,
    pub env: EnvSource,
    pub secrets: BTreeMap<String, String>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            overlay: None,
            env: EnvSource::Fixed(BTreeMap::new()),
            secrets: BTreeMap::new(),
        }
    }
}

impl LoadOptions {
    pub fn from_process() -> Self {
        Self {
            env: EnvSource::Process,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug)]
pub enum EnvSource {
    Fixed(BTreeMap<String, String>),
    Process,
}

struct Input {
    logical: String,
    actual: String,
    text: String,
    overlay: bool,
}

struct Parsed {
    logical: String,
    node: Node,
    system_overlay: Option<Node>,
}

struct Indexed {
    system: Node,
    system_overlay: Option<Node>,
    defaults_common: Option<Node>,
    defaults: BTreeMap<DocKind, Node>,
    presets: BTreeMap<String, Node>,
    profiles: BTreeMap<String, Node>,
    modules: BTreeMap<String, Node>,
}

struct Built {
    system: Node,
    baseline_policy: Node,
    modules: BTreeMap<String, Node>,
    selected: BTreeMap<String, String>,
    global_preset: String,
}

struct Choice {
    id: String,
    node: Node,
}

pub fn validate(dir: &Path, options: &LoadOptions) -> Result<Snapshot, Error> {
    validate_with_warnings(dir, options).map(|(snapshot, _)| snapshot)
}

/// 与 [`validate`] 相同，但把「非声明文件」的提示另外返回。
///
/// 配置目录里会出现不属于任何声明类型的东西：已下线功能留下的目录、临时草稿、
/// 编辑器备份。它们不能让整次加载失败——否则一份杂项文件就升级成「系统起不来」。
/// 这些位置记一条警告后跳过；只有声明本身的错误才是致命诊断。
pub fn validate_with_warnings(
    dir: &Path,
    options: &LoadOptions,
) -> Result<(Snapshot, Vec<Diagnostic>), Error> {
    let mut diags = Vec::new();
    let mut warnings = Vec::new();
    if let Some(name) = &options.overlay {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == ".."
            || name == "."
        {
            diags.push(diag("", 0, "环境名非法"));
        }
    }
    let env = match &options.env {
        EnvSource::Fixed(map) => map.clone(),
        EnvSource::Process => std::env::vars().collect(),
    };
    let files = read_tree(dir, options.overlay.as_deref(), &mut warnings)?;
    let schemas = read_schemas(dir, &mut diags);
    let docs = parse_docs(files, &mut diags);
    if !diags.is_empty() {
        return Err(Error::Invalid(Diagnostics(diags)));
    }
    let Some(mut built) = assemble(docs, &mut diags) else {
        return Err(Error::Invalid(Diagnostics(diags)));
    };
    diags.extend(interpolate(
        &mut built.system,
        &mut built.modules,
        &env,
        &options.secrets,
    ));
    if !diags.is_empty() {
        return Err(Error::Invalid(Diagnostics(diags)));
    }
    let snapshot = lower(built, &schemas, dir, &mut diags);
    if !diags.is_empty() {
        return Err(Error::Invalid(Diagnostics(diags)));
    }
    Ok((snapshot, warnings))
}

fn assemble(docs: Vec<Parsed>, diags: &mut Vec<Diagnostic>) -> Option<Built> {
    let indexed = index(docs, diags);
    if !diags.is_empty() {
        return None;
    }
    let global_node = select_global(&indexed.system, indexed.system_overlay.as_ref(), diags)?;
    let global = preset_id(global_node.as_str()?);
    let resolved_presets = resolve_presets(&indexed.presets, diags);
    let resolved_profiles = resolve_profiles(&indexed.profiles, diags);
    if !diags.is_empty() {
        return None;
    }
    let Some(global_policy) = resolved_presets.get(&global).cloned() else {
        diags.push(diag(
            &global_node.file,
            global_node.line,
            format!("预设不存在: {global}"),
        ));
        return None;
    };
    let file_policy = indexed
        .system
        .get("policy")
        .cloned()
        .unwrap_or_else(|| Node::map(&indexed.system.file, indexed.system.line));
    let overlay_policy = indexed
        .system_overlay
        .as_ref()
        .and_then(|node| node.get("policy"))
        .cloned();
    let mut baseline_policy = file_policy.clone();
    if let Some(overlay_policy) = &overlay_policy {
        baseline_policy =
            merge(baseline_policy, overlay_policy.clone()).unwrap_or(overlay_policy.clone());
    }
    let mut system_policy = merge(file_policy, global_policy).unwrap_or_else(|message| {
        diags.push(diag(&indexed.system.file, indexed.system.line, message));
        Node::map(&indexed.system.file, indexed.system.line)
    });
    if let Some(overlay_policy) = overlay_policy {
        system_policy = match merge(system_policy.clone(), overlay_policy) {
            Ok(merged) => merged,
            Err(message) => {
                diags.push(diag(&indexed.system.file, indexed.system.line, message));
                system_policy
            }
        };
    }
    let mut system = indexed.system.clone();
    if let Some(overlay) = &indexed.system_overlay {
        if let Ok(merged) = merge(system.clone(), without_policy(overlay)) {
            system = merged;
        }
    }
    set_key(&mut system, "policy", system_policy);
    if system.get("id").is_none() {
        let id_node = string_node(&system, "system");
        set_key(&mut system, "id", id_node);
    }

    let mut modules = BTreeMap::new();
    let mut selected = BTreeMap::new();
    for (id, doc) in &indexed.presets {
        let mut preset = doc.clone();
        if let Some(policy) = resolved_presets.get(id) {
            set_key(&mut preset, "policy", policy.clone());
        }
        modules.insert(id.clone(), preset);
        selected.insert(id.clone(), id.clone());
    }
    for (id, doc) in &indexed.profiles {
        modules.insert(id.clone(), doc.clone());
    }
    for (id, doc) in &indexed.modules {
        let kind = kind_from_id(id).unwrap_or(DocKind::Source);
        let choice = select_preset(doc, &indexed.profiles, &global_node);
        let Some(preset_policy) = resolved_presets.get(&choice.id).cloned() else {
            diags.push(diag(
                &doc.file,
                doc.line,
                format!("预设不存在: {}", choice.id),
            ));
            continue;
        };
        let inputs = ModuleInputs {
            common: &indexed.defaults_common,
            defaults: indexed.defaults.get(&kind),
            baseline: &baseline_policy,
            preset_policy: &preset_policy,
            choice: &choice,
            profiles: &resolved_profiles,
        };
        let merged = merge_module(doc, kind, &inputs, diags);
        modules.insert(id.clone(), merged);
        selected.insert(id.clone(), choice.id);
    }
    if !diags.is_empty() {
        return None;
    }
    Some(Built {
        system,
        baseline_policy,
        modules,
        selected,
        global_preset: global,
    })
}

fn select_global(
    system: &Node,
    overlay: Option<&Node>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Node> {
    let from_overlay = overlay
        .and_then(|node| node.get("policy"))
        .and_then(|policy| policy.get("preset"))
        .cloned();
    let node = from_overlay.or_else(|| {
        system
            .get("policy")
            .and_then(|policy| policy.get("preset"))
            .cloned()
    });
    let Some(node) = node else {
        diags.push(diag(
            &system.file,
            system.line,
            "system.yaml 缺少 policy.preset",
        ));
        return None;
    };
    if node.as_str().is_none() {
        diags.push(diag(&node.file, node.line, "policy.preset 必须是字符串"));
        return None;
    }
    Some(node)
}

fn select_preset(doc: &Node, profiles: &BTreeMap<String, Node>, global: &Node) -> Choice {
    if let Some(node) = doc.get("policy").and_then(|policy| policy.get("preset")) {
        if let Some(id) = node.as_str() {
            return Choice {
                id: preset_id(id),
                node: node.clone(),
            };
        }
    }
    let mut found = None;
    for profile_id in string_list(doc, "extends") {
        if let Some(node) = profiles
            .get(&profile_id)
            .and_then(|profile| profile.get("policy"))
            .and_then(|policy| policy.get("preset"))
        {
            if node.as_str().is_some() {
                found = Some(node.clone());
            }
        }
    }
    if let Some(node) = found {
        return Choice {
            id: preset_id(node.as_str().unwrap_or("")),
            node,
        };
    }
    Choice {
        id: preset_id(global.as_str().unwrap_or("")),
        node: global.clone(),
    }
}

fn preset_id(raw: &str) -> String {
    if raw.contains('/') {
        raw.to_string()
    } else {
        format!("preset/{raw}")
    }
}

struct ModuleInputs<'a> {
    common: &'a Option<Node>,
    defaults: Option<&'a Node>,
    baseline: &'a Node,
    preset_policy: &'a Node,
    choice: &'a Choice,
    profiles: &'a BTreeMap<String, Node>,
}

fn merge_module(
    doc: &Node,
    kind: DocKind,
    inputs: &ModuleInputs<'_>,
    diags: &mut Vec<Diagnostic>,
) -> Node {
    let mut acc = Node::map(&doc.file, doc.line);
    if let Some(common) = inputs.common {
        absorb(&mut acc, common, kind, diags);
    }
    if let Some(defaults) = inputs.defaults {
        absorb(&mut acc, defaults, kind, diags);
    }
    let mut holder = Node::map(&inputs.baseline.file, inputs.baseline.line);
    if let Err(message) = merge_policy(&mut holder, inputs.baseline, kind, false) {
        diags.push(diag(&inputs.baseline.file, inputs.baseline.line, message));
    }
    attach_policy(&mut acc, holder);
    let mut holder = Node::map(&inputs.preset_policy.file, inputs.preset_policy.line);
    if let Err(message) = merge_policy(&mut holder, inputs.preset_policy, kind, false) {
        diags.push(diag(&doc.file, doc.line, message));
    }
    attach_policy(&mut acc, holder);
    for profile_id in string_list(doc, "extends") {
        match inputs.profiles.get(&profile_id) {
            Some(profile) => absorb(&mut acc, &without_identity(profile), kind, diags),
            None => diags.push(diag(
                &doc.file,
                doc.line,
                format!("profile 不存在: {profile_id}"),
            )),
        }
    }
    absorb(&mut acc, doc, kind, diags);
    if acc.get("policy").is_none() {
        set_key(&mut acc, "policy", Node::map(&doc.file, doc.line));
    }
    if let Some(mut policy) = acc.get("policy").cloned() {
        set_key(&mut policy, "preset", inputs.choice.node.clone());
        set_key(&mut acc, "policy", policy);
    }
    if acc.get("enabled").is_none()
        && matches!(
            kind,
            DocKind::Source
                | DocKind::Accessor
                | DocKind::Adapter
                | DocKind::AdapterBinding
                | DocKind::Type
        )
    {
        set_key(
            &mut acc,
            "enabled",
            Node {
                file: String::new(),
                line: 0,
                preset_id: None,
                data: Data::Bool(true),
            },
        );
    }
    if kind == DocKind::Source {
        default_render(&mut acc);
    }
    acc
}

fn attach_policy(dst: &mut Node, policy: Node) {
    match dst.get("policy").cloned() {
        Some(previous) => {
            if let Ok(merged) = merge(previous, policy.clone()) {
                set_key(dst, "policy", merged);
            } else {
                set_key(dst, "policy", policy);
            }
        }
        None => set_key(dst, "policy", policy),
    }
}

fn absorb(dst: &mut Node, src: &Node, kind: DocKind, diags: &mut Vec<Diagnostic>) {
    let Some(entries) = src.as_map() else {
        diags.push(diag(&src.file, src.line, "配置必须是映射"));
        return;
    };
    for (key, value) in entries {
        if key == "policy" {
            let mut holder = dst
                .get("policy")
                .cloned()
                .unwrap_or_else(|| Node::map(&value.file, value.line));
            if let Err(message) = merge_policy(&mut holder, value, kind, false) {
                diags.push(diag(&value.file, value.line, message));
            }
            set_key(dst, "policy", holder);
            continue;
        }
        match dst.get(key).cloned() {
            Some(previous) => match merge(previous, value.clone()) {
                Ok(merged) => set_key(dst, key, merged),
                Err(message) => diags.push(diag(&value.file, value.line, message)),
            },
            None => set_key(dst, key, value.clone()),
        }
    }
}

fn resolve_presets(
    presets: &BTreeMap<String, Node>,
    diags: &mut Vec<Diagnostic>,
) -> BTreeMap<String, Node> {
    let mut out = BTreeMap::new();
    for id in presets.keys() {
        let mut stack = Vec::new();
        if let Some(policy) = resolve_preset(id, presets, &mut stack, diags) {
            out.insert(id.clone(), policy);
        }
    }
    out
}

fn resolve_preset(
    id: &str,
    presets: &BTreeMap<String, Node>,
    stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Node> {
    if stack.iter().any(|item| item == id) {
        diags.push(diag("", 0, format!("预设继承成环: {id}")));
        return None;
    }
    let doc = presets.get(id)?;
    stack.push(id.to_string());
    let mut acc = Node::map(&doc.file, doc.line);
    for parent in string_list(doc, "extends") {
        if !presets.contains_key(&parent) {
            diags.push(diag(&doc.file, doc.line, format!("预设不存在: {parent}")));
            continue;
        }
        if let Some(policy) = resolve_preset(&parent, presets, stack, diags) {
            match merge(acc.clone(), policy) {
                Ok(merged) => acc = merged,
                Err(message) => diags.push(diag(&doc.file, doc.line, message)),
            }
        }
    }
    if let Some(policy) = doc.get("policy") {
        match merge(acc.clone(), policy.clone()) {
            Ok(merged) => acc = merged,
            Err(message) => diags.push(diag(&policy.file, policy.line, message)),
        }
    }
    stack.pop();
    Some(acc)
}

fn resolve_profiles(
    profiles: &BTreeMap<String, Node>,
    diags: &mut Vec<Diagnostic>,
) -> BTreeMap<String, Node> {
    let mut out = BTreeMap::new();
    for id in profiles.keys() {
        let mut stack = Vec::new();
        if let Some(body) = resolve_profile(id, profiles, &mut stack, diags) {
            out.insert(id.clone(), body);
        }
    }
    out
}

fn resolve_profile(
    id: &str,
    profiles: &BTreeMap<String, Node>,
    stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Node> {
    if stack.iter().any(|item| item == id) {
        diags.push(diag("", 0, format!("profile 继承成环: {id}")));
        return None;
    }
    let doc = profiles.get(id)?;
    stack.push(id.to_string());
    let mut acc = Node::map(&doc.file, doc.line);
    for parent in string_list(doc, "extends") {
        if !parent.starts_with("profile/") || !profiles.contains_key(&parent) {
            diags.push(diag(
                &doc.file,
                doc.line,
                format!("profile 不存在: {parent}"),
            ));
            continue;
        }
        if let Some(body) = resolve_profile(&parent, profiles, stack, diags) {
            match merge(acc.clone(), without_identity(&body)) {
                Ok(merged) => acc = merged,
                Err(message) => diags.push(diag(&doc.file, doc.line, message)),
            }
        }
    }
    match merge(acc.clone(), without_identity(doc)) {
        Ok(merged) => acc = merged,
        Err(message) => diags.push(diag(&doc.file, doc.line, message)),
    }
    stack.pop();
    Some(acc)
}

fn index(docs: Vec<Parsed>, diags: &mut Vec<Diagnostic>) -> Indexed {
    let mut indexed = Indexed {
        system: Node::map("", 0),
        system_overlay: None,
        defaults_common: None,
        defaults: BTreeMap::new(),
        presets: BTreeMap::new(),
        profiles: BTreeMap::new(),
        modules: BTreeMap::new(),
    };
    let mut seen = HashSet::new();
    for doc in docs {
        let kind = match kind_of(&doc.logical) {
            Ok(kind) => kind,
            Err(message) => {
                diags.push(diag(&doc.node.file, 1, message));
                continue;
            }
        };
        let defaults_for = defaults_target(&doc.logical);
        check_top_level(&doc.node, kind, diags);
        if let Some(overlay) = &doc.system_overlay {
            check_top_level(overlay, DocKind::System, diags);
            check_policy_keys(overlay, DocKind::System, None, diags);
        }
        check_policy_keys(&doc.node, kind, defaults_for, diags);
        if kind == DocKind::Defaults {
            match defaults_for {
                None if is_common(&doc.logical) => indexed.defaults_common = Some(doc.node),
                Some(target) => {
                    indexed.defaults.insert(target, doc.node);
                }
                None => diags.push(diag(&doc.node.file, 1, "未知的 defaults 文件")),
            }
            continue;
        }
        if kind == DocKind::System {
            check_kind_field(&doc.node, "system", diags);
            indexed.system = doc.node;
            indexed.system_overlay = doc.system_overlay;
            continue;
        }
        let Some(id_node) = doc.node.get("id") else {
            diags.push(diag(&doc.node.file, doc.node.line, "缺少 id"));
            continue;
        };
        let Some(id) = id_node.as_str() else {
            diags.push(diag(&id_node.file, id_node.line, "id 必须是字符串"));
            continue;
        };
        let id = id.to_string();
        if let Err(message) = check_id(&id, kind) {
            diags.push(diag(&id_node.file, id_node.line, message));
        }
        let expected = kind_name(kind);
        check_kind_field(&doc.node, expected, diags);
        if !seen.insert(id.clone()) {
            diags.push(diag(
                &doc.node.file,
                id_node.line,
                format!("重复的 id: {id}"),
            ));
        }
        let mut node = doc.node;
        if kind == DocKind::Preset {
            if let Some(policy) = node.get("policy").cloned() {
                let mut policy = policy;
                policy.stamp_preset(&id);
                set_key(&mut node, "policy", policy);
            } else {
                diags.push(diag(&node.file, node.line, "预设缺少 policy"));
            }
            indexed.presets.insert(id, node);
        } else if kind == DocKind::Profile {
            indexed.profiles.insert(id, node);
        } else {
            indexed.modules.insert(id, node);
        }
    }
    if indexed.system.file.is_empty() {
        diags.push(diag("system.yaml", 1, "缺少 system.yaml"));
    }
    indexed
}

fn lower(
    built: Built,
    schemas: &BTreeMap<String, Value>,
    root: &Path,
    diags: &mut Vec<Diagnostic>,
) -> Snapshot {
    let (system_value, system_origins) = built.system.lower(Some(&built.global_preset));
    let mut origins = BTreeMap::new();
    for (path, origin) in system_origins {
        origins.insert(("system".to_string(), path), origin);
    }
    let mut modules = BTreeMap::new();
    for (id, node) in &built.modules {
        let selected = built.selected.get(id).map(String::as_str);
        let (value, module_origins) = node.lower(selected);
        for (path, origin) in module_origins {
            origins.insert((id.clone(), path), origin);
        }
        modules.insert(id.clone(), value);
    }
    attach_schema(
        &schema::validate_system(&system_value),
        "system",
        &origins,
        diags,
    );
    for (id, module) in &modules {
        attach_schema(&schema::validate_module(module), id, &origins, diags);
        if let Some(pointer) = module.pointer("/spec/$schema").and_then(Value::as_str) {
            if let Some(spec_schema) = schemas.get(pointer) {
                let mut spec = module.get("spec").cloned().unwrap_or(Value::Null);
                if let Some(object) = spec.as_object_mut() {
                    object.remove("$schema");
                }
                attach_schema(
                    &schema::validate_named(spec_schema, &spec),
                    id,
                    &origins,
                    diags,
                );
            }
        }
    }
    let baseline = built.baseline_policy.lower(None).0;
    let locks = system_value
        .pointer("/policy/locks")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let system_policy = system_value
        .get("policy")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    diags.extend(check_modules(
        &modules,
        &system_policy,
        &baseline,
        &locks,
        schemas,
        root,
    ));
    Snapshot {
        generation: 0,
        system: system_value,
        modules,
        origins,
    }
}

fn attach_schema(
    failures: &[SchemaFailure],
    owner: &str,
    origins: &BTreeMap<(String, String), crate::node::Origin>,
    diags: &mut Vec<Diagnostic>,
) {
    for failure in failures {
        let path = failure.pointer.trim_start_matches('/').replace('/', ".");
        let origin = origins.get(&(owner.to_string(), path));
        diags.push(Diagnostic {
            file: origin.map(|item| item.file.clone()).unwrap_or_default(),
            line: origin.map(|item| item.line).unwrap_or(0),
            message: failure.message.clone(),
        });
    }
}

fn parse_docs(files: Vec<Input>, diags: &mut Vec<Diagnostic>) -> Vec<Parsed> {
    let mut grouped: BTreeMap<String, (Option<Node>, Option<Node>)> = BTreeMap::new();
    for file in files {
        match yaml::parse(&file.actual, &file.text) {
            Ok(node) => {
                let entry = grouped.entry(file.logical.clone()).or_default();
                let slot = if file.overlay {
                    &mut entry.1
                } else {
                    &mut entry.0
                };
                if slot.is_some() {
                    diags.push(diag(
                        &file.actual,
                        1,
                        format!("重复的文件: {}", file.logical),
                    ));
                } else {
                    *slot = Some(node);
                }
            }
            Err(message) => diags.push(diag(&file.actual, 1, message)),
        }
    }
    let mut docs = Vec::new();
    for (logical, (base, overlay)) in grouped {
        let is_system = logical == "system.yaml" || logical == "system.yml";
        let node = match (base, overlay) {
            (Some(base), Some(over)) if is_system => {
                docs.push(Parsed {
                    logical,
                    node: base,
                    system_overlay: Some(over),
                });
                continue;
            }
            (Some(base), Some(over)) => match merge(base, over.clone()) {
                Ok(merged) => merged,
                Err(message) => {
                    diags.push(diag(&over.file, over.line, message));
                    continue;
                }
            },
            (Some(base), None) => base,
            (None, Some(over)) => over,
            (None, None) => continue,
        };
        docs.push(Parsed {
            logical,
            node,
            system_overlay: None,
        });
    }
    docs
}

fn check_top_level(node: &Node, kind: DocKind, diags: &mut Vec<Diagnostic>) {
    let Some(entries) = node.as_map() else {
        diags.push(diag(&node.file, node.line, "配置必须是映射"));
        return;
    };
    let allowed = top_level_keys(kind);
    for (key, child) in entries {
        if !allowed.iter().any(|name| *name == key) {
            diags.push(diag(&child.file, child.line, format!("未知字段 {key}")));
        }
    }
}

fn check_policy_keys(
    node: &Node,
    kind: DocKind,
    defaults_for: Option<DocKind>,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(policy) = node.get("policy") else {
        return;
    };
    let mut found = Vec::new();
    leaves(policy, "", &mut found);
    for leaf in found {
        if leaf.path == "locks" {
            if kind != DocKind::System {
                diags.push(diag(leaf.file, leaf.line, "策略字段低于作用域: locks"));
            }
            continue;
        }
        if leaf.path == "preset" {
            if kind == DocKind::Preset {
                diags.push(diag(leaf.file, leaf.line, "预设不能写出 policy.preset"));
            }
            continue;
        }
        match scope_of(&leaf.path) {
            None => diags.push(diag(
                leaf.file,
                leaf.line,
                format!("未知策略字段 {}", leaf.path),
            )),
            Some(scope) if !allows_write(kind, scope, defaults_for) => diags.push(diag(
                leaf.file,
                leaf.line,
                format!("策略字段低于作用域: {}", leaf.path),
            )),
            Some(_) => {}
        }
    }
}

fn check_kind_field(node: &Node, expected: &str, diags: &mut Vec<Diagnostic>) {
    match node.get("kind").and_then(Node::as_str) {
        Some(actual) if actual == expected => {}
        Some(actual) => diags.push(diag(
            &node.file,
            node.line,
            format!("kind 应为 {expected}，实际为 {actual}"),
        )),
        None => diags.push(diag(&node.file, node.line, "缺少 kind")),
    }
}

fn check_id(id: &str, kind: DocKind) -> Result<(), String> {
    let Some((prefix, name)) = id.split_once('/') else {
        return Err(format!("id 格式应为 {0}/<name>: {id}", kind_name(kind)));
    };
    if prefix != kind_name(kind) || name.is_empty() || name.contains('/') {
        return Err(format!("id 与所在目录不一致: {id}"));
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(format!("id 与所在目录不一致: {id}"));
    };
    if !first.is_ascii_alphanumeric()
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
    {
        return Err(format!("id 含有非法字符: {id}"));
    }
    Ok(())
}

/// 不在配置快照内的声明路径。
///
/// 具体名单由 [`layout`] 维护：`schemas/` 与 `models/` 是子系统，`workflows/` 是已下线
/// 功能的遗留。这些位置整棵跳过：加载不为它们的形状负责，也不能因为一个遗留文件让整次
/// 加载失败。
fn is_ignored_path(logical: &str) -> bool {
    layout::is_ignored(logical)
}

fn kind_of(logical: &str) -> Result<DocKind, String> {
    layout::kind_of(logical)
}

fn kind_name(kind: DocKind) -> &'static str {
    match kind {
        DocKind::System => "system",
        DocKind::Preset => "preset",
        DocKind::Profile => "profile",
        DocKind::Accessor => "accessor",
        DocKind::Adapter => "adapter",
        DocKind::AdapterBinding => "binding",
        DocKind::Source => "source",
        DocKind::Plan => "plan",
        DocKind::Type => "type",
        DocKind::Defaults => "defaults",
    }
}

fn kind_from_id(id: &str) -> Option<DocKind> {
    match id.split_once('/')?.0 {
        "accessor" => Some(DocKind::Accessor),
        "adapter" => Some(DocKind::Adapter),
        "binding" => Some(DocKind::AdapterBinding),
        "source" => Some(DocKind::Source),
        "plan" => Some(DocKind::Plan),
        "type" => Some(DocKind::Type),
        "profile" => Some(DocKind::Profile),
        "preset" => Some(DocKind::Preset),
        _ => None,
    }
}

fn defaults_target(logical: &str) -> Option<DocKind> {
    let name = logical.rsplit('/').next().unwrap_or(logical);
    match name {
        "accessor.yaml" | "accessor.yml" => Some(DocKind::Accessor),
        "adapter.yaml" | "adapter.yml" => Some(DocKind::Adapter),
        "source.yaml" | "source.yml" => Some(DocKind::Source),
        _ => None,
    }
}

fn is_common(logical: &str) -> bool {
    matches!(
        logical.rsplit('/').next(),
        Some("common.yaml" | "common.yml")
    )
}

fn without_policy(node: &Node) -> Node {
    let Some(entries) = node.as_map() else {
        return node.clone();
    };
    let kept = entries
        .iter()
        .filter(|(key, _)| key != "policy")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Node {
        file: node.file.clone(),
        line: node.line,
        preset_id: node.preset_id.clone(),
        data: Data::Map(kept),
    }
}

fn string_list(node: &Node, key: &str) -> Vec<String> {
    let Some(value) = node.get(key) else {
        return Vec::new();
    };
    let Data::Array(items) = &value.data else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

fn string_node(template: &Node, text: &str) -> Node {
    Node {
        file: template.file.clone(),
        line: template.line,
        preset_id: None,
        data: Data::String(text.to_string()),
    }
}

fn set_key(node: &mut Node, key: &str, value: Node) {
    if !matches!(node.data, Data::Map(_)) {
        *node = Node::map(value.file.clone(), value.line);
    }
    let Data::Map(entries) = &mut node.data else {
        return;
    };
    if let Some(index) = entries.iter().position(|(name, _)| name == key) {
        entries[index].1 = value;
    } else {
        entries.push((key.to_string(), value));
    }
}

fn default_render(doc: &mut Node) {
    let Some(access) = doc.get("access").cloned() else {
        return;
    };
    if access.get("render").is_some() {
        return;
    }
    let mut access = access;
    set_key(
        &mut access,
        "render",
        Node {
            file: String::new(),
            line: 0,
            preset_id: None,
            data: Data::String("auto".into()),
        },
    );
    set_key(doc, "access", access);
}

fn diag(file: &str, line: u32, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        file: file.to_string(),
        line,
        message: message.into(),
    }
}

fn read_tree(
    dir: &Path,
    overlay: Option<&str>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Vec<Input>, Error> {
    let mut out = Vec::new();
    walk(dir, dir, overlay, &mut out, warnings)?;
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    overlay: Option<&str>,
    out: &mut Vec<Input>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Read {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Read {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            let rel = rel_path(root, &path);
            if rel == "overlays" && overlay.is_none() {
                continue;
            }
            // 非声明子树整棵跳过，不进去报错。
            if layout::is_ignored(&rel) {
                continue;
            }
            walk(root, &path, overlay, out, warnings)?;
            continue;
        }
        let rel = rel_path(root, &path);
        // `dictionary.yaml` 是语义字典覆盖，不描述任何能力声明：由领域 API 的 `/dictionary`
        // 直接读取，配置加载只跳过，不把它写进配置快照。
        if rel.rsplit('/').next() == Some("dictionary.yaml") {
            continue;
        }
        // `schemas/` 由专门的读取器处理；子系统目录由各自的领域模块读取，
        // 配置加载只负责跳过，避免把它们的形状写进快照 schema。
        if is_ignored_path(&rel) {
            continue;
        }
        if let Some(rest) = rel.strip_prefix("overlays/") {
            let Some((env, tail)) = rest.split_once('/') else {
                continue;
            };
            if Some(env) != overlay || !is_yaml(tail) {
                continue;
            }
            if kind_of(tail).is_err() {
                warnings.push(diag(&rel, 1, format!("未知的配置路径，已跳过: {tail}")));
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|source| Error::Read {
                path: path.display().to_string(),
                source,
            })?;
            out.push(Input {
                logical: tail.to_string(),
                actual: rel,
                text,
                overlay: true,
            });
            continue;
        }
        if !is_yaml(&rel) {
            continue;
        }
        if kind_of(&rel).is_err() {
            warnings.push(diag(&rel, 1, format!("未知的配置路径，已跳过: {rel}")));
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.display().to_string(),
            source,
        })?;
        out.push(Input {
            logical: rel.clone(),
            actual: rel,
            text,
            overlay: false,
        });
    }
    Ok(())
}

fn read_schemas(dir: &Path, diags: &mut Vec<Diagnostic>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let root = dir.join("schemas");
    if !root.is_dir() {
        return out;
    }
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(source) => {
                diags.push(diag(&current.display().to_string(), 0, source.to_string()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let rel = rel_path(dir, &path);
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    diags.push(diag(&rel, 0, error.to_string()));
                    continue;
                }
            };
            match serde_json::from_str::<Value>(&text) {
                Ok(value) => {
                    out.insert(rel, value);
                }
                Err(error) => diags.push(diag(
                    &rel_path(dir, &path),
                    1,
                    format!("schema 不是 JSON: {error}"),
                )),
            }
        }
    }
    out
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_yaml(path: &str) -> bool {
    path.ends_with(".yaml") || path.ends_with(".yml")
}
