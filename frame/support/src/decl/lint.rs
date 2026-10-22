//! 能力声明包校验（结构 + 交叉引用 + 源码一致性）。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::ability::{AbilityDecl, RuntimeKind};
use super::package::AbilityPackageMeta;
use super::protocol::ProtocolKind;
use crate::manifest::EventDir;

/// 校验选项。
#[derive(Clone, Debug)]
pub struct LintOptions {
    /// 已知能力 id 集；用于校验 `depends`。为 `None` 时跳过。
    pub known_ability_ids: Option<HashSet<String>>,
    /// handler 模式是否检查 `src/logic/<method>.rs` 存在。
    pub check_logic_modules: bool,
    /// 是否扫描 `src/`（不含 `generated/`）中 `ctx.emit("…")` 与声明对齐。
    pub scan_emit_in_source: bool,
    /// 是否扫描域内出站 topic 字符串与 `outbound_topics` 对齐。
    pub scan_outbound_topics_in_source: bool,
    /// 将 warning 视为 error（用于 CI / 全量校验）。
    pub strict: bool,
}

impl Default for LintOptions {
    fn default() -> Self {
        Self {
            known_ability_ids: None,
            check_logic_modules: true,
            scan_emit_in_source: true,
            scan_outbound_topics_in_source: true,
            strict: false,
        }
    }
}

impl LintOptions {
    /// 全仓库声明校验：交叉引用 + 源码一致性 + warning 视为 error。
    pub fn workspace_strict() -> Self {
        Self {
            strict: true,
            ..Self::default()
        }
    }
}

/// 单能力包校验结果。
#[derive(Clone, Debug)]
pub struct LintReport {
    pub root: PathBuf,
    pub ability_id: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl LintReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 校验已加载的能力包。
pub fn lint_ability_package(meta: &AbilityPackageMeta, opts: &LintOptions) -> LintReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let decl = &meta.decl;
    let method_ids: HashSet<&str> = decl.methods.iter().map(|m| m.id.as_str()).collect();

    lint_protocols(decl, &method_ids, meta, &mut errors);
    lint_depends(decl, opts, &mut errors);
    lint_factory_config(decl, &mut errors);
    lint_runtime(meta, &mut errors);
    if opts.check_logic_modules && decl.uses_handlers() && meta.runtime_kind == RuntimeKind::Rust {
        lint_logic_modules(meta.root.as_path(), decl, &mut errors);
    }
    if opts.scan_emit_in_source {
        lint_emit_in_source(meta.root.as_path(), decl, &mut errors, &mut warnings);
    }
    lint_emit_payload_schema(decl, &mut errors);
    if opts.scan_outbound_topics_in_source && !decl.outbound_topics.is_empty() {
        lint_outbound_topics_in_source(meta.root.as_path(), decl, &mut errors, &mut warnings);
    }

    if opts.strict {
        errors.extend(warnings.drain(..));
    }

    LintReport {
        root: meta.root.clone(),
        ability_id: decl.id.clone(),
        errors,
        warnings,
    }
}

/// 发现 workspace 下全部能力根目录。
///
/// - `abilitys/<category>/<name>/ability.yml`：通用能力
/// - `apps/<app>/ability/<name>/ability.yml`：场景应用自有能力
pub fn discover_ability_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    collect_ability_roots(&workspace.join("abilitys"), workspace, &mut roots);
    let apps_dir = workspace.join("apps");
    if let Ok(apps) = fs::read_dir(&apps_dir) {
        for app in apps.flatten() {
            collect_ability_roots(&app.path().join("ability"), workspace, &mut roots);
        }
    }
    roots.sort();
    roots
}

fn collect_ability_roots(ability_dir: &Path, workspace: &Path, roots: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(ability_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let rel = path
            .strip_prefix(workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.contains("/templates/") {
            continue;
        }
        if path.join("ability.yml").is_file() {
            roots.push(path);
        } else {
            collect_ability_roots(&path, workspace, roots);
        }
    }
}

/// 加载并校验 workspace 内全部能力声明。
pub fn lint_workspace(workspace: &Path, opts: &LintOptions) -> Result<Vec<LintReport>, String> {
    let roots = discover_ability_roots(workspace);
    if roots.is_empty() {
        return Err(format!(
            "未在 {} 下发现 abilitys 或 apps/*/ability",
            workspace.display()
        ));
    }

    let mut packages = Vec::new();
    for root in &roots {
        let meta = super::load::load_ability_package(root)?;
        packages.push(meta);
    }

    let mut known: HashSet<String> = packages.iter().map(|p| p.decl.id.clone()).collect();
    if let Some(extra) = &opts.known_ability_ids {
        known.extend(extra.iter().cloned());
    }
    let mut effective = opts.clone();
    effective.known_ability_ids = Some(known);

    Ok(packages
        .iter()
        .map(|meta| lint_ability_package(meta, &effective))
        .collect())
}

/// 合并报告；任一 error 则返回 `Err`。
pub fn assert_clean_reports(reports: &[LintReport]) -> Result<(), String> {
    let mut lines = Vec::new();
    for r in reports {
        for e in &r.errors {
            lines.push(format!("{} ({}): {e}", r.root.display(), r.ability_id));
        }
        for w in &r.warnings {
            lines.push(format!(
                "{} ({}): [warn] {w}",
                r.root.display(),
                r.ability_id
            ));
        }
    }
    let has_error = reports.iter().any(|r| !r.errors.is_empty());
    if has_error {
        return Err(lines.join("\n"));
    }
    Ok(())
}

fn lint_runtime(meta: &AbilityPackageMeta, errors: &mut Vec<String>) {
    super::runtime_resolve::lint_runtime_artifacts(&meta.root, meta.runtime_kind, errors);
}

fn lint_protocols(
    _decl: &AbilityDecl,
    method_ids: &HashSet<&str>,
    meta: &AbilityPackageMeta,
    errors: &mut Vec<String>,
) {
    if let Some(mcp) = &meta.mcp {
        for tool in &mcp.tools {
            if !method_ids.contains(tool.method.as_str()) {
                errors.push(format!(
                    "mcp/info.yml tools[].method `{}` 未在 ability.yml methods 中声明",
                    tool.method
                ));
            }
        }
    }
    if let Some(tools) = &meta.tools {
        for tool in &tools.tools {
            if !method_ids.contains(tool.method.as_str()) {
                errors.push(format!(
                    "tools/info.yml tools[].method `{}` 未在 ability.yml methods 中声明",
                    tool.method
                ));
            }
        }
    }
    if let Some(grpc) = &meta.grpc {
        for rpc in &grpc.rpcs {
            if !method_ids.contains(rpc.method.as_str()) {
                errors.push(format!(
                    "grpc/info.yml rpcs[].method `{}` 未在 ability.yml methods 中声明",
                    rpc.method
                ));
            }
        }
    }
    if let Some(cli) = &meta.cli {
        if !method_ids.contains(cli.entry.as_str()) {
            errors.push(format!(
                "cli/info.yml entry `{}` 未在 ability.yml methods 中声明",
                cli.entry
            ));
        }
    }
    if meta.protocols.contains(&ProtocolKind::Skill) {
        if let Some(skill) = &meta.skill {
            if skill.name.is_empty() {
                errors.push("skill/info.yml name 不可为空".into());
            }
        }
    }
    let tools_info = meta.root.join("tools").join("info.yml");
    if tools_info.is_file() && !meta.protocols.contains(&ProtocolKind::Tools) {
        errors.push("存在 tools/info.yml 但 ability.yml protocols 未声明 tools".into());
    }
}

fn lint_depends(decl: &AbilityDecl, opts: &LintOptions, errors: &mut Vec<String>) {
    let Some(known) = &opts.known_ability_ids else {
        return;
    };
    for dep in &decl.depends {
        let id = match dep {
            super::ability::DepDecl::Simple(id) => id.as_str(),
            super::ability::DepDecl::Full { id, .. } => id.as_str(),
        };
        if !known.contains(id) {
            errors.push(format!("depends 引用未知能力 id: {id}"));
        }
    }
}

fn lint_factory_config(decl: &AbilityDecl, errors: &mut Vec<String>) {
    if decl.factory && decl.config_schema.is_null() {
        errors.push("factory: true 时须提供 config_schema".into());
    }
}

fn lint_logic_modules(root: &Path, decl: &AbilityDecl, errors: &mut Vec<String>) {
    let logic_dir = root.join("src").join("logic");
    for m in &decl.methods {
        let module = AbilityDecl::logic_module_name(&m.id);
        let path = logic_dir.join(format!("{module}.rs"));
        if !path.is_file() {
            errors.push(format!(
                "handler 模式缺少 src/logic/{module}.rs（method `{}`）",
                m.id
            ));
        }
    }
}

fn lint_emit_payload_schema(decl: &AbilityDecl, errors: &mut Vec<String>) {
    use crate::schema::is_unrestricted_schema;

    for ev in decl.events.iter().filter(|e| e.dir == EventDir::Emit) {
        let schema = &ev.payload_schema;
        if is_unrestricted_schema(schema) {
            errors.push(format!(
                "events `{}` (emit) 缺少 payload_schema 或 schema 为空，应声明载荷结构",
                ev.name
            ));
            continue;
        }
        if schema.get("type").is_none() {
            errors.push(format!(
                "events `{}` payload_schema 缺少 type 字段",
                ev.name
            ));
        }
    }
}

fn lint_emit_in_source(
    root: &Path,
    decl: &AbilityDecl,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let declared: HashSet<String> = decl
        .events
        .iter()
        .filter(|e| e.dir == EventDir::Emit)
        .map(|e| e.name.clone())
        .collect();
    let found = scan_quoted_calls(root, &["ctx.emit("]);
    for name in &found {
        if !declared.contains(name) {
            errors.push(format!(
                "源码 emit `{name}` 未在 ability.yml events (dir: emit) 中声明"
            ));
        }
    }
    for name in &declared {
        if !found.contains(name) {
            warnings.push(format!(
                "events 声明 emit `{name}` 但源码中未发现 ctx.emit 调用"
            ));
        }
    }
}

fn lint_outbound_topics_in_source(
    root: &Path,
    decl: &AbilityDecl,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let declared: HashSet<String> = decl
        .outbound_topics
        .iter()
        .map(|t| t.name.clone())
        .collect();
    let found = scan_topic_literals(root);
    for name in &found {
        if !declared.contains(name) {
            errors.push(format!(
                "源码出站 topic `{name}` 未在 ability.yml outbound_topics 中声明"
            ));
        }
    }
    for name in &declared {
        if !found.contains(name) {
            warnings.push(format!(
                "outbound_topics 声明 `{name}` 但源码中未发现对应字符串字面量"
            ));
        }
    }
}

/// 扫描 `src/` 下非 generated 源码中的 emit 调用（支持跨行）。
fn scan_quoted_calls(root: &Path, prefixes: &[&str]) -> HashSet<String> {
    let src = root.join("src");
    let mut out = HashSet::new();
    walk_rs_sources(&src, &mut |path, content| {
        if path.to_string_lossy().contains("generated") {
            return;
        }
        for prefix in prefixes {
            let mut search = content;
            while let Some(idx) = search.find(prefix) {
                let rest = &search[idx + prefix.len()..];
                if let Some(name) = parse_string_literal(rest) {
                    out.insert(name);
                }
                search = &search[idx + prefix.len()..];
            }
        }
    });
    out
}

/// 扫描形如 `"kb.xxx.yyy"` 的 topic 字面量（域内出站）。
fn scan_topic_literals(root: &Path) -> HashSet<String> {
    let src = root.join("src");
    let mut out = HashSet::new();
    walk_rs_sources(&src, &mut |path, content| {
        if path.to_string_lossy().contains("generated") {
            return;
        }
        for cap in extract_quoted_strings(content) {
            if cap.contains('.') && !cap.contains(' ') && cap.len() > 3 {
                // 仅收集显式 topic 形态（含点号的路径式名称）
                if cap.starts_with("kb.") || cap.contains(".miu.") {
                    out.insert(cap);
                }
            }
        }
    });
    out
}

fn walk_rs_sources(dir: &Path, f: &mut dyn FnMut(&Path, &str)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_sources(&path, f);
        } else if path.extension().is_some_and(|e| e == "rs") {
            if let Ok(content) = fs::read_to_string(&path) {
                f(&path, &content);
            }
        }
    }
}

fn parse_string_literal(s: &str) -> Option<String> {
    let s = s.trim_start();
    let q = s.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}

fn extract_quoted_strings(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut s = String::new();
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '"' {
                    break;
                }
                if nc == '\\' {
                    if let Some(esc) = chars.next() {
                        s.push(esc);
                    }
                } else {
                    s.push(nc);
                }
            }
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_emit_literal() {
        assert_eq!(
            parse_string_literal(r#""changed", payload)"#),
            Some("changed".into())
        );
    }
}
