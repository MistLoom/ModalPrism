//! 统一的出站 HTTP。
//!
//! 模型、通知与工作流里的 HTTP 步骤共用：同一个超时语义、同一份代理设置，
//! 不各自新开一套请求参数。

use std::time::Duration;

use egress::ProxySetting;
use url::Url;

use crate::error::{Error, Result};

/// 一次出站请求的结果。
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    /// 2xx 视为成功，其余带上响应正文的片段。
    pub fn ok(self, what: &str) -> Result<String> {
        if (200..300).contains(&self.status) {
            Ok(self.body)
        } else {
            Err(Error::Remote(format!(
                "{what} 返回 {}: {}",
                self.status,
                clip(&self.body, 300)
            )))
        }
    }
}

pub async fn request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<String>,
    timeout: Duration,
    proxy: &ProxySetting,
) -> Result<Reply> {
    let parsed = Url::parse(url).map_err(|error| Error::invalid(format!("地址无效: {error}")))?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(timeout);
    if let Some(route) = egress::route(proxy, &parsed) {
        let item = reqwest::Proxy::all(route.url.as_str())
            .map_err(|error| Error::local(format!("上游代理不可用: {error}")))?;
        builder = builder.proxy(item);
    }
    let client = builder
        .build()
        .map_err(|error| Error::local(format!("请求器不可用: {error}")))?;
    let verb = reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
        .map_err(|_| Error::invalid(format!("方法无效: {method}")))?;
    let mut request = client.request(verb, parsed);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| Error::Remote(format!("请求失败: {error}")))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| Error::Remote(format!("读取响应失败: {error}")))?;
    Ok(Reply { status, body })
}

pub fn clip(text: &str, limit: usize) -> String {
    let mut out: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::clip;

    #[test]
    fn clip_keeps_char_boundaries() {
        assert_eq!(clip("摘要内容", 2), "摘要…");
        assert_eq!(clip("短", 8), "短");
    }
}
