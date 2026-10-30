//! 三种模型调用方式。
//!
//! - `api`：远端 OpenAI 兼容接口，密钥从环境变量读。
//! - `local`：本机或局域网里的同协议端点（Ollama、vLLM、LM Studio 等），密钥可省略。
//! - `cli`：把提示词写进子进程标准输入，取标准输出（`claude -p`、`llm` 等）。
//!
//! 三种方式对上层是同一个函数：给提示词，拿回文本。

use std::time::Duration;

use egress::ProxySetting;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::http;
use crate::spec::{Mode, ModelSpec};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

pub async fn complete(model: &ModelSpec, prompt: &str, proxy: &ProxySetting) -> Result<String> {
    let text = match model.mode {
        Mode::Cli => cli(model, prompt).await?,
        Mode::Api | Mode::Local => chat(model, prompt, proxy).await?,
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(Error::Remote("模型返回了空内容".into()));
    }
    Ok(text)
}

async fn chat(model: &ModelSpec, prompt: &str, proxy: &ProxySetting) -> Result<String> {
    let endpoint = model.endpoint.as_deref().unwrap_or_default();
    let url = chat_url(endpoint);
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = &model.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.push(json!({"role": "user", "content": prompt}));
    let mut body = json!({
        "model": model.model,
        "messages": messages,
        "stream": false,
    });
    if let Some(max_tokens) = model.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = model.temperature {
        body["temperature"] = json!(temperature);
    }
    let mut headers = model.headers.clone();
    headers.push(("content-type".into(), "application/json".into()));
    // 内置密钥优先；没有才回落到环境变量引用。
    let inline = model
        .key_value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(key) = inline {
        headers.push(("authorization".into(), format!("Bearer {key}")));
    } else if let Some(name) = &model.key_env {
        let key =
            std::env::var(name).map_err(|_| Error::invalid(format!("环境变量未设置: {name}")))?;
        headers.push(("authorization".into(), format!("Bearer {key}")));
    }
    let reply = http::request(
        "POST",
        &url,
        &headers,
        Some(body.to_string()),
        timeout(model),
        proxy,
    )
    .await?;
    let text = reply.ok("模型")?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| Error::Remote(format!("模型返回的不是 JSON: {}", http::clip(&text, 200))))?;
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Remote("模型响应里没有 choices[0].message.content".into()))
}

async fn cli(model: &ModelSpec, prompt: &str) -> Result<String> {
    let Some(program) = model.command.first() else {
        return Err(Error::invalid("cli 模式缺少 command"));
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(&model.command[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| Error::local(format!("无法启动 {program}: {error}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|error| Error::local(format!("写入标准输入失败: {error}")))?;
        stdin
            .shutdown()
            .await
            .map_err(|error| Error::local(format!("关闭标准输入失败: {error}")))?;
    }
    let output = tokio::time::timeout(timeout(model), child.wait_with_output())
        .await
        .map_err(|_| {
            Error::Remote(format!(
                "{program} 超过 {} 未返回",
                timeout(model).as_secs()
            ))
        })?
        .map_err(|error| Error::local(format!("等待 {program} 失败: {error}")))?;
    if !output.status.success() {
        return Err(Error::Remote(format!(
            "{program} 退出码 {:?}: {}",
            output.status.code(),
            http::clip(&String::from_utf8_lossy(&output.stderr), 300)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn chat_url(endpoint: &str) -> String {
    if endpoint.ends_with("/chat/completions") {
        endpoint.to_string()
    } else {
        format!("{}/chat/completions", endpoint.trim_end_matches('/'))
    }
}

fn timeout(model: &ModelSpec) -> Duration {
    model
        .timeout
        .as_deref()
        .and_then(config::duration)
        .unwrap_or(DEFAULT_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::chat_url;

    #[test]
    fn endpoint_is_completed_once() {
        assert_eq!(
            chat_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("http://127.0.0.1:11434/v1/chat/completions"),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }
}
