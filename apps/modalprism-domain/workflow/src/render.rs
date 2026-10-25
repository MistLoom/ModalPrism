//! 提示词与正文模板。
//!
//! 只支持 `{{a.b}}` 形式的字段引用：没有表达式、没有函数，声明能读到的东西
//! 与接口返回的字段一一对应。变量缺失直接报错，避免把空字符串写进产物。

use serde_json::Value;

use crate::error::{Error, Result};

/// 用变量表渲染模板。`vars` 的形状是 `{article: {...}, steps: {id: "..."}}`。
pub fn render(template: &str, vars: &Value) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let Some(end) = tail.find("}}") else {
            return Err(Error::invalid("模板缺少 }}"));
        };
        let key = tail[..end].trim();
        let value =
            lookup(vars, key).ok_or_else(|| Error::invalid(format!("模板变量不存在: {key}")))?;
        out.push_str(&text(value));
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn lookup<'a>(vars: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = vars;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::render;
    use serde_json::json;

    #[test]
    fn renders_nested_fields() {
        let vars = json!({"article": {"title": "标题"}, "steps": {"摘要": "总结"}});
        let text = render("标题：{{article.title}}\n摘要：{{steps.摘要}}", &vars).unwrap();
        assert_eq!(text, "标题：标题\n摘要：总结");
    }

    #[test]
    fn missing_variable_is_an_error() {
        let vars = json!({"article": {"title": "标题"}});
        assert!(render("{{article.url}}", &vars).is_err());
    }
}
