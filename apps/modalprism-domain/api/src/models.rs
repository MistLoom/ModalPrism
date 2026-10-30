//! 模型资源的调用封装：把 `model` 文档翻译成一次模型请求。
//!
//! 这里是「投递到模型」与信源 AI 分析的唯一出口；系统设置只提供采样与超时的全局覆盖。

use serde_json::Value;

use crate::AppState;
use crate::ApiError;

/// 把模型资源转换成一次调用的声明。密钥优先取文档内置值，其次才是环境变量引用。
pub(crate) fn model_spec(model: &Value) -> Result<workflow::ModelSpec, ApiError> {
    let kind = model["kind"].as_str().unwrap_or("remoteApi");
    let mode = match kind {
        "remoteApi" => workflow::Mode::Api,
        "localService" => workflow::Mode::Local,
        "localCli" => workflow::Mode::Cli,
        other => {
            return Err(ApiError::validation(
                "kind",
                "enum",
                format!("未知模型类型: {other}"),
            ));
        }
    };
    let endpoint = model["endpoint"].as_str().unwrap_or_default().to_string();
    let secret_inline = model["secretValue"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let secret_ref = model["secretRef"]
        .as_str()
        .filter(|value| !value.is_empty());
    if secret_inline.is_none()
        && let Some(name) = secret_ref
        && std::env::var(name).is_err()
    {
        return Err(ApiError::dependency(format!("环境变量 {name} 未设置")).retryable());
    }
    let timeout = model["timeoutMs"]
        .as_i64()
        .map(crate::util::duration_text)
        .unwrap_or_else(|| "90s".into());
    let headers: Vec<(String, String)> = model["headers"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(workflow::ModelSpec {
        id: None,
        kind: None,
        name: model["name"].as_str().map(str::to_string),
        description: None,
        mode,
        endpoint: Some(endpoint.clone()),
        model: model["model"].as_str().map(str::to_string),
        key_env: if secret_inline.is_some() {
            None
        } else {
            secret_ref.map(str::to_string)
        },
        key_value: secret_inline,
        headers,
        command: if mode == workflow::Mode::Cli {
            endpoint.split_whitespace().map(str::to_string).collect()
        } else {
            Vec::new()
        },
        system: None,
        timeout: Some(timeout),
        max_tokens: model["maxTokens"].as_u64().map(|value| value as u32),
        temperature: model["temperature"].as_f64().map(|value| value as f32),
    })
}

pub async fn call_model(
    state: &AppState,
    model: &Value,
    prompt: &str,
    operation_id: &str,
) -> Result<String, ApiError> {
    if !model["enabled"].as_bool().unwrap_or(false) {
        return Err(ApiError::dependency("模型已停用").retryable());
    }
    let mut spec = model_spec(model)?;
    crate::operations::check_cancel(state, operation_id).await?;
    // 采样与超时：模型自己声明的缺省优先（登记时按预设带出对应模型的最佳值），
    // 系统设置只在模型未声明时托底，避免全局值把每个模型拉成同一档。
    // 显式声明为 null 表示该模型不接受自定义采样参数（思考型模型），保持不发送。
    if model.get("temperature").is_none()
        && spec.temperature.is_none()
        && let Some(value) = crate::settings::value(state, "ai.temperature").await?.as_f64()
    {
        spec.temperature = Some(value as f32);
    }
    if spec.max_tokens.is_none()
        && let Some(value) = crate::settings::value(state, "ai.maxTokens").await?.as_u64()
    {
        spec.max_tokens = Some(value.min(u32::MAX as u64) as u32);
    }
    if let Some(value) = crate::settings::value(state, "ai.requestTimeoutMs")
        .await?
        .as_u64()
    {
        spec.timeout = Some(crate::util::duration_text(value as i64));
    }
    let proxy = state.services.proxy_setting();
    let reply = workflow::complete_model(&spec, prompt, &proxy)
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("环境变量") || message.contains("超时") || message.contains("连接")
            {
                ApiError::dependency(message).retryable()
            } else {
                ApiError::dependency(message)
            }
        })?;
    Ok(reply)
}

/// 从模型回复里提取第一段可解析的 JSON。
pub fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Value>(&trimmed[start..=end]).ok()
}
