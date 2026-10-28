//! 结构数据 ↔ 字节。上层只看到 Value。

use serde_json::{Map, Value};

use crate::types::Codec;
use ability_support::{Error, Result};

pub fn encode(codec: Codec, value: &Value) -> Result<Vec<u8>> {
    match codec {
        Codec::Json => serde_json::to_vec_pretty(value).map_err(|e| Error::failed(e.to_string())),
        Codec::Yaml => serde_yaml::to_string(value)
            .map(|s| s.into_bytes())
            .map_err(|e| Error::failed(e.to_string())),
        Codec::Xml => Ok(value_to_xml(value)?.into_bytes()),
        Codec::Md => Ok(value_to_md(value).into_bytes()),
        Codec::Bin => match value.as_str() {
            Some(s) => decode_b64(s),
            None => Err(Error::invalid_args("bin codec 需要 base64 字符串")),
        },
        Codec::Sqlite => serde_json::to_vec(value).map_err(|e| Error::failed(e.to_string())),
    }
}

pub fn decode(codec: Codec, bytes: &[u8]) -> Result<Value> {
    let raw = std::str::from_utf8(bytes).map_err(|e| Error::failed(e.to_string()));
    match codec {
        Codec::Json => serde_json::from_slice(bytes).map_err(|e| Error::failed(e.to_string())),
        Codec::Yaml => serde_yaml::from_str(raw?).map_err(|e| Error::failed(e.to_string())),
        Codec::Xml => xml_to_value(raw?),
        Codec::Md => Ok(md_to_value(raw?)),
        Codec::Bin => Ok(Value::String(b64(bytes))),
        Codec::Sqlite => serde_json::from_slice(bytes).map_err(|e| Error::failed(e.to_string())),
    }
}

pub fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn decode_b64(s: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| Error::invalid_args(format!("base64 无效: {e}")))
}

fn value_to_md(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            let body = map
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match map.get("frontmatter") {
                Some(fm) if !fm.is_null() => {
                    let y = serde_yaml::to_string(fm).unwrap_or_default();
                    format!("---\n{}---\n{body}", y)
                }
                _ => body,
            }
        }
        other => serde_yaml::to_string(other).unwrap_or_default(),
    }
}

fn md_to_value(raw: &str) -> Value {
    let mut map = Map::new();
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let fm = &rest[..end];
            let body = &rest[end + 5..];
            let front: Value = serde_yaml::from_str(fm).unwrap_or(Value::Null);
            map.insert("frontmatter".into(), front);
            map.insert("body".into(), Value::String(body.to_string()));
            return Value::Object(map);
        }
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n');
            let front: Value = serde_yaml::from_str(fm).unwrap_or(Value::Null);
            map.insert("frontmatter".into(), front);
            map.insert("body".into(), Value::String(body.to_string()));
            return Value::Object(map);
        }
    }
    map.insert("frontmatter".into(), Value::Null);
    map.insert("body".into(), Value::String(raw.to_string()));
    Value::Object(map)
}

fn value_to_xml(value: &Value) -> Result<String> {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><doc>");
    write_xml(&mut s, value);
    s.push_str("</doc>");
    Ok(s)
}

fn write_xml(s: &mut String, value: &Value) {
    match value {
        Value::Null => {}
        Value::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => s.push_str(&n.to_string()),
        Value::String(t) => {
            s.push_str(&escape(t));
        }
        Value::Array(arr) => {
            for item in arr {
                s.push_str("<item>");
                write_xml(s, item);
                s.push_str("</item>");
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let tag = xml_name(k);
                s.push('<');
                s.push_str(&tag);
                s.push('>');
                write_xml(s, v);
                s.push_str("</");
                s.push_str(&tag);
                s.push('>');
            }
        }
    }
}

fn xml_name(k: &str) -> String {
    let mut out = String::new();
    for (i, c) in k.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            if i == 0 && c.is_ascii_digit() {
                out.push('_');
            }
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "_".into() } else { out }
}

fn escape(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_to_value(raw: &str) -> Result<Value> {
    use quick_xml::Reader;
    use quick_xml::events::Event;
    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<(String, Map<String, Value>, Vec<Value>, String)> = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                stack.push((name, Map::new(), Vec::new(), String::new()));
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                push_child(&mut stack, &name, Value::Null);
            }
            Ok(Event::Text(t)) => {
                let txt = t
                    .unescape()
                    .map_err(|e| Error::failed(e.to_string()))?
                    .into_owned();
                if let Some(top) = stack.last_mut() {
                    top.3.push_str(&txt);
                }
            }
            Ok(Event::End(_)) => {
                let Some((name, map, items, text)) = stack.pop() else {
                    break;
                };
                let node = if !map.is_empty() {
                    Value::Object(map)
                } else if !items.is_empty() {
                    Value::Array(items)
                } else if text.is_empty() {
                    Value::Null
                } else {
                    parse_scalar(&text)
                };
                if stack.is_empty() {
                    if name == "doc" {
                        return Ok(node);
                    }
                    return Ok(node);
                }
                push_child(&mut stack, &name, node);
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::failed(format!("XML 无效: {e}"))),
            _ => {}
        }
        buf.clear();
    }
    Err(Error::failed("XML 不完整"))
}

fn push_child(
    stack: &mut [(String, Map<String, Value>, Vec<Value>, String)],
    name: &str,
    node: Value,
) {
    let Some(top) = stack.last_mut() else {
        return;
    };
    if name == "item" && top.1.is_empty() {
        top.2.push(node);
        return;
    }
    match top.1.get_mut(name) {
        Some(Value::Array(arr)) => arr.push(node),
        Some(prev) => {
            let old = prev.take();
            *prev = Value::Array(vec![old, node]);
        }
        None => {
            top.1.insert(name.to_string(), node);
        }
    }
}

fn parse_scalar(s: &str) -> Value {
    if s == "true" {
        return Value::Bool(true);
    }
    if s == "false" {
        return Value::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(n) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return Value::Number(num);
        }
    }
    Value::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_yaml_md_roundtrip() {
        let v = json!({"a": 1, "b": "x"});
        let b = encode(Codec::Json, &v).unwrap();
        assert_eq!(decode(Codec::Json, &b).unwrap()["a"], 1);
        let b = encode(Codec::Yaml, &v).unwrap();
        assert_eq!(decode(Codec::Yaml, &b).unwrap()["b"], "x");
        let md = encode(Codec::Md, &json!({"frontmatter": {"t": "n"}, "body": "hi"})).unwrap();
        let back = decode(Codec::Md, &md).unwrap();
        assert_eq!(back["body"], "hi");
        assert_eq!(back["frontmatter"]["t"], "n");
    }

    #[test]
    fn xml_roundtrip_object() {
        let v = json!({"title": "n", "n": 2});
        let b = encode(Codec::Xml, &v).unwrap();
        let back = decode(Codec::Xml, &b).unwrap();
        assert_eq!(back["title"], "n");
        assert_eq!(back["n"], 2);
    }
}
