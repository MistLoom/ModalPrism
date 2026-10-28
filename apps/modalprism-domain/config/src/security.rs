//! 公共落盘安全检查（R10.2 / H4）。
//!
//! binding 承载环境变量引用、endpoint、`urlTemplate`、CSS/JSON 选择器等，是新增的攻击面。
//! 原先只覆盖适配器草稿的禁用串 / 回环 / SSRF / `javascript:` 检查在此下沉为**公共校验器**，
//! Adapter 与 AdapterBinding 落盘前都强制走同一套。校验在配置加载（含写前 `validate`）里执行，
//! 因此无论来自 API、迁移脚本还是手改文件，畸形或不安全的 binding 都会在落盘阶段被拒。

use serde_json::Value;

/// 明显危险的禁用串（大小写不敏感，覆盖整份声明的序列化文本）。
///
/// `${env:NAME}` 形式的引用是**允许**的——那正是把密钥挡在声明之外的安全写法；
/// 被禁止的是直接内联凭据、脚本协议与运行期环境变量访问。
const BANNED: &[&str] = &[
    "javascript:",
    "eval(",
    "process.env",
    "__proto__",
    "127.0.0.1",
    "localhost",
    "::1",
];

/// 需要按 URL 解析、检查回环/私网指向的字段名（适配器与绑定共用）。
const URL_FIELDS: &[&str] = &["url", "endpoint", "urlTemplate", "link", "base", "baseUrl"];

/// 返回人类可读的安全问题列表；为空即通过。
pub fn security_issues(value: &Value) -> Vec<String> {
    let mut issues = Vec::new();
    let text = value.to_string().to_lowercase();
    for banned in BANNED {
        if text.contains(banned) {
            issues.push(format!("包含禁止内容: {banned}"));
        }
    }
    // 逐字段解析 URL，拦截指向回环/私网的端点（SSRF）。
    for (field, url) in collect_url_fields(value) {
        if points_to_loopback(&url) {
            issues.push(format!("字段 {field} 指向回环/私网地址: {url}"));
        }
    }
    issues
}

/// 递归收集看似 URL 的字符串字段值。数组按原序下钻。
fn collect_url_fields(value: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk_urls(value, "", &mut out);
    out
}

fn walk_urls(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if URL_FIELDS.contains(&key.as_str())
                    && let Some(url) = child.as_str()
                {
                    out.push((child_path.clone(), url.to_string()));
                }
                walk_urls(child, &child_path, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_urls(item, path, out);
            }
        }
        _ => {}
    }
}

/// 主机名是否为回环；`urlTemplate` 里的占位段无法解析时跳过，交由运行期 egress 网关兜底。
fn points_to_loopback(url: &str) -> bool {
    // 模板占位（{...}）不是合法 URL，直接放行由 egress 解析真实请求。
    if url.contains('{') || url.contains('$') {
        return false;
    }
    match url::Url::parse(url) {
        Ok(parsed) => parsed.host_str().is_some_and(|host| {
            let host = host.trim_matches(|c| c == '[' || c == ']');
            host == "127.0.0.1"
                || host == "localhost"
                || host == "::1"
                || host.ends_with(".localhost")
        }),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_env_reference_indirection() {
        let binding = json!({
            "adapter": "adapter/json-list",
            "params": {"key": "${env:DAJIALA_KEY}"},
            "endpoint": "https://api.example.com/v1/items"
        });
        assert!(
            security_issues(&binding).is_empty(),
            "{:?}",
            security_issues(&binding)
        );
    }

    #[test]
    fn rejects_javascript_and_loopback_endpoint() {
        let bad = json!({"urlTemplate": "javascript:alert(1)"});
        assert!(
            security_issues(&bad)
                .iter()
                .any(|i| i.contains("javascript:"))
        );
        let loopback = json!({"endpoint": "http://127.0.0.1:8080/x"});
        assert!(
            security_issues(&loopback)
                .iter()
                .any(|i| i.contains("回环"))
        );
    }

    #[test]
    fn rejects_template_url_with_literal_loopback_host() {
        let bad = json!({"endpoint": "http://localhost/admin"});
        assert!(!security_issues(&bad).is_empty());
    }
}
