//! YAML 1.2 解析。显式标签与重复键使文档失败。

use saphyr::{LoadableYamlNode, MarkedYamlOwned, ScalarOwned, YamlDataOwned};

use crate::node::{Data, Node};

pub fn parse(file: &str, text: &str) -> Result<Node, String> {
    let docs = MarkedYamlOwned::load_from_str(text).map_err(|error| error.to_string())?;
    if docs.len() != 1 {
        return Err("一个文件只能有一个 YAML 文档".into());
    }
    convert(file, &docs[0])
}

/// 把 YAML 直接解析成 JSON 值，供不参与快照的声明（如内容加工）使用。
pub fn parse_value(file: &str, text: &str) -> Result<serde_json::Value, String> {
    parse(file, text).map(|node| data_value(node.data))
}

fn data_value(data: Data) -> serde_json::Value {
    match data {
        Data::Null => serde_json::Value::Null,
        Data::Bool(value) => serde_json::Value::Bool(value),
        Data::Int(value) => serde_json::Value::from(value),
        Data::Float(value) => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Data::String(value) => serde_json::Value::String(value),
        Data::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| data_value(item.data))
                .collect(),
        ),
        Data::Map(entries) => serde_json::Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, data_value(value.data)))
                .collect(),
        ),
    }
}

fn convert(file: &str, node: &MarkedYamlOwned) -> Result<Node, String> {
    let line = line_number(node);
    let data = match &node.data {
        YamlDataOwned::Value(value) => scalar(value)?,
        YamlDataOwned::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(convert(file, item)?);
            }
            Data::Array(out)
        }
        YamlDataOwned::Mapping(map) => {
            let mut entries = Vec::with_capacity(map.len());
            for (key, value) in map {
                let name = match &key.data {
                    YamlDataOwned::Value(ScalarOwned::String(text)) => text.clone(),
                    _ => return Err(format!("{file}:{line}: 映射键必须是字符串")),
                };
                entries.push((name, convert(file, value)?));
            }
            Data::Map(entries)
        }
        YamlDataOwned::Tagged(tag, _) => {
            return Err(format!("{file}:{line}: 不允许显式标签 {tag}"));
        }
        YamlDataOwned::Representation(_, _, tag) => {
            return Err(format!(
                "{file}:{line}: 不允许未解析的标量{}",
                tag.as_ref()
                    .map(|item| format!(" {item}"))
                    .unwrap_or_default()
            ));
        }
        YamlDataOwned::Alias(_) => return Err(format!("{file}:{line}: 不允许别名")),
        YamlDataOwned::BadValue => return Err(format!("{file}:{line}: 无法解析标量")),
    };
    Ok(Node {
        file: file.to_string(),
        line,
        preset_id: None,
        data,
    })
}

fn scalar(value: &ScalarOwned) -> Result<Data, String> {
    Ok(match value {
        ScalarOwned::Null => Data::Null,
        ScalarOwned::Boolean(value) => Data::Bool(*value),
        ScalarOwned::Integer(value) => Data::Int(*value),
        ScalarOwned::FloatingPoint(value) => Data::Float(value.into_inner()),
        ScalarOwned::String(value) => Data::String(value.clone()),
    })
}

fn line_number(node: &MarkedYamlOwned) -> u32 {
    // saphyr 的行号从 1 开始。
    u32::try_from(node.span.start.line()).unwrap_or(1)
}
