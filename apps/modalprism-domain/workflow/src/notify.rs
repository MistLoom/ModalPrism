//! 通知投递。
//!
//! 两种形态：`webhook` 把结构化结果原样 POST 出去；`feishu` 按飞书自定义机器人的
//! 文本消息格式包一层。飞书用 HTTP 200 加 `code != 0` 表示失败，所以成功与否要看正文。
//!
//! 投递只用一份代理设置，与采集共用 `policy.egress.proxy`。

use std::time::Duration;

use egress::ProxySetting;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::http;
use crate::spec::{Notify, NotifyKind};

const TIMEOUT: Duration = Duration::from_secs(15);

/// 投递一条通知。失败不会回滚已经落盘的结果，调用方只记录。
pub async fn deliver(notify: &Notify, payload: &Value, proxy: &ProxySetting) -> Result<()> {
    let title = notify
        .title
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "ModalPrism".into());
    let (body, what) = match notify.kind {
        NotifyKind::Webhook => (webhook_body(&title, payload), "Webhook"),
        NotifyKind::Feishu => (feishu_body(&title, payload), "飞书"),
    };
    let headers = [("content-type".to_string(), "application/json".to_string())];
    let reply = http::request(
        "POST",
        &notify.url,
        &headers,
        Some(body.to_string()),
        TIMEOUT,
        proxy,
    )
    .await?;
    let text = reply.ok(what)?;
    if notify.kind == NotifyKind::Feishu {
        check_feishu(&text)?;
    }
    Ok(())
}

fn webhook_body(title: &str, payload: &Value) -> Value {
    match payload {
        Value::Object(map) => {
            let mut map = map.clone();
            map.entry("title")
                .or_insert_with(|| Value::String(title.to_string()));
            Value::Object(map)
        }
        other => json!({"title": title, "payload": other}),
    }
}

fn feishu_body(title: &str, payload: &Value) -> Value {
    json!({
        "msg_type": "text",
        "content": {"text": format!("{title}\n{}", summary(payload))},
    })
}

/// 飞书的文本卡片只放最关键的几行，细节留在运行记录里。
fn summary(payload: &Value) -> String {
    let mut lines = Vec::new();
    for (label, path) in [
        ("工作流", "/workflow"),
        ("状态", "/status"),
        ("标题", "/title"),
        ("说明", "/message"),
        ("错误", "/error"),
    ] {
        if let Some(text) = payload.pointer(path).and_then(Value::as_str) {
            if !text.is_empty() {
                lines.push(format!("{label}：{text}"));
            }
        }
    }
    if lines.is_empty() {
        return http::clip(&payload.to_string(), 400);
    }
    lines.join("\n")
}

fn check_feishu(text: &str) -> Result<()> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(());
    };
    let code = value
        .get("code")
        .or_else(|| value.get("StatusCode"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if code != 0 {
        let message = value
            .get("msg")
            .or_else(|| value.get("StatusMessage"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(Error::Remote(format!(
            "飞书返回 {code}: {message} {}",
            http::clip(text, 160)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feishu_text_keeps_key_lines() {
        let payload = json!({"workflow": "摘要", "status": "ok", "title": "一篇文章"});
        let body = feishu_body("加工完成", &payload);
        let text = body
            .pointer("/content/text")
            .and_then(Value::as_str)
            .unwrap();
        assert!(text.contains("加工完成"));
        assert!(text.contains("工作流：摘要"));
        assert_eq!(body.get("msg_type").and_then(Value::as_str), Some("text"));
    }

    #[test]
    fn webhook_keeps_payload_fields() {
        let payload = json!({"workflow": "摘要", "status": "ok"});
        let body = webhook_body("标题", &payload);
        assert_eq!(body.get("workflow").and_then(Value::as_str), Some("摘要"));
        assert_eq!(body.get("title").and_then(Value::as_str), Some("标题"));
    }

    #[test]
    fn feishu_error_code_is_a_failure() {
        assert!(check_feishu(r#"{"code":19002,"msg":"sign match fail"}"#).is_err());
        assert!(check_feishu(r#"{"code":0,"msg":"success"}"#).is_ok());
        assert!(check_feishu("ok").is_ok());
    }
}
