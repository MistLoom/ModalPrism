//! 声明文件的可视化编辑。
//!
//! 信源、访问器与适配器都是 YAML 声明。这个模块让界面不再依赖编译：按模板生成草稿、
//! 写入文件、立即用同一套加载器校验，失败时把磁盘恢复到写入前的状态。
//!
//! 内置适配器的判定不写死清单：`implementation` 为 `builtin:<name>` 且 id 恰好是
//! `adapter/<name>` 的声明就是系统自带的那一份，可以修改、可以停用，但不能删除。

use std::fs;
use std::path::Path;

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

/// 删除一份声明。内置适配器拒绝删除；被其它声明引用时由校验兜底并恢复原文。
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
    fs::remove_file(&target).map_err(|_| invalid(&file, "删除失败"))?;
    if let Err(error) = validate(dir, options) {
        restore(&target, &original)?;
        return Err(error);
    }
    Ok(Saved {
        id: format!("{prefix}/{slug}"),
        file,
    })
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
    let all: Vec<&Entry> = sources
        .iter()
        .chain(accessors.iter())
        .chain(adapters.iter())
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
    Ok(json!({
        "sources": sources,
        "accessors": accessors,
        "adapters": adapters,
    }))
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
        _ => Err(invalid("", "只支持信源、访问器与适配器")),
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

/// 生成块状 YAML。字符串一律加引号，避免把 `yes`、数字或含冒号的值读成别的类型。
pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    if let Some(map) = value.as_object() {
        write_map(&mut out, map, 0);
    }
    out
}

fn write_map(out: &mut String, map: &Map<String, Value>, indent: usize) {
    for (key, value) in map {
        let pad = " ".repeat(indent);
        match value {
            Value::Object(inner) if !inner.is_empty() => {
                out.push_str(&format!("{pad}{}:\n", key_text(key)));
                write_map(out, inner, indent + 2);
            }
            Value::Array(items) if !items.is_empty() => {
                out.push_str(&format!("{pad}{}:\n", key_text(key)));
                write_seq(out, items, indent + 2);
            }
            _ => out.push_str(&format!("{pad}{}: {}\n", key_text(key), scalar(value))),
        }
    }
}

fn write_seq(out: &mut String, items: &[Value], indent: usize) {
    let pad = " ".repeat(indent);
    for item in items {
        match item {
            Value::Object(map) if !map.is_empty() => {
                let mut nested = String::new();
                write_map(&mut nested, map, indent + 2);
                out.push_str(&format!("{pad}- {}", nested.trim_start_matches(' ')));
            }
            Value::Array(inner) if !inner.is_empty() => {
                let mut nested = String::new();
                write_seq(&mut nested, inner, indent + 2);
                out.push_str(&format!("{pad}- {}", nested.trim_start_matches(' ')));
            }
            _ => out.push_str(&format!("{pad}- {}\n", scalar(item))),
        }
    }
}

fn key_text(key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'));
    if plain { key.to_string() } else { quoted(key) }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(text) => {
            if plain(text) {
                text.clone()
            } else {
                quoted(text)
            }
        }
        Value::Array(_) => "[]".into(),
        Value::Object(_) => "{}".into(),
    }
}

fn plain(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '/')
        && text.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '.' | '/' | ':' | '-' | '+' | '@' | '=')
        })
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
        )
        && text.parse::<f64>().is_err()
}

fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
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
}
