//! 媒体类型、URL 和文本的纯函数。

use strata_model::{Locator, Timestamp};
use time::format_description::well_known::{Rfc2822, Rfc3339};
use time::{Date, Month, OffsetDateTime};

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

/// 配置里的日期模式。使用第 1 个捕获组；模式无法编译或没有捕获组时返回空。
pub fn first_capture(pattern: &str, text: &str) -> Option<String> {
    let expression = regex::Regex::new(pattern).ok()?;
    let groups = expression.captures(text)?;
    let raw = groups.get(1)?.as_str().trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

pub fn capture_pattern(pattern: &str) -> Result<(), regex::Error> {
    regex::Regex::new(pattern).map(|_| ())
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
    if let Some(parsed) = weekday_stamp(raw) {
        return Some(parsed);
    }
    let date = OffsetDateTime::parse(raw, &Rfc2822).ok()?;
    let text = date.format(&Rfc3339).ok()?;
    Timestamp::parse(&text).ok()
}

/// 按单位或墙上时间读发布时间。纯日期按 UTC 零点。没有时区的日期时间使用 `offset`，缺省 `+08:00`。
pub fn parse_published(raw: &str, offset: Option<&str>, unit: Option<&str>) -> Option<Timestamp> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match unit {
        Some("epoch_s") => return epoch_millis(raw, 1000),
        Some("epoch_ms") => return epoch_millis(raw, 1),
        Some("yyyymmdd") => {
            let (year, month, day) = compact_date(raw)?;
            return utc_midnight(year, month, day);
        }
        Some(_) => return None,
        None => {}
    }
    if let Some(parsed) = parse_timestamp(raw) {
        return Some(parsed);
    }
    if let Some((year, month, day)) = split_date(raw) {
        return utc_midnight(year, month, day);
    }
    let (year, month, day, hour, minute, second) = split_naive(raw)?;
    at_offset(
        year,
        month,
        day,
        hour,
        minute,
        second,
        offset.unwrap_or("+08:00"),
    )
}

pub fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }
    let mut current = value;
    for segment in path.split('.') {
        current = match current {
            serde_json::Value::Array(items) => {
                let index: usize = segment.parse().ok()?;
                items.get(index)?
            }
            serde_json::Value::Object(map) => map.get(segment)?,
            _ => return None,
        };
    }
    Some(current)
}

pub fn render_template(template: &str, item: &serde_json::Value) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}')?;
        let token = &after[..end];
        let (raw, path) = token
            .strip_prefix("raw:")
            .map(|path| (true, path))
            .unwrap_or((false, token));
        let value = json_path(item, path)?;
        let text = scalar_text(value)?;
        if raw {
            out.push_str(&text);
        } else {
            out.push_str(&encode_component(&text).replace("%2F", "/"));
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    if out.is_empty() { None } else { Some(out) }
}

pub fn decode_basic_entities(raw: &str) -> String {
    let mut current = raw.to_string();
    for _ in 0..2 {
        let next = decode_entities_once(&current);
        if next == current {
            return current;
        }
        current = next;
    }
    current
}

fn decode_entities_once(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(';') else {
            out.push('&');
            rest = after;
            break;
        };
        if let Some(ch) = entity_char(&after[..end]) {
            out.push(ch);
            rest = &after[end + 1..];
        } else {
            out.push('&');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn entity_char(body: &str) -> Option<char> {
    match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => numeric_entity(body),
    }
}

fn numeric_entity(body: &str) -> Option<char> {
    let digits = body.strip_prefix('#')?;
    let code = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        digits.parse().ok()?
    };
    char::from_u32(code).filter(|ch| *ch != '\0')
}

pub fn http_absolute(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    Some(url.to_string())
}

fn epoch_millis(raw: &str, scale: i64) -> Option<Timestamp> {
    let value: i64 = raw.parse().ok()?;
    let millis = value.checked_mul(scale)?;
    Timestamp::from_unix_millis(millis).ok()
}

fn compact_date(raw: &str) -> Option<(i32, u8, u8)> {
    if raw.len() != 8 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: i32 = raw[..4].parse().ok()?;
    let month: u8 = raw[4..6].parse().ok()?;
    let day: u8 = raw[6..8].parse().ok()?;
    calendar(year, month, day)?;
    Some((year, month, day))
}

fn split_date(raw: &str) -> Option<(i32, u8, u8)> {
    let mut parts = raw.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    calendar(year, month, day)?;
    Some((year, month, day))
}

fn split_naive(raw: &str) -> Option<(i32, u8, u8, u8, u8, u8)> {
    let split = raw.find(['T', ' '])?;
    let (year, month, day) = split_date(&raw[..split])?;
    let mut clock = raw[split + 1..].split(':');
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second = match clock.next() {
        None => 0,
        Some(text) if clock.next().is_none() => text.parse().ok()?,
        Some(_) => return None,
    };
    Some((year, month, day, hour, minute, second))
}

fn utc_midnight(year: i32, month: u8, day: u8) -> Option<Timestamp> {
    at_offset(year, month, day, 0, 0, 0, "+00:00")
}

fn at_offset(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    offset: &str,
) -> Option<Timestamp> {
    calendar(year, month, day)?;
    if hour > 23 || minute > 59 || second > 59 || !offset_ok(offset) {
        return None;
    }
    Timestamp::parse(&format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{offset}"
    ))
    .ok()
}

fn calendar(year: i32, month: u8, day: u8) -> Option<Date> {
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

fn offset_ok(offset: &str) -> bool {
    let bytes = offset.as_bytes();
    bytes.len() == 6
        && (bytes[0] == b'+' || bytes[0] == b'-')
        && bytes[3] == b':'
        && bytes[1..3].iter().all(u8::is_ascii_digit)
        && bytes[4..6].iter().all(u8::is_ascii_digit)
}

fn weekday_stamp(raw: &str) -> Option<Timestamp> {
    let mut parts = raw.split_whitespace();
    let _weekday = parts.next()?;
    let month = month_index(parts.next()?)?;
    let day: u8 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    let zone = normalize_zone(parts.next()?)?;
    let year: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let mut clock = time.split(':');
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second: u8 = clock.next()?.parse().ok()?;
    if clock.next().is_some() {
        return None;
    }
    at_offset(year, month, day, hour, minute, second, &zone)
}

fn normalize_zone(raw: &str) -> Option<String> {
    if offset_ok(raw) {
        return Some(raw.to_string());
    }
    let bytes = raw.as_bytes();
    if bytes.len() == 5
        && (bytes[0] == b'+' || bytes[0] == b'-')
        && bytes[1..].iter().all(u8::is_ascii_digit)
    {
        return Some(format!("{}:{}", &raw[..3], &raw[3..5]));
    }
    None
}

fn month_index(name: &str) -> Option<u8> {
    Some(match name {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn encode_component(raw: &str) -> String {
    let mut out = String::new();
    for byte in raw.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
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
