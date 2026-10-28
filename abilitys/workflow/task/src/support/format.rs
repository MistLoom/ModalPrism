//! 响应格式渲染。

use serde_json::{Value, json};

use super::model::{OutputFormat, ProjectMeta, Task, TaskGraph, TaskStatus};

pub fn wrap(format: OutputFormat, payload: Value) -> Value {
    match format {
        OutputFormat::Json => {
            let mut obj = match payload {
                Value::Object(m) => m,
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("data".into(), other);
                    m
                }
            };
            obj.insert("format".into(), json!("json"));
            Value::Object(obj)
        }
        OutputFormat::Md => {
            json!({
                "format": "md",
                "content": value_to_md(&payload),
            })
        }
    }
}

pub fn task_to_value(t: &Task) -> Value {
    json!({
        "id": t.id,
        "name": t.name,
        "description": t.description,
        "status": t.status.as_str(),
        "claimed_by": t.claimed_by,
        "depends_on": t.depends_on,
        "system_version": t.system_version,
    })
}

pub fn tasks_to_value(tasks: &[&Task]) -> Value {
    Value::Array(tasks.iter().map(|t| task_to_value(t)).collect())
}

pub fn project_to_value(p: &ProjectMeta) -> Value {
    json!({
        "id": p.id,
        "data_dir": p.data_dir,
        "default_format": p.default_format.as_str(),
        "system_version": p.system_version,
    })
}

pub fn graph_summary(graph: &TaskGraph) -> Value {
    let mut counts = serde_json::Map::new();
    for s in [
        TaskStatus::Unclaimed,
        TaskStatus::Claimed,
        TaskStatus::Running,
        TaskStatus::Verifying,
        TaskStatus::Completed,
        TaskStatus::Failed,
    ] {
        let n = graph.tasks.values().filter(|t| t.status == s).count();
        counts.insert(s.as_str().into(), json!(n));
    }
    json!({
        "task_count": graph.tasks.len(),
        "ready_count": graph.ready_ids().len(),
        "status_counts": counts,
    })
}

fn value_to_md(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            if let Some(tasks) = m.get("tasks").and_then(|x| x.as_array()) {
                let mut lines = vec!["# Tasks".into(), String::new()];
                for t in tasks {
                    lines.push(task_value_md(t));
                    lines.push(String::new());
                }
                if let Some(empty) = m.get("empty") {
                    lines.push(format!("empty: {empty}"));
                }
                return lines.join("\n");
            }
            if m.contains_key("id") && m.contains_key("status") {
                return task_value_md(v);
            }
            if m.contains_key("task_count") {
                return format!(
                    "# Project status\n\n- task_count: {}\n- ready_count: {}\n",
                    m.get("task_count").unwrap_or(&Value::Null),
                    m.get("ready_count").unwrap_or(&Value::Null),
                );
            }
            // 通用键值
            let mut lines = Vec::new();
            for (k, val) in m {
                if k == "format" {
                    continue;
                }
                lines.push(format!("- **{k}**: {val}"));
            }
            lines.join("\n")
        }
        Value::Array(arr) => {
            let mut lines = vec!["# Tasks".into(), String::new()];
            for t in arr {
                lines.push(task_value_md(t));
                lines.push(String::new());
            }
            lines.join("\n")
        }
        other => other.to_string(),
    }
}

fn task_value_md(t: &Value) -> String {
    format!(
        "## {}\n- name: {}\n- status: {}\n- claimed_by: {}\n- depends_on: {}\n- description: {}",
        t.get("id").and_then(|x| x.as_str()).unwrap_or("?"),
        t.get("name").and_then(|x| x.as_str()).unwrap_or(""),
        t.get("status").and_then(|x| x.as_str()).unwrap_or(""),
        t.get("claimed_by")
            .map(|x| x.to_string())
            .unwrap_or_else(|| "null".into()),
        t.get("depends_on")
            .map(|x| x.to_string())
            .unwrap_or_else(|| "[]".into()),
        t.get("description").and_then(|x| x.as_str()).unwrap_or(""),
    )
}
