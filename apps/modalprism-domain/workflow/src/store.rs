//! 工作流与模型的声明文件。
//!
//! 两份目录、一个规则：`workflows/<slug>.yaml` 与 `models/<slug>.yaml`。写盘前先解析，
//! 解析过不去就不写；写入后引擎下一次 tick 自然读到新声明，不需要重启。
//!
//! 这里只碰文件，不碰存储。运行记录由 [`crate::Engine`] 写进 Strata。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::spec::{ModelSpec, WorkflowSpec};

const WORKFLOWS: &str = "workflows";
const MODELS: &str = "models";

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 工作流列表。单份声明坏掉时给出 `error`，不连累其它声明。
    pub fn workflows(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for (slug, path) in self.files(WORKFLOWS)? {
            let file = format!("{WORKFLOWS}/{slug}.yaml");
            let row = match read(&path).and_then(|doc| WorkflowSpec::parse(&file, &doc)) {
                Ok(spec) => {
                    let mut summary = spec.summary(&slug, &file);
                    summary["document"] = spec.document();
                    summary["available"] = json!(self.model_file(&spec.model).exists());
                    summary
                }
                Err(error) => json!({
                    "slug": slug,
                    "file": file,
                    "name": slug,
                    "enabled": false,
                    "available": false,
                    "steps": [],
                    "notify": [],
                    "trigger": {"on": "commit", "sources": [], "kinds": ["article"]},
                    "error": error.to_string(),
                }),
            };
            out.push(row);
        }
        Ok(out)
    }

    /// 可运行的工作流。坏掉的声明跳过并记日志，避免一处笔误拖住整条流水线。
    pub fn runnable(&self) -> Result<Vec<(String, WorkflowSpec)>> {
        let mut out = Vec::new();
        for (slug, path) in self.files(WORKFLOWS)? {
            let file = format!("{WORKFLOWS}/{slug}.yaml");
            match read(&path).and_then(|doc| WorkflowSpec::parse(&file, &doc)) {
                Ok(spec) if spec.enabled => out.push((slug, spec)),
                Ok(_) => {}
                Err(error) => tracing::warn!(file = %file, error = %error, "工作流声明无效"),
            }
        }
        Ok(out)
    }

    pub fn workflow(&self, slug: &str) -> Result<Value> {
        let slug = check(slug)?;
        let file = format!("{WORKFLOWS}/{slug}.yaml");
        let doc = read(&self.dir.join(&file))?;
        let spec = WorkflowSpec::parse(&file, &doc)?;
        Ok(json!({
            "slug": slug,
            "file": file,
            "document": spec.document(),
            "available": self.model_file(&spec.model).exists(),
        }))
    }

    /// 取一条工作流声明。引擎与手动运行都从这里读，保证与校验用的是同一份结构。
    pub fn spec(&self, slug: &str) -> Result<WorkflowSpec> {
        let slug = check(slug)?;
        let file = format!("{WORKFLOWS}/{slug}.yaml");
        let doc = read(&self.dir.join(&file))
            .map_err(|_| Error::invalid(format!("{file}: 工作流不存在")))?;
        WorkflowSpec::parse(&file, &doc)
    }

    /// 新建或改写工作流。`{slug, doc}`。
    pub fn save_workflow(&self, input: &Value) -> Result<Value> {
        let slug = text(input, "slug")?;
        let slug = check(&slug)?;
        let file = format!("{WORKFLOWS}/{slug}.yaml");
        let doc = stamp(input, "workflow")?;
        let spec = WorkflowSpec::parse(&file, &doc)?;
        if !self.model_file(&spec.model).exists() {
            return Err(Error::invalid(format!(
                "{file}: 模型 {} 不存在，请先在模型页创建",
                spec.model
            )));
        }
        write(&self.dir.join(&file), &doc)?;
        Ok(json!({"slug": slug, "file": file}))
    }

    pub fn remove_workflow(&self, slug: &str) -> Result<Value> {
        let slug = check(slug)?;
        let file = format!("{WORKFLOWS}/{slug}.yaml");
        fs::remove_file(self.dir.join(&file))
            .map_err(|_| Error::invalid(format!("{file}: 删除失败")))?;
        Ok(json!({"slug": slug, "file": file}))
    }

    pub fn toggle_workflow(&self, slug: &str, enabled: bool) -> Result<Value> {
        let slug = check(slug)?;
        let file = format!("{WORKFLOWS}/{slug}.yaml");
        let path = self.dir.join(&file);
        let mut object = read(&path)?
            .as_object()
            .cloned()
            .ok_or_else(|| Error::invalid(format!("{file}: 声明必须是对象")))?;
        object.insert("enabled".into(), Value::Bool(enabled));
        let doc = Value::Object(object);
        WorkflowSpec::parse(&file, &doc)?;
        write(&path, &doc)?;
        Ok(json!({"slug": slug, "enabled": enabled}))
    }

    pub fn models(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for (slug, path) in self.files(MODELS)? {
            let file = format!("{MODELS}/{slug}.yaml");
            let row = match read(&path).and_then(|doc| ModelSpec::parse(&file, &doc)) {
                Ok(spec) => {
                    let mut summary = spec.summary(&slug);
                    summary["document"] = spec.document();
                    summary
                }
                Err(error) => json!({
                    "slug": slug,
                    "name": slug,
                    "mode": "api",
                    "error": error.to_string(),
                }),
            };
            out.push(row);
        }
        Ok(out)
    }

    /// 取一个模型声明。工作流只写模型名，密钥留在环境变量里。
    pub fn model(&self, slug: &str) -> Result<ModelSpec> {
        let slug = check(slug)?;
        let file = format!("{MODELS}/{slug}.yaml");
        let doc = read(&self.dir.join(&file))
            .map_err(|_| Error::invalid(format!("{file}: 模型不存在")))?;
        ModelSpec::parse(&file, &doc)
    }

    pub fn save_model(&self, input: &Value) -> Result<Value> {
        let slug = text(input, "slug")?;
        let slug = check(&slug)?;
        let file = format!("{MODELS}/{slug}.yaml");
        let doc = stamp(input, "model")?;
        ModelSpec::parse(&file, &doc)?;
        write(&self.dir.join(&file), &doc)?;
        Ok(json!({"slug": slug, "file": file}))
    }

    pub fn remove_model(&self, slug: &str) -> Result<Value> {
        let slug = check(slug)?;
        let file = format!("{MODELS}/{slug}.yaml");
        let using = self
            .workflows()?
            .into_iter()
            .filter(|row| row.get("model").and_then(Value::as_str) == Some(slug.as_str()))
            .filter_map(|row| row.get("slug").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>();
        if !using.is_empty() {
            return Err(Error::invalid(format!(
                "{file}: 仍被工作流引用（{}）",
                using.join("、")
            )));
        }
        fs::remove_file(self.dir.join(&file))
            .map_err(|_| Error::invalid(format!("{file}: 删除失败")))?;
        Ok(json!({"slug": slug, "file": file}))
    }

    /// 界面上的新建模板：三种模型形态各一份，外加一份开箱可用的工作流。
    pub fn templates() -> Value {
        json!({
            "workflows": [{
                "slug": "digest",
                "name": "要点摘要",
                "description": "把新文章压成三个要点，写到 struct:summary，失败时发飞书",
                "doc": {
                    "name": "要点摘要",
                    "description": "新文章入库后自动概括",
                    "trigger": {"on": "commit", "kinds": ["article"]},
                    "model": "default",
                    "steps": [{
                        "id": "summary",
                        "kind": "llm",
                        "prompt": "标题：{{article.title}}\n正文：{{article.text}}\n用三行要点概括，每行以「- 」开头。"
                    }],
                    "outputs": {"facet": {"kind": "md", "variant": "summary", "body": "{{steps.summary}}"}}
                }
            }],
            "models": [
                {
                    "slug": "default",
                    "name": "远端 API",
                    "description": "OpenAI 兼容接口，密钥从环境变量读",
                    "doc": {
                        "name": "默认模型",
                        "mode": "api",
                        "endpoint": "https://api.openai.com/v1",
                        "model": "gpt-4o-mini",
                        "keyEnv": "OPENAI_API_KEY",
                        "timeout": "90s",
                        "temperature": 0.3
                    }
                },
                {
                    "slug": "local",
                    "name": "本地模型",
                    "description": "Ollama、vLLM、LM Studio 等本机端点，密钥可省略",
                    "doc": {
                        "name": "本地模型",
                        "mode": "local",
                        "endpoint": "http://127.0.0.1:11434/v1",
                        "model": "qwen2.5:7b",
                        "timeout": "120s"
                    }
                },
                {
                    "slug": "cli",
                    "name": "命令行模型",
                    "description": "把提示词写进子进程标准输入，取标准输出",
                    "doc": {
                        "name": "命令行模型",
                        "mode": "cli",
                        "command": ["claude", "-p"],
                        "timeout": "180s"
                    }
                }
            ]
        })
    }

    fn model_file(&self, slug: &str) -> PathBuf {
        self.dir.join(MODELS).join(format!("{slug}.yaml"))
    }

    fn files(&self, folder: &str) -> Result<Vec<(String, PathBuf)>> {
        let root = self.dir.join(folder);
        let Ok(read) = fs::read_dir(&root) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for item in read.flatten() {
            let path = item.path();
            let extension = path.extension().and_then(|value| value.to_str());
            if !matches!(extension, Some("yaml" | "yml")) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                out.push((stem.to_string(), path));
            }
        }
        out.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(out)
    }
}

fn read(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)
        .map_err(|_| Error::invalid(format!("{}: 声明不存在", path.display())))?;
    config::parse_yaml(&path.display().to_string(), &text).map_err(Error::invalid)
}

fn write(path: &Path, doc: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| Error::local(format!("{}: {error}", parent.display())))?;
    }
    fs::write(path, config::to_yaml(doc))
        .map_err(|error| Error::local(format!("{}: {error}", path.display())))
}

fn text(input: &Value, key: &str) -> Result<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::invalid(format!("缺少 {key}")))
}

/// 把界面传来的正文补上 `id` 与 `kind`，让磁盘上的声明与手写风格一致。
fn stamp(input: &Value, prefix: &str) -> Result<Value> {
    let slug = text(input, "slug")?;
    let doc = input
        .get("doc")
        .cloned()
        .ok_or_else(|| Error::invalid("缺少 doc"))?;
    let mut object = doc
        .as_object()
        .cloned()
        .ok_or_else(|| Error::invalid("声明必须是对象"))?;
    object.insert("id".into(), json!(format!("{prefix}/{slug}")));
    object.insert("kind".into(), json!(prefix));
    object
        .entry("name")
        .or_insert_with(|| Value::String(slug.clone()));
    Ok(Value::Object(object))
}

fn check(slug: &str) -> Result<String> {
    config::check_slug(slug)
        .map(str::to_string)
        .map_err(|error| Error::invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("临时目录");
        let store = Store::new(dir.path());
        (dir, store)
    }

    #[test]
    fn templates_parse_into_specs() {
        let values = Store::templates();
        for item in values["models"].as_array().unwrap() {
            let doc = stamp(item, "model").expect("模型声明");
            ModelSpec::parse("models/x.yaml", &doc).expect("模型模板可用");
        }
        for item in values["workflows"].as_array().unwrap() {
            let doc = stamp(item, "workflow").expect("工作流声明");
            WorkflowSpec::parse("workflows/x.yaml", &doc).expect("工作流模板可用");
        }
    }

    #[test]
    fn saving_requires_an_existing_model() {
        let (_dir, store) = store();
        let doc = json!({
            "name": "摘要",
            "model": "missing",
            "steps": [{"id": "a", "kind": "llm", "prompt": "hi"}]
        });
        let error = store
            .save_workflow(&json!({"slug": "digest", "doc": doc}))
            .expect_err("模型不存在时拒绝");
        assert!(error.to_string().contains("模型"));
    }

    #[test]
    fn model_is_saved_and_listed() {
        let (_dir, store) = store();
        let template = Store::templates();
        let api = &template["models"][0];
        store
            .save_model(&json!({"slug": "default", "doc": api["doc"]}))
            .expect("写入模型");
        let listed = store.models().expect("列出模型");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["slug"], json!("default"));
        assert_eq!(
            store.model("default").expect("读取模型").mode.label(),
            "api"
        );
    }
}
