//! 媒体类型、URL 和文本的纯函数。

use strata_model::{Locator, Timestamp};
use time::OffsetDateTime;
use time::format_description::well_known::{Rfc2822, Rfc3339};

pub fn essence(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase()
}

pub fn locator_text(locator: &Locator) -> &str {
    match locator {
        Locator::Url { url } => url,
        Locator::Query { query } => query,
        Locator::ApiEndpoint { endpoint } => endpoint,
        Locator::PushChannel { channel } => channel,
    }
}

pub fn absolute_http(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    let url = if let Ok(parsed) = url::Url::parse(href) {
        parsed
    } else {
        let base = url::Url::parse(base).ok()?;
        base.join(href).ok()?
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    Some(url.to_string())
}

pub fn url_identity(absolute: &str) -> Option<String> {
    if absolute.is_empty() || absolute.chars().any(char::is_whitespace) {
        None
    } else {
        Some(format!("url:{absolute}"))
    }
}

pub fn guid_identity(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_whitespace) {
        None
    } else {
        Some(format!("ext:guid:{raw}"))
    }
}

pub fn parse_timestamp(raw: &str) -> Option<Timestamp> {
    let raw = raw.trim();
    if let Ok(parsed) = Timestamp::parse(raw) {
        return Some(parsed);
    }
    let date = OffsetDateTime::parse(raw, &Rfc2822).ok()?;
    let text = date.format(&Rfc3339).ok()?;
    Timestamp::parse(&text).ok()
}

pub fn parse_duration(raw: &str) -> Option<std::time::Duration> {
    let unit_at = raw.find(|c: char| !c.is_ascii_digit())?;
    let (number, unit) = raw.split_at(unit_at);
    if number.is_empty() || number.starts_with('0') || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: u64 = number.parse().ok()?;
    let seconds = match unit {
        "ms" => return Some(std::time::Duration::from_millis(value)),
        "s" => value,
        "m" => value.checked_mul(60)?,
        "h" => value.checked_mul(3600)?,
        "d" => value.checked_mul(86_400)?,
        _ => return None,
    };
    Some(std::time::Duration::from_secs(seconds))
}

pub fn collapse_ws(raw: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            pending = !out.is_empty();
            continue;
        }
        if pending {
            out.push(' ');
            pending = false;
        }
        out.push(ch);
    }
    out
}

pub fn strip_tags(raw: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    collapse_ws(&out)
}

pub fn find_ascii_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

pub fn params_hash(spec: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(spec).unwrap_or_else(|_| b"null".to_vec());
    strata_model::content_hash(&bytes)
}
