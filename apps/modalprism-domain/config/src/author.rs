//! 声明文件的可视化编辑。
//!
//! 信源、访问器与适配器都是 YAML 声明。这个模块让界面不再依赖编译：按模板生成草稿、
//! 写入文件、立即用同一套加载器校验，失败时把磁盘恢复到写入前的状态。
//!
//! 内置适配器的判定不写死清单：`implementation` 为 `builtin:<name>` 且 id 恰好是
//! `adapter/<name>` 的声明就是系统自带的那一份，可以修改、可以停用，但不能删除。

use std::borrow::Cow;
use std::fs;
use std::path::Path;

use ordered_float::OrderedFloat;
use saphyr::{Mapping, Scalar, Yaml, YamlEmitter};
use serde_json::{Map, Value, json};

use crate::error::{Diagnostic, Diagnostics, Error};
use crate::load::{LoadOptions, validate};
use crate::yaml;

/// 一次创建或改写请求。
pub struct Draft {
    pub kind: String,
    pub slug: String,
    pub doc: Value,
}

/// 写入结果。`file` 是相对配置目录的路径。
pub struct Saved {
    pub id: String,
    pub file: String,
}

impl Draft {
    /// 解析界面传来的草稿：`{kind, slug, doc}`。
    pub fn parse(value: &Value) -> Result<Self, Error> {
        let kind = text(value, "kind")?;
        let slug = text(value, "slug")?;
        let doc = value
            .get("doc")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        if !doc.is_object() {
            return Err(invalid("", "声明必须是对象"));
        }
        Ok(Self { kind, slug, doc })
    }
}

/// 界面上的模板。`doc` 是可以直接提交给 [`create`] 的草稿正文。
pub fn templates() -> Value {
    Value::Array(vec![
        template(
            "source",
            "source-rss",
            "RSS / Atom 订阅",
            "抓取一个 RSS 或 Atom 地址，按条目入库。最省心的信源形态。",
            json!({
                "name": "新的 RSS 信源",
                "access": {
                    "accessor": "accessor/rss",
                    "locator": {"url": "https://example.com/feed.xml"}
                },
                "pipeline": [{"run": ["adapter/feed-items"]}],
                "schedule": {"interval": "2h"},
                "admission": {"initialBackfill": {"limit": 8, "maxAge": "365d"}}
            }),
        ),
        template(
            "source",
            "source-web",
            "网页列表",
            "抓取一个列表页，用 CSS 选择器取出条目链接。适合没有订阅源的新站。",
            json!({
                "name": "新的网页信源",
                "access": {
                    "accessor": "accessor/http",
                    "locator": {"url": "https://example.com/news"}
                },
                "pipeline": [{
                    "run": ["adapter/html-list"],
                    "spec": {"adapter/html-list": {
                        "mode": "html",
                        "item": "article",
                        "link": "a",
                        "title": "a"
                    }}
                }],
                "schedule": {"interval": "6h"},
                "admission": {"initialBackfill": {"limit": 10, "maxAge": "365d"}}
            }),
        ),
        template(
            "source",
            "source-search",
            "搜索查询",
            "把一个查询地址当作信源，每次抓取当前结果。适合持续跟踪关键词。",
            json!({
                "name": "新的搜索信源",
                "access": {
                    "accessor": "accessor/search",
                    "locator": {"query": "artificial intelligence"},
                    "spec": {"endpoint": "https://html.duckduckgo.com/html/?q={query}"}
                },
                "pipeline": [{
                    "run": ["adapter/html-list"],
                    "spec": {"adapter/html-list": {
                        "mode": "html",
                        "item": ".result",
                        "link": ".result__a",
                        "title": ".result__a"
                    }}
                }],
                "schedule": {"interval": "6h"},
                "admission": {}
            }),
        ),
        template(
            "source",
            "source-json",
            "JSON 接口",
            "直接消费一个返回列表的 JSON 接口，用 JSON Pointer 定位条目。",
            json!({
                "name": "新的 JSON 信源",
                "access": {
                    "accessor": "accessor/http",
                    "locator": {"url": "https://example.com/api/items"}
                },
                "pipeline": [{
                    "run": ["adapter/json-list"],
                    "spec": {"adapter/json-list": {
                        "items": "/items",
                        "urlTemplate": "{url}"
                    }}
                }],
                "schedule": {"interval": "6h"},
                "admission": {}
            }),
        ),
        template(
            "accessor",
            "accessor-http",
            "HTTP 访问器",
            "按普通 HTTP 请求取回页面。大多数信源都从这里开始。",
            json!({
                "name": "新的访问器",
                "implementation": "builtin:http",
                "limits": limits(45, 4, 2),
                "egress": {"profile": "direct"}
            }),
        ),
        template(
            "adapter",
            "adapter-list",
            "列表解析器",
            "把列表页拆成条目。复制这份声明即可为新站点定制选择器，无需改代码。",
            json!({
                "name": "新的列表解析器",
                "implementation": "builtin:html-list",
                "role": "search",
                "consumes": [{"kind": "raw", "part_role": "document", "media_type": "text/html"}],
                "produces": ["discovery"],
                "limits": limits(30, 1, 0),
                "spec": {"mode": "html", "item": "article", "link": "a", "title": "a"}
            }),
        ),
    ])
}

/// 新建一份声明，写入后立即校验；校验不通过时删除刚写入的文件。
pub fn create(dir: &Path, options: &LoadOptions, draft: &Draft) -> Result<Saved, Error> {
    let (folder, prefix) = folder_of(&draft.kind)?;
    let slug = check_slug(&draft.slug)?;
    let target = dir.join(folder).join(format!("{slug}.yaml"));
    let file = format!("{folder}/{slug}.yaml");
    if target.exists() {
        return Err(invalid(&file, "同名声明已经存在"));
    }
    let mut doc = draft
        .doc
        .as_object()
        .cloned()
        .ok_or_else(|| invalid(&file, "声明必须是对象"))?;
    doc.insert("id".into(), json!(format!("{prefix}/{slug}")));
    doc.insert("kind".into(), json!(prefix));
    stamp_source_type(prefix, &mut doc);
    doc.entry("name").or_insert_with(|| json!(slug));
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Read {
            path: parent.display().to_string(),
            source,
        })?;
    }
    write(&target, &Value::Object(doc))?;
    if let Err(error) = validate(dir, options) {
        let _ = fs::remove_file(&target);
        return Err(error);
    }
    Ok(Saved {
        id: format!("{prefix}/{slug}"),
        file,
    })
}

/// 改写一份已存在的声明；校验失败时恢复写入前的原文。
pub fn update(dir: &Path, options: &LoadOptions, draft: &Draft) -> Result<Saved, Error> {
    let (folder, prefix) = folder_of(&draft.kind)?;
    let slug = check_slug(&draft.slug)?;
    let target = dir.join(folder).join(format!("{slug}.yaml"));
    let file = format!("{folder}/{slug}.yaml");
    let original = fs::read_to_string(&target).map_err(|_| invalid(&file, "声明不存在"))?;
    let mut doc = draft
        .doc
        .as_object()
        .cloned()
        .ok_or_else(|| invalid(&file, "声明必须是对象"))?;
    doc.insert("id".into(), json!(format!("{prefix}/{slug}")));
    doc.insert("kind".into(), json!(prefix));
    stamp_source_type(prefix, &mut doc);
    write(&target, &Value::Object(doc))?;
    if let Err(error) = validate(dir, options) {
        restore(&target, &original)?;
        return Err(error);
    }
    Ok(Saved {
        id: format!("{prefix}/{slug}"),
        file,
    })
}

/// 落盘前为信源补齐有效 `sourceType`。
///
/// 类型是信源的意图层声明，缺省时由 access/pipeline 推导，推导规则只有
/// `source_type::derive` 一处。HTTP、MCP、命令行都经这里写盘，因此补齐放在这一层，
/// 而不是各入口各写一遍。已声明则原样保留：一致性、矩阵与付费限制仍由
/// `check_source` 在整库校验里判定，这里不做第二套校验。
fn stamp_source_type(prefix: &str, doc: &mut serde_json::Map<String, Value>) {
    if prefix != "source" || doc.contains_key("sourceType") {
        return;
    }
    let kind = crate::source_type::derive(&Value::Object(doc.clone()));
    doc.insert("sourceType".into(), json!(kind.as_str()));
}

/// 删除一份声明。内置适配器拒绝删除；被其它声明引用时由校验兜底并恢复原文。
///
/// 绑定额外带生命周期护栏（R5.2/R5.3）：仍被信源引用的绑定直接拒删并回显占用方；
/// 删除信源时级联清理其**独占**绑定，共享绑定保留。
pub fn remove(dir: &Path, options: &LoadOptions, kind: &str, slug: &str) -> Result<Saved, Error> {
    let (folder, prefix) = folder_of(kind)?;
    let slug = check_slug(slug)?;
    let target = dir.join(folder).join(format!("{slug}.yaml"));
    let file = format!("{folder}/{slug}.yaml");
    let original = fs::read_to_string(&target).map_err(|_| invalid(&file, "声明不存在"))?;
    let doc = yaml::parse_value(&file, &original).map_err(|message| invalid(&file, message))?;
    if prefix == "adapter" && builtin_adapter(&doc) {
        return Err(invalid(&file, "内置适配器不能删除，可以停用或改写"));
    }
    // R5.2：绑定被引用则拒删，错误里回显占用它的信源。
    if prefix == "binding" {
        let binding_id = format!("binding/{slug}");
        let occupiers = binding_referrer_source_ids(dir, &binding_id)?;
        if !occupiers.is_empty() {
            return Err(invalid(
                &file,
                format!("绑定仍被信源引用，不能删除: {}", occupiers.join(", ")),
            ));
        }
    }
    // R5.3：删除信源前先记下它引用的绑定，供删除后判定哪些变成孤儿。
    let referenced_bindings = if prefix == "source" {
        collect_binding_refs(&doc)
    } else {
        Vec::new()
    };
    fs::remove_file(&target).map_err(|_| invalid(&file, "删除失败"))?;
    if let Err(error) = validate(dir, options) {
        restore(&target, &original)?;
        return Err(error);
    }
    // R5.3：级联删除该信源独占、现已无人引用的绑定；共享绑定保留。
    if prefix == "source" {
        cascade_orphan_bindings(dir, options, &referenced_bindings)?;
    }
    Ok(Saved {
        id: format!("{prefix}/{slug}"),
        file,
    })
}

/// 从任意声明正文里收集 `binding/<slug>` 引用（数组/对象递归）。
fn collect_binding_refs(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_binding_refs(value, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk_binding_refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.starts_with("binding/") {
                out.push(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_binding_refs(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                walk_binding_refs(item, out);
            }
        }
        _ => {}
    }
}

/// 哪些信源引用了给定绑定 id（占用方，R5.1/R5.2）。
fn binding_referrer_source_ids(dir: &Path, binding_id: &str) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for entry in entries(dir, "source")? {
        let id = id_of(&entry);
        if id != binding_id && mentions(&entry.doc, binding_id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// 信源删除后，对其引用过的绑定做私有级联：剩余信源不再引用的绑定文件被移除；
/// 仍被其它信源引用的共享绑定保留。移除后整库校验，失败则回滚刚删的绑定。
fn cascade_orphan_bindings(
    dir: &Path,
    options: &LoadOptions,
    referenced: &[String],
) -> Result<(), Error> {
    let sources: Vec<Value> = entries(dir, "source")?
        .into_iter()
        .map(|entry| entry.doc)
        .collect();
    let mut removed: Vec<(std::path::PathBuf, String)> = Vec::new();
    for binding_id in referenced {
        if sources.iter().any(|doc| mentions(doc, binding_id)) {
            continue;
        }
        let Some(stem) = binding_id.split_once('/') else {
            continue;
        };
        let path = dir.join("bindings").join(format!("{}.yaml", stem.1));
        let Ok(original) = fs::read_to_string(&path) else {
            continue;
        };
        match fs::remove_file(&path) {
            Ok(()) => removed.push((path, original)),
            Err(_) => continue,
        }
    }
    if !removed.is_empty() {
        if let Err(error) = validate(dir, options) {
            for (path, original) in removed.iter().rev() {
                let _ = fs::write(path, original);
            }
            return Err(error);
        }
    }
    Ok(())
}

/// 启用或停用一份声明。
pub fn toggle(
    dir: &Path,
    options: &LoadOptions,
    kind: &str,
    slug: &str,
    enabled: bool,
) -> Result<Saved, Error> {
    let (folder, prefix) = folder_of(kind)?;
    let slug = check_slug(slug)?;
    let target = dir.join(folder).join(format!("{slug}.yaml"));
    let file = format!("{folder}/{slug}.yaml");
    let original = fs::read_to_string(&target).map_err(|_| invalid(&file, "声明不存在"))?;
    let doc = yaml::parse_value(&file, &original).map_err(|message| invalid(&file, message))?;
    let mut object = doc
        .as_object()
        .cloned()
        .ok_or_else(|| invalid(&file, "声明必须是对象"))?;
    object.insert("enabled".into(), Value::Bool(enabled));
    write(&target, &Value::Object(object))?;
    if let Err(error) = validate(dir, options) {
        restore(&target, &original)?;
        return Err(error);
    }
    Ok(Saved {
        id: format!("{prefix}/{slug}"),
        file,
    })
}

/// 设置系统级出站代理。
///
/// 值 `system` 表示跟随环境变量与系统设置，`off` 表示直连，其余按代理地址处理。
/// 与其它写操作走同一条校验路径：写盘后整目录校验，失败时恢复原文。
pub fn proxy(dir: &Path, options: &LoadOptions, value: &str) -> Result<Option<String>, Error> {
    let file = ["system.yaml", "system.yml"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
        .ok_or_else(|| invalid("system.yaml", "配置目录缺少 system.yaml"))?;
    let name = file
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("system.yaml")
        .to_string();
    let original = fs::read_to_string(&file).map_err(|source| Error::Read {
        path: file.display().to_string(),
        source,
    })?;
    let doc = yaml::parse_value(&name, &original).map_err(|message| invalid(&name, message))?;
    let mut root = doc
        .as_object()
        .cloned()
        .ok_or_else(|| invalid(&name, "声明必须是对象"))?;
    let mut policy = root
        .remove("policy")
        .and_then(|item| item.as_object().cloned())
        .unwrap_or_default();
    let mut egress = policy
        .remove("egress")
        .and_then(|item| item.as_object().cloned())
        .unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        egress.remove("proxy");
    } else {
        egress.insert("proxy".into(), json!(value));
    }
    if egress.is_empty() {
        policy.remove("egress");
    } else {
        policy.insert("egress".into(), Value::Object(egress));
    }
    if !policy.is_empty() {
        root.insert("policy".into(), Value::Object(policy));
    }
    write(&file, &Value::Object(root))?;
    if let Err(error) = validate(dir, options) {
        restore(&file, &original)?;
        return Err(error);
    }
    Ok((!value.is_empty()).then(|| value.to_string()))
}

/// 一份声明文件及其正文。
pub struct Entry {
    pub kind: String,
    pub slug: String,
    pub file: String,
    pub doc: Value,
}

/// 读出一类声明（`source`、`accessor` 或 `adapter`），按 slug 升序。
///
/// 只读磁盘，不合并、不预设：界面要改的是自己写下的那份，而不是叠加后的结果。
pub fn entries(dir: &Path, kind: &str) -> Result<Vec<Entry>, Error> {
    let (folder, _) = folder_of(kind)?;
    let root = dir.join(folder);
    let Ok(read) = fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in read.flatten() {
        let path = item.path();
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !matches!(extension, "yaml" | "yml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let file = format!("{folder}/{stem}.{extension}");
        let text = fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.display().to_string(),
            source,
        })?;
        let doc = yaml::parse_value(&file, &text).map_err(|message| invalid(&file, message))?;
        out.push(Entry {
            kind: kind.to_string(),
            slug: stem.to_string(),
            file,
            doc,
        });
    }
    out.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(out)
}

/// 管理列表：三类声明、启用状态与引用计数。
///
/// 引用计数让界面能说清「这一份还有 3 个信源在用」，而不是只给一个开关。
pub fn list(dir: &Path) -> Result<Value, Error> {
    let sources = entries(dir, "source")?;
    let accessors = entries(dir, "accessor")?;
    let adapters = entries(dir, "adapter")?;
    let bindings = entries(dir, "binding")?;
    let plans = entries(dir, "plan")?;
    let all: Vec<&Entry> = sources
        .iter()
        .chain(accessors.iter())
        .chain(adapters.iter())
        .chain(bindings.iter())
        .collect();
    let sources: Vec<Value> = sources
        .iter()
        .map(|entry| source_row(entry, &all))
        .collect();
    let accessors: Vec<Value> = accessors
        .iter()
        .map(|entry| accessor_row(entry, &all))
        .collect();
    let adapters: Vec<Value> = adapters
        .iter()
        .map(|entry| adapter_row(entry, &all))
        .collect();
    let bindings: Vec<Value> = bindings
        .iter()
        .map(|entry| binding_row(entry, &all))
        .collect();
    let plans: Vec<Value> = plans.iter().map(plan_row).collect();
    Ok(json!({
        "sources": sources,
        "accessors": accessors,
        "adapters": adapters,
        "bindings": bindings,
        "plans": plans,
    }))
}

/// 计划行：一等采集计划的声明视图（节律、额度、目标信源、优先级、启停）。
fn plan_row(entry: &Entry) -> Value {
    let doc = &entry.doc;
    json!({
        "id": id_of(entry),
        "slug": entry.slug,
        "file": entry.file,
        "name": name_of(doc, &entry.slug),
        "description": doc.get("description").cloned().unwrap_or(Value::Null),
        "enabled": enabled_of(doc),
        "targets": doc.get("targets").cloned().unwrap_or_else(|| json!([])),
        "trigger": doc.get("trigger").cloned().unwrap_or(Value::Null),
        "queue": doc.get("queue").cloned().unwrap_or(Value::Null),
        "priority": doc.get("priority").cloned().unwrap_or(Value::Null),
        "quota": doc.get("quota").cloned().unwrap_or(Value::Null),
        "stopPolicy": doc.get("stopPolicy").cloned().unwrap_or(Value::Null),
        "doc": doc,
    })
}

fn source_row(entry: &Entry, all: &[&Entry]) -> Value {
    let doc = &entry.doc;
    let pipeline: Vec<String> = doc
        .get("pipeline")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .flat_map(|step| {
                    step.get("run")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<String>>()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": id_of(entry),
        "slug": entry.slug,
        "file": entry.file,
        "name": name_of(doc, &entry.slug),
        "description": doc.get("description").cloned().unwrap_or(Value::Null),
        "enabled": enabled_of(doc),
        "accessor": doc.pointer("/access/accessor"),
        "locator": doc.pointer("/access/locator"),
        "endpoint": doc.pointer("/access/spec/endpoint"),
        "queue": doc.pointer("/access/queue"),
        "schedule": doc.pointer("/schedule/interval"),
        "adapters": pipeline,
        "proxy": doc.pointer("/policy/egress/proxy"),
        "references": referrers(all, entry),
        "doc": doc,
    })
}

fn accessor_row(entry: &Entry, all: &[&Entry]) -> Value {
    let doc = &entry.doc;
    json!({
        "id": id_of(entry),
        "slug": entry.slug,
        "file": entry.file,
        "name": name_of(doc, &entry.slug),
        "description": doc.get("description").cloned().unwrap_or(Value::Null),
        "enabled": enabled_of(doc),
        "implementation": doc.get("implementation"),
        "timeout": doc.pointer("/limits/timeout"),
        "concurrency": doc.pointer("/limits/concurrency"),
        "profile": doc.pointer("/egress/profile"),
        "proxy": doc.pointer("/policy/egress/proxy"),
        "references": referrers(all, entry),
        "doc": doc,
    })
}

fn adapter_row(entry: &Entry, all: &[&Entry]) -> Value {
    let doc = &entry.doc;
    json!({
        "id": id_of(entry),
        "slug": entry.slug,
        "file": entry.file,
        "name": name_of(doc, &entry.slug),
        "description": doc.get("description").cloned().unwrap_or(Value::Null),
        "enabled": enabled_of(doc),
        "implementation": doc.get("implementation"),
        "role": doc.get("role"),
        "produces": doc.get("produces"),
        "builtin": builtin_adapter(doc),
        "removable": !builtin_adapter(doc),
        "references": referrers(all, entry),
        "doc": doc,
    })
}

/// 绑定行：带被哪些信源引用（R5.1）与「是否可删」判定（R5.2 前置信息）。
fn binding_row(entry: &Entry, all: &[&Entry]) -> Value {
    let doc = &entry.doc;
    json!({
        "id": id_of(entry),
        "slug": entry.slug,
        "file": entry.file,
        "name": name_of(doc, &entry.slug),
        "description": doc.get("description").cloned().unwrap_or(Value::Null),
        "enabled": enabled_of(doc),
        "adapter": doc.get("adapter"),
        "params": doc.get("params").cloned().unwrap_or(Value::Null),
        "referencedBy": referrer_ids(all, entry),
        "references": referrers(all, entry),
        "removable": referrers(all, entry) == 0,
        "doc": doc,
    })
}

fn id_of(entry: &Entry) -> String {
    entry
        .doc
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{}", entry.kind, entry.slug))
}

fn name_of(doc: &Value, fallback: &str) -> String {
    doc.get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn enabled_of(doc: &Value) -> bool {
    doc.get("enabled").and_then(Value::as_bool).unwrap_or(true)
}

/// 除自己之外，有多少份声明提到了它的 id。
fn referrers(all: &[&Entry], entry: &Entry) -> usize {
    let needle = id_of(entry);
    all.iter()
        .filter(|other| !(other.kind == entry.kind && other.slug == entry.slug))
        .filter(|other| mentions(&other.doc, &needle))
        .count()
}

/// 除自己之外，哪些声明引用了它的 id（返回它们的 id，供删除守卫回显占用方）。
fn referrer_ids(all: &[&Entry], entry: &Entry) -> Vec<String> {
    let needle = id_of(entry);
    all.iter()
        .filter(|other| !(other.kind == entry.kind && other.slug == entry.slug))
        .filter(|other| mentions(&other.doc, &needle))
        .map(|other| id_of(other))
        .collect()
}

fn mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text == needle,
        Value::Array(items) => items.iter().any(|item| mentions(item, needle)),
        Value::Object(map) => map.values().any(|item| mentions(item, needle)),
        _ => false,
    }
}

fn template(kind: &str, slug: &str, name: &str, description: &str, doc: Value) -> Value {
    json!({
        "kind": kind,
        "slug": slug,
        "name": name,
        "description": description,
        "doc": doc,
    })
}

fn limits(timeout: u64, concurrency: u64, attempts: u64) -> Value {
    json!({
        "timeout": format!("{timeout}s"),
        "concurrency": concurrency,
        "retry": {
            "attempts": attempts,
            "backoff": "exponential",
            "base": "1s",
            "max": "30s"
        }
    })
}

fn folder_of(kind: &str) -> Result<(&'static str, &'static str), Error> {
    match kind {
        "source" => Ok(("sources", "source")),
        "accessor" => Ok(("accessors", "accessor")),
        "adapter" => Ok(("adapters", "adapter")),
        "binding" => Ok(("bindings", "binding")),
        "plan" => Ok(("plans", "plan")),
        _ => Err(invalid("", "只支持信源、访问器、适配器、绑定与计划")),
    }
}

fn text(value: &Value, key: &str) -> Result<String, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid("", format!("缺少 {key}")))
}

/// id 的约束与加载器保持一致：字母数字开头，其余只允许 `._-`。
///
/// 工作流的声明名也走同一条规则，避免两套「合法名字」在不同目录里不一致。
pub fn check_slug(slug: &str) -> Result<&str, Error> {
    let slug = slug.trim();
    let mut chars = slug.chars();
    let Some(first) = chars.next() else {
        return Err(invalid("", "缺少声明名"));
    };
    if slug.len() > 64
        || !first.is_ascii_alphanumeric()
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(invalid(
            "",
            "声明名只能包含字母、数字、点、下划线与连字符，且以字母或数字开头",
        ));
    }
    Ok(slug)
}

fn builtin_adapter(doc: &Value) -> bool {
    let Some(implementation) = doc.get("implementation").and_then(Value::as_str) else {
        return false;
    };
    let Some(name) = implementation.strip_prefix("builtin:") else {
        return false;
    };
    let expected = format!("adapter/{name}");
    doc.get("id").and_then(Value::as_str) == Some(expected.as_str())
}

fn write(target: &Path, doc: &Value) -> Result<(), Error> {
    fs::write(target, to_yaml(doc)).map_err(|source| Error::Read {
        path: target.display().to_string(),
        source,
    })
}

fn restore(target: &Path, original: &str) -> Result<(), Error> {
    fs::write(target, original).map_err(|source| Error::Read {
        path: target.display().to_string(),
        source,
    })
}

fn invalid(file: &str, message: impl Into<String>) -> Error {
    Error::Invalid(Diagnostics(vec![Diagnostic {
        file: file.to_string(),
        line: 0,
        message: message.into(),
    }]))
}

/// 用 saphyr 序列化器生成块状 YAML——与加载用的解析器同库，保证「写出即可读回」。
///
/// 不再手写标量加引号规则：由成熟发射器统一处理结尾冒号、`: `、前后空白、`#`/`-` 起始、
/// flow 指示符、`yes/null/数字` 歧义等边界，避免逐条打补丁仍漏（历史上就漏过 `sectionTitle:`）。
pub fn to_yaml(value: &Value) -> String {
    let node = to_saphyr(value);
    let mut buffer = String::new();
    let emitted = {
        let mut emitter = YamlEmitter::new(&mut buffer);
        emitter.dump(&node)
    };
    // 纯 JSON 树（无二进制/别名/标签）不会发射失败；万一失败兜底回 JSON——
    // YAML 是 JSON 超集，仍能被同一解析器读回，不丢数据。
    let text = match emitted {
        Ok(()) => buffer,
        Err(_) => serde_json::to_string(value).unwrap_or_default(),
    };
    let body = text.strip_prefix("---\n").unwrap_or(&text);
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

/// 把 `serde_json::Value` 映射成 saphyr 节点（键按 BTreeMap 字典序，与既有写出形态一致）。
fn to_saphyr(value: &Value) -> Yaml<'static> {
    match value {
        Value::Null => Yaml::Value(Scalar::Null),
        Value::Bool(flag) => Yaml::Value(Scalar::Boolean(*flag)),
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                Yaml::Value(Scalar::Integer(int))
            } else if let Some(uint) = number.as_u64() {
                Yaml::Value(Scalar::Integer(uint as i64))
            } else if let Some(float) = number.as_f64() {
                Yaml::Value(Scalar::FloatingPoint(OrderedFloat(float)))
            } else {
                Yaml::Value(Scalar::String(Cow::Owned(number.to_string())))
            }
        }
        Value::String(text) => Yaml::Value(Scalar::String(Cow::Owned(text.clone()))),
        Value::Array(items) => Yaml::Sequence(items.iter().map(to_saphyr).collect()),
        Value::Object(map) => {
            let mut mapping = Mapping::new();
            for (key, value) in map {
                mapping.insert(
                    Yaml::Value(Scalar::String(Cow::Owned(key.clone()))),
                    to_saphyr(value),
                );
            }
            Yaml::Mapping(mapping)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_roundtrip_keeps_strings() {
        let doc = json!({
            "id": "source/demo",
            "enabled": false,
            "name": "示例 源",
            "schedule": {"interval": "2h"},
            "pipeline": [{"run": ["adapter/feed-items"]}],
            "admission": {}
        });
        let text = to_yaml(&doc);
        let parsed = yaml::parse_value("demo.yaml", &text).expect("可解析");
        assert_eq!(parsed, doc);
    }

    /// 结尾冒号（如 `sectionTitle:`）等歧义标量必须能写出后原样读回。
    ///
    /// 断言的是「值往返」而非具体引号形态——发射器用单引号/双引号皆可，只要可读回。
    #[test]
    fn yaml_roundtrip_preserves_trailing_colon_and_urls() {
        let doc = json!({
            "spec": {"extract": [{"endMarker": "sectionTitle:", "type": "quoted-fields"}]},
            "locator": {"url": "https://mimo.xiaomi.com/"}
        });
        let text = to_yaml(&doc);
        let parsed = yaml::parse_value("mimo.yaml", &text).expect("迁移产物必须可重新解析");
        assert_eq!(parsed, doc, "往返必须保持原值:\n{text}");
    }

    /// 覆盖 YAML 标量的全部典型歧义：类型陷阱、前后空白、控制符、注释/序列起始、数字与浮点。
    #[test]
    fn yaml_roundtrip_ambiguous_scalars() {
        let doc = json!({
            "nullWord": "null",
            "yesWord": "yes",
            "octalLike": "0755",
            "empty": "",
            "padded": "  padded  ",
            "multiline": "line1\nline2",
            "hashStart": "#hash",
            "dashStart": "- dash",
            "colonInside": "x: y",
            "floatWord": "1.5",
            "boolTrue": true,
            "intVal": 42,
            "floatVal": 3.5,
            "nested": [{"a": "b:c"}, []],
            "emptyMap": {}
        });
        let text = to_yaml(&doc);
        let parsed = yaml::parse_value("amb.yaml", &text).expect("可解析");
        assert_eq!(parsed, doc, "歧义标量必须原样往返:\n{text}");
    }

    #[test]
    fn proxy_lands_under_system_policy() {
        let dir = tempfile::tempdir().expect("临时目录");
        let preset = crate::builtin_presets()
            .into_iter()
            .find(|item| item.id == "preset/standard")
            .expect("内置预设");
        std::fs::create_dir_all(dir.path().join("presets")).expect("presets 目录");
        std::fs::write(dir.path().join("presets/standard.yaml"), preset.source).expect("写入预设");
        std::fs::write(
            dir.path().join("system.yaml"),
            "id: system\nkind: system\npolicy:\n  preset: standard\n  raw:\n    store: true\n",
        )
        .expect("写入 system.yaml");
        let options = LoadOptions::default();
        let value = proxy(dir.path(), &options, "http://127.0.0.1:7890").expect("写入代理");
        assert_eq!(value.as_deref(), Some("http://127.0.0.1:7890"));
        let text = std::fs::read_to_string(dir.path().join("system.yaml")).unwrap();
        let doc = yaml::parse_value("system.yaml", &text).expect("可解析");
        assert_eq!(
            doc.pointer("/policy/egress/proxy"),
            Some(&json!("http://127.0.0.1:7890"))
        );
        assert_eq!(doc.pointer("/policy/raw/store"), Some(&json!(true)));
        assert_eq!(proxy(dir.path(), &options, "").expect("清除代理"), None);
        let text = std::fs::read_to_string(dir.path().join("system.yaml")).unwrap();
        assert!(!text.contains("proxy"));
    }

    #[test]
    fn builtin_adapter_is_only_the_shipped_one() {
        let shipped = json!({"id": "adapter/html-list", "implementation": "builtin:html-list"});
        let custom = json!({"id": "adapter/my-list", "implementation": "builtin:html-list"});
        assert!(builtin_adapter(&shipped));
        assert!(!builtin_adapter(&custom));
    }

    /// 搭一份能过 `validate` 的最小配置：系统 + 预设 + 访问器 + 适配器。
    fn seed_config(dir: &Path) {
        let preset = crate::builtin_presets()
            .into_iter()
            .find(|item| item.id == "preset/standard")
            .expect("内置预设");
        std::fs::create_dir_all(dir.join("presets")).unwrap();
        std::fs::write(dir.join("presets/standard.yaml"), preset.source).unwrap();
        std::fs::write(
            dir.join("system.yaml"),
            "id: system\nkind: system\nname: s\npolicy:\n  preset: standard\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("accessors")).unwrap();
        std::fs::write(
            dir.join("accessors/rss.yaml"),
            "id: accessor/rss\nkind: accessor\nname: RSS\nimplementation: builtin:rss\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("adapters")).unwrap();
        std::fs::write(
            dir.join("adapters/json-list.yaml"),
            "id: adapter/json-list\nkind: adapter\nname: JSON\nimplementation: builtin:json-list\nrole: search\nconsumes:\n  - {kind: raw, part_role: document, media_type: application/json}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: exponential\n    base: 1s\n    max: 30s\n",
        )
        .unwrap();
    }

    fn create_binding(dir: &Path, slug: &str) -> Saved {
        create(
            dir,
            &LoadOptions::default(),
            &Draft {
                kind: "binding".into(),
                slug: slug.into(),
                doc: json!({
                    "name": slug,
                    "adapter": "adapter/json-list",
                    "params": {"items": "/data"},
                }),
            },
        )
        .expect("创建绑定")
    }

    fn create_bound_source(dir: &Path, slug: &str, binding: &str) -> Saved {
        create(
            dir,
            &LoadOptions::default(),
            &Draft {
                kind: "source".into(),
                slug: slug.into(),
                doc: json!({
                    "name": slug,
                    "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
                    "pipeline": [{"binding": binding}],
                    "schedule": {"interval": "1h"},
                    "admission": {},
                }),
            },
        )
        .expect("创建绑定信源")
    }

    /// R12/S2：写盘入口统一补齐 `sourceType`，HTTP 与 MCP 不再各写一遍。
    #[test]
    fn author_stamps_effective_source_type() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        create_binding(dir.path(), "b1");
        create_bound_source(dir.path(), "s1", "binding/b1");
        let file = dir.path().join("sources/s1.yaml");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("sourceType: feed"), "{text}");

        // 已声明则保留原值，不在写盘层改写调用者的选择。
        let explicit = create(
            dir.path(),
            &LoadOptions::default(),
            &Draft {
                kind: "source".into(),
                slug: "s2".into(),
                doc: json!({
                    "name": "s2",
                    "sourceType": "web",
                    "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
                    "pipeline": [{"binding": "binding/b1"}],
                    "schedule": {"interval": "1h"},
                    "admission": {},
                }),
            },
        );
        // web 与 rss 访问器不一致，由整库校验拒绝并恢复原文。
        assert!(explicit.is_err());
        assert!(!dir.path().join("sources/s2.yaml").exists());

        // 缺省声明的更新同样补齐。
        let saved = update(
            dir.path(),
            &LoadOptions::default(),
            &Draft {
                kind: "source".into(),
                slug: "s1".into(),
                doc: json!({
                    "name": "s1",
                    "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed2"}},
                    "pipeline": [{"binding": "binding/b1"}],
                    "schedule": {"interval": "2h"},
                    "admission": {},
                }),
            },
        )
        .expect("更新绑定信源");
        assert_eq!(saved.id, "source/s1");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("sourceType: feed"), "{text}");
    }

    /// R5.2：仍被信源引用的绑定拒删。
    #[test]
    fn referenced_binding_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        create_binding(dir.path(), "b1");
        create_bound_source(dir.path(), "s1", "binding/b1");
        let options = LoadOptions::default();
        let error = match remove(dir.path(), &options, "binding", "b1") {
            Ok(_) => panic!("被引用的绑定应拒删"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("binding/s1") || error.to_string().contains("source/s1"),
            "错误应回显占用方: {error}"
        );
        // 绑定文件仍在。
        assert!(dir.path().join("bindings/b1.yaml").exists());
    }

    /// R5.3：删除信源时，独占绑定级联清理，共享绑定保留。
    #[test]
    fn source_delete_cascades_private_binding_and_retains_shared() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let shared = create_binding(dir.path(), "shared");
        let private = create_binding(dir.path(), "private");
        // s1 独占 private，与 s2 共享 shared。
        create(
            dir.path(),
            &LoadOptions::default(),
            &Draft {
                kind: "source".into(),
                slug: "s1".into(),
                doc: json!({
                    "name": "s1",
                    "access": {"accessor": "accessor/rss", "locator": {"url": "https://example.com/feed"}},
                    "pipeline": [{"binding": "binding/private"}, {"binding": "binding/shared"}],
                    "schedule": {"interval": "1h"},
                    "admission": {},
                }),
            },
        )
        .unwrap();
        create_bound_source(dir.path(), "s2", "binding/shared");
        let options = LoadOptions::default();
        remove(dir.path(), &options, "source", "s1").expect("删除 s1");
        // 独占的 private 被级联删除；共享的 shared 保留。
        assert!(
            !dir.path().join("bindings/private.yaml").exists(),
            "私有绑定应级联删除"
        );
        assert!(
            dir.path().join("bindings/shared.yaml").exists(),
            "共享绑定应保留"
        );
        // 二次删除 s2 后，shared 变孤儿被清理。
        remove(dir.path(), &options, "source", "s2").expect("删除 s2");
        assert!(!dir.path().join("bindings/shared.yaml").exists());
        let _ = (shared.id, private.id);
    }
}
