//! 落库前的响应头脱敏。

use serde_json::Value;
use strata_model::RawEnvelope;

/// 从有效策略读取要脱敏的头名称。缺省为空。
pub fn redact_names(policy: &Value) -> Vec<String> {
    policy
        .pointer("/raw/redact_headers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn redact_headers(headers: &mut [(String, String)], names: &[String]) {
    for (name, value) in headers.iter_mut() {
        if names.iter().any(|needle| eq_ascii(name, needle)) {
            *value = "***".to_string();
        }
    }
}

pub fn redact_envelope(envelope: &mut RawEnvelope, policy: &Value) {
    let names = redact_names(policy);
    redact_headers(&mut envelope.transport.headers, &names);
}

fn eq_ascii(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}
