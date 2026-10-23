//! 内置检测器与识别适配器使用的同一套判定。

use async_trait::async_trait;
use strata_model::{PartId, PartRole};

use crate::model::{Declared, Detection, ReadonlyView, RegionHit};
use crate::textutil::{collapse_ws, essence, find_ascii_ci};
use crate::traits::Detector;

const SNIFF: usize = 65_536;
const SCAN: usize = 1024 * 1024;

pub struct BasicDetector;

impl BasicDetector {
    pub fn detect_view(&self, view: &ReadonlyView, declared: &Declared) -> Detection {
        let sample = prefix(&view.bytes, SNIFF);
        let charset = charset_of(sample, &view.bytes);
        let (media_type, confidence) = media_of(sample, &essence(&declared.media_type));
        let mut traits = Vec::new();
        if media_type == "text/html" {
            let scan = prefix(&view.bytes, SCAN);
            if visible_len(scan) < 40 {
                traits.push("shell-page".to_string());
            }
            if contains_any(
                scan,
                &[
                    b"cf-challenge",
                    b"captcha",
                    b"attention required",
                    b"just a moment",
                ],
            ) {
                traits.push("challenge-page".to_string());
            }
            if find_ascii_ci(scan, b"application/ld+json").is_some() {
                traits.push("has-jsonld".to_string());
            }
            if find_ascii_ci(scan, b"<img").is_some() {
                traits.push("has-images".to_string());
            }
            traits.sort();
        }
        Detection {
            media_type,
            charset,
            confidence,
            traits,
        }
    }

    pub fn regions_view(
        &self,
        view: &ReadonlyView,
        part_id: &str,
        wanted: &[String],
    ) -> Vec<RegionHit> {
        if !wanted.iter().any(|item| item == "json-ld") {
            return Vec::new();
        }
        let scan = prefix(&view.bytes, SCAN);
        let mut hits = Vec::new();
        let mut offset = 0;
        while let Some(found) = find_ascii_ci(&scan[offset..], b"application/ld+json") {
            let marker = offset + found;
            let Some(rel_gt) = scan[marker..].iter().position(|byte| *byte == b'>') else {
                break;
            };
            let raw_start = marker + rel_gt + 1;
            let Some(rel_end) = find_ascii_ci(&scan[raw_start..], b"</script>") else {
                break;
            };
            let raw_end = raw_start + rel_end;
            let (start, end) = trimmed_range(scan, raw_start, raw_end);
            if serde_json::from_slice::<serde_json::Value>(&scan[start..end]).is_ok() {
                hits.push(RegionHit {
                    r#type: "json-ld".to_string(),
                    part_id: part_id.to_string(),
                    start: start as u64,
                    end: end as u64,
                    confidence: 0.9,
                });
            }
            offset = raw_end + "</script>".len();
        }
        hits
    }
}

fn trimmed_range(bytes: &[u8], start: usize, end: usize) -> (usize, usize) {
    let slice = &bytes[start..end];
    let lead = slice
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    let trail = slice
        .iter()
        .rev()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    let body_end = end.saturating_sub(trail);
    let body_start = (start + lead).min(body_end);
    (body_start, body_end)
}

pub struct PartDetection {
    pub part_id: PartId,
    pub role: PartRole,
    pub detection: Detection,
}

pub fn detection_body(parts: &[PartDetection]) -> serde_json::Value {
    let primary = parts
        .iter()
        .find(|part| part.role == PartRole::Document)
        .or_else(|| parts.first());
    let (media_type, charset, confidence, traits) = match primary {
        Some(part) => (
            part.detection.media_type.clone(),
            part.detection.charset.clone(),
            part.detection.confidence,
            part.detection.traits.clone(),
        ),
        None => (
            "application/octet-stream".to_string(),
            None,
            0.2,
            Vec::new(),
        ),
    };
    serde_json::json!({
        "mediaType": media_type,
        "charset": charset,
        "confidence": confidence,
        "traits": traits,
        "parts": parts.iter().map(|part| serde_json::json!({
            "partId": part.part_id.as_str(),
            "mediaType": part.detection.media_type,
            "charset": part.detection.charset,
            "confidence": part.detection.confidence,
            "traits": part.detection.traits,
        })).collect::<Vec<_>>(),
    })
}

fn prefix(bytes: &[u8], limit: usize) -> &[u8] {
    &bytes[..bytes.len().min(limit)]
}

fn media_of(sample: &[u8], declared: &str) -> (String, f64) {
    if let Some(media) = magic(sample) {
        return (media, 0.95);
    }
    let rest = skip_bom(sample);
    let trimmed = trim_start_ws(rest);
    if trimmed.first() == Some(&b'<') {
        return markup(trimmed);
    }
    if trimmed.first() == Some(&b'{') || trimmed.first() == Some(&b'[') {
        return ("application/json".to_string(), 0.9);
    }
    if !declared.is_empty() {
        return (declared.to_string(), 0.5);
    }
    ("application/octet-stream".to_string(), 0.2)
}

fn magic(sample: &[u8]) -> Option<String> {
    if sample.starts_with(b"%PDF-") {
        return Some("application/pdf".into());
    }
    if sample.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png".into());
    }
    if sample.starts_with(b"\xFF\xD8\xFF") {
        return Some("image/jpeg".into());
    }
    if sample.starts_with(b"GIF87a") || sample.starts_with(b"GIF89a") {
        return Some("image/gif".into());
    }
    if sample.len() >= 12 && sample.starts_with(b"RIFF") && &sample[8..12] == b"WEBP" {
        return Some("image/webp".into());
    }
    if sample.starts_with(b"PK\x03\x04") {
        return Some("application/zip".into());
    }
    if sample.starts_with(b"\x1f\x8b") {
        return Some("application/gzip".into());
    }
    None
}

fn markup(sample: &[u8]) -> (String, f64) {
    if let Some(name) = first_element(sample) {
        let media = match name.as_str() {
            "rss" => "application/rss+xml",
            "feed" => "application/atom+xml",
            "html" => "text/html",
            _ => {
                if find_ascii_ci(sample, b"<!doctype html").is_some()
                    || find_ascii_ci(sample, b"<html").is_some()
                {
                    "text/html"
                } else {
                    "application/xml"
                }
            }
        };
        return (media.to_string(), 0.95);
    }
    if find_ascii_ci(sample, b"<!doctype html").is_some()
        || find_ascii_ci(sample, b"<html").is_some()
    {
        ("text/html".to_string(), 0.95)
    } else {
        ("application/xml".to_string(), 0.95)
    }
}

fn first_element(sample: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(sample);
    let mut reader = quick_xml::Reader::from_str(&text);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(event) | quick_xml::events::Event::Empty(event)) => {
                return Some(local_name(&event));
            }
            Ok(quick_xml::events::Event::DocType(raw)) => {
                if find_ascii_ci(&raw, b"html").is_some() {
                    return Some("html".into());
                }
            }
            Ok(quick_xml::events::Event::Eof) => return None,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

fn local_name(event: &quick_xml::events::BytesStart<'_>) -> String {
    String::from_utf8_lossy(event.local_name().as_ref()).to_ascii_lowercase()
}

fn charset_of(sample: &[u8], all: &[u8]) -> Option<String> {
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some("utf-8".into());
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return Some("utf-16be".into());
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return Some("utf-16le".into());
    }
    let scan = prefix(all, SNIFF);
    if let Some(found) = find_ascii_ci(scan, b"charset=") {
        return Some(read_token(&scan[found + "charset=".len()..]));
    }
    if let Some(found) = find_ascii_ci(scan, b"encoding=") {
        return Some(read_token(&scan[found + "encoding=".len()..]));
    }
    None
}

fn read_token(bytes: &[u8]) -> String {
    let bytes = bytes
        .strip_prefix(b"\"")
        .or_else(|| bytes.strip_prefix(b"'"))
        .unwrap_or(bytes);
    let end = bytes
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'-' && *byte != b'_')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase()
}

fn skip_bom(sample: &[u8]) -> &[u8] {
    sample
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .or_else(|| sample.strip_prefix(&[0xFE, 0xFF]))
        .or_else(|| sample.strip_prefix(&[0xFF, 0xFE]))
        .unwrap_or(sample)
}

fn trim_start_ws(sample: &[u8]) -> &[u8] {
    let lead = sample
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    &sample[lead..]
}

fn contains_any(hay: &[u8], needles: &[&[u8]]) -> bool {
    needles
        .iter()
        .any(|needle| find_ascii_ci(hay, needle).is_some())
}

fn visible_len(bytes: &[u8]) -> usize {
    let mut skipped = Vec::new();
    let lower_scan = bytes;
    let mut index = 0;
    while index < lower_scan.len() {
        if starts_ci(&lower_scan[index..], b"<script") || starts_ci(&lower_scan[index..], b"<style")
        {
            let close: &[u8] = if starts_ci(&lower_scan[index..], b"<script") {
                b"</script>"
            } else {
                b"</style>"
            };
            if let Some(found) = find_ascii_ci(&lower_scan[index + 1..], close) {
                index = index + 1 + found + close.len();
                continue;
            }
        }
        skipped.push(lower_scan[index]);
        index += 1;
    }
    let text = String::from_utf8_lossy(&skipped);
    let mut plain = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => plain.push(ch),
            _ => {}
        }
    }
    collapse_ws(&plain).chars().count()
}

#[async_trait]
impl Detector for BasicDetector {
    fn id(&self) -> &str {
        "detector/basic"
    }

    fn version(&self) -> &str {
        "1"
    }

    async fn detect(
        &self,
        view: &ReadonlyView,
        declared: &Declared,
    ) -> Result<Detection, crate::error::AdapterError> {
        Ok(self.detect_view(view, declared))
    }

    async fn regions(&self, view: &ReadonlyView, wanted: &[String]) -> Vec<RegionHit> {
        self.regions_view(view, "", wanted)
    }
}

fn starts_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len()
        && hay[..needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}
