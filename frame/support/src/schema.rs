//! Manifest `payload_schema` / `input_schema` 子集校验。
//!
//! 覆盖声明中常用的 JSON Schema 片段：`type`、`properties`、`required`、`enum`、
//! 空对象属性（不约束）。完整 JSON Schema 不在此实现范围。

use serde_json::Value;

/// 校验失败详情。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaError {
    pub path: String,
    pub message: String,
}

impl SchemaError {
    fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// schema 为空或 null 时表示不约束载荷。
pub fn is_unrestricted_schema(schema: &Value) -> bool {
    is_unrestricted(schema)
}

/// 按 Manifest 中的 schema 校验 JSON 值。schema 为空或 null 时跳过。
pub fn validate_schema(value: &Value, schema: &Value) -> Result<(), SchemaError> {
    if is_unrestricted(schema) {
        return Ok(());
    }
    validate_node(value, schema, "")
}

fn is_unrestricted(schema: &Value) -> bool {
    schema.is_null() || (schema.is_object() && schema.as_object().is_some_and(|o| o.is_empty()))
}

fn validate_node(value: &Value, schema: &Value, path: &str) -> Result<(), SchemaError> {
    if is_unrestricted(schema) {
        return Ok(());
    }

    if let Some(enums) = schema.get("enum").and_then(Value::as_array) {
        if !enums.iter().any(|e| e == value) {
            return Err(SchemaError::at(
                path,
                format!("值不在 enum 允许范围: {value}"),
            ));
        }
    }

    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if !matches_type(value, ty) {
            return Err(SchemaError::at(
                path,
                format!("期望类型 {ty}，实际为 {}", json_type_name(value)),
            ));
        }
        match ty {
            "object" => validate_object(value, schema, path)?,
            "array" => validate_array(value, schema, path)?,
            _ => {}
        }
    }

    Ok(())
}

fn validate_object(value: &Value, schema: &Value, path: &str) -> Result<(), SchemaError> {
    let obj = value
        .as_object()
        .ok_or_else(|| SchemaError::at(path, "期望 object"))?;
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required {
            let Some(key) = key.as_str() else {
                continue;
            };
            if !obj.contains_key(key) {
                let field_path = join_path(path, key);
                return Err(SchemaError::at(field_path, "缺少必填字段"));
            }
        }
    }

    for (key, prop_schema) in &props {
        if let Some(v) = obj.get(key) {
            let field_path = join_path(path, key);
            validate_node(v, prop_schema, &field_path)?;
        }
    }
    Ok(())
}

fn validate_array(value: &Value, schema: &Value, path: &str) -> Result<(), SchemaError> {
    let arr = value
        .as_array()
        .ok_or_else(|| SchemaError::at(path, "期望 array"))?;
    if let Some(item_schema) = schema.get("items") {
        for (i, item) in arr.iter().enumerate() {
            let item_path = if path.is_empty() {
                format!("[{i}]")
            } else {
                format!("{path}[{i}]")
            };
            validate_node(item, item_schema, &item_path)?;
        }
    }
    Ok(())
}

fn matches_type(value: &Value, ty: &str) -> bool {
    match ty {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || (value.is_f64() && value.as_f64().unwrap().fract() == 0.0)
        }
        "number" => value.is_number(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        _ => true,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn join_path(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_string()
    } else {
        format!("{base}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_required_and_enum() {
        let schema = json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "role": { "type": "string", "enum": ["user", "assistant", "tool"] }
            },
            "required": ["id", "role"]
        });
        validate_schema(&json!({"id": "s1", "role": "tool"}), &schema).unwrap();
        let err = validate_schema(&json!({"id": "s1", "role": "system"}), &schema).unwrap_err();
        assert!(err.message.contains("enum"));
    }

    #[test]
    fn skips_empty_schema() {
        validate_schema(&json!({"any": 1}), &json!({})).unwrap();
        validate_schema(&json!({"any": 1}), &Value::Null).unwrap();
    }
}
