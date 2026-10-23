//! 响应头、压缩和下一页链接。

use std::io::Read;
use std::time::Duration;

use bytes::Bytes;
use flate2::read::GzDecoder;
use strata_model::Signals;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

use crate::ready::permanent;

pub fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .rev()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub fn media_type(headers: &[(String, String)]) -> (String, Option<String>) {
    let Some(raw) = header(headers, "content-type") else {
        return ("application/octet-stream".into(), None);
    };
    let mut parts = raw.split(';');
    let kind = parts
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_ascii_lowercase();
    let kind = if kind.is_empty() {
        "application/octet-stream".into()
    } else {
        kind
    };
    let charset = parts.find_map(|part| {
        let part = part.trim();
        let (name, value) = part.split_once('=')?;
        if name.trim().eq_ignore_ascii_case("charset") {
            Some(value.trim().trim_matches('"').to_string())
        } else {
            None
        }
    });
    (kind, charset)
}

pub fn decode_body(
    headers: &mut Vec<(String, String)>,
    body: Bytes,
) -> Result<Bytes, accessors::AccessError> {
    let encoding = header(headers, "content-encoding")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if encoding.is_empty() || encoding == "identity" {
        return Ok(body);
    }
    if encoding != "gzip" {
        return Err(permanent("encoding"));
    }
    let mut decoder = GzDecoder::new(body.as_ref());
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|_| permanent("encoding"))?;
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"));
    if let Some((_, value)) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        *value = decoded.len().to_string();
    }
    Ok(Bytes::from(decoded))
}

pub fn signals(headers: &[(String, String)]) -> Signals {
    Signals {
        retry_after: header(headers, "retry-after").and_then(retry_after),
        ttl: max_age(headers),
        cache_max_age: max_age(headers),
    }
}

fn max_age(headers: &[(String, String)]) -> Option<Duration> {
    let value = header(headers, "cache-control")?;
    for part in value.split(',') {
        let part = part.trim();
        let Some(rest) = part.strip_prefix("max-age=") else {
            continue;
        };
        let seconds: u64 = rest.trim().parse().ok()?;
        return Some(Duration::from_secs(seconds));
    }
    None
}

fn retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let when = OffsetDateTime::parse(value, &Rfc2822).ok()?;
    let now = OffsetDateTime::now_utc();
    let delta = when - now;
    if delta.is_negative() {
        return Some(Duration::ZERO);
    }
    delta.try_into().ok()
}

pub fn link_next(headers: &[(String, String)]) -> Option<String> {
    headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("link") {
            next_in_link(value)
        } else {
            None
        }
    })
}

fn next_in_link(value: &str) -> Option<String> {
    split_links(value).into_iter().find_map(|part| {
        if rel_has_next(part) {
            angle_url(part)
        } else {
            None
        }
    })
}

fn split_links(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut angle = 0i32;
    let mut quote = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '"' if angle == 0 => quote = !quote,
            '<' if !quote => angle += 1,
            '>' if !quote && angle > 0 => angle -= 1,
            ',' if !quote && angle == 0 => {
                let piece = value[start..index].trim();
                if !piece.is_empty() {
                    out.push(piece);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = value[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn rel_has_next(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    let Some(at) = lower.find("rel=") else {
        return false;
    };
    let rest = part[at + 4..].trim_start();
    let tokens = if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"').unwrap_or(stripped.len());
        &stripped[..end]
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'').unwrap_or(stripped.len());
        &stripped[..end]
    } else {
        rest.split([';', ' ', ',']).next().unwrap_or("")
    };
    tokens
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("next"))
}

fn angle_url(part: &str) -> Option<String> {
    let start = part.find('<')? + 1;
    let end = part[start..].find('>')? + start;
    let url = part[start..end].trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

pub fn body_next(body: &[u8]) -> Option<String> {
    let slice = &body[..body.len().min(65536)];
    let text = String::from_utf8_lossy(slice);
    let lower = text.to_ascii_lowercase();
    let at = ["rel=\"next\"", "rel='next'", "rel=next"]
        .into_iter()
        .filter_map(|pattern| lower.find(pattern))
        .min()?;
    let from = char_floor(&text, at.saturating_sub(300));
    let to = char_ceil(&text, (at + 300).min(text.len()));
    closest_href(&text[from..to], at - from)
}

fn char_floor(text: &str, index: usize) -> usize {
    let mut at = index.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn char_ceil(text: &str, index: usize) -> usize {
    let mut at = index.min(text.len());
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

fn closest_href(window: &str, anchor: usize) -> Option<String> {
    let lower = window.to_ascii_lowercase();
    let mut best: Option<(usize, String)> = None;
    let mut search_from = 0;
    while let Some(found) = lower[search_from..].find("href=") {
        let at = search_from + found;
        if let Some(href) = href_at(&window[at..]) {
            let distance = at.abs_diff(anchor);
            if best
                .as_ref()
                .is_none_or(|(best_at, _)| distance < anchor.abs_diff(*best_at))
            {
                best = Some((at, href));
            }
        }
        search_from = at + 5;
    }
    best.map(|(_, href)| href)
}

fn href_at(window: &str) -> Option<String> {
    let rest = window[5..].trim_start();
    let (quote, body) = if let Some(stripped) = rest.strip_prefix('"') {
        ('"', stripped)
    } else {
        let stripped = rest.strip_prefix('\'')?;
        ('\'', stripped)
    };
    let end = body.find(quote)?;
    let href = body[..end].trim();
    if href.is_empty() {
        None
    } else {
        Some(href.to_string())
    }
}

pub fn resolve_url(base: &str, next: &str) -> Option<String> {
    let base = url::Url::parse(base).ok()?;
    base.join(next).ok().map(|url| url.to_string())
}

pub fn is_http(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .is_some_and(|url| url.scheme() == "http" || url.scheme() == "https")
}

#[cfg(test)]
mod tests {
    use super::body_next;

    #[test]
    fn body_next_slices_on_char_boundaries() {
        let mut body = "a".repeat(10);
        body.push('你');
        body.push_str(&"b".repeat(298));
        body.push_str(r#"rel="next" href="https://example.com/n""#);
        assert_eq!(
            body_next(body.as_bytes()).as_deref(),
            Some("https://example.com/n")
        );
    }
}
