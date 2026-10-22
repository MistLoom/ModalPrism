//! 字段默认值合并：请求优先，禁止用 defaults 覆盖已出现字段。

use serde_json::{Map, Value};

/// 将 `defaults` 补入 `request`：仅填充 request 中缺失的键。
///
/// - 对象：递归合并；request 已有键（含显式 `null`）保留不动。
/// - 非对象：若 request 非 `Null` 则整段保留，否则用 defaults。
pub fn merge_defaults(request: Value, defaults: Option<&Value>) -> Value {
    let Some(defaults) = defaults else {
        return request;
    };
    match (request, defaults) {
        (Value::Object(mut req), Value::Object(def)) => {
            for (k, dv) in def {
                match req.get(k) {
                    None => {
                        req.insert(k.clone(), dv.clone());
                    }
                    Some(Value::Object(_)) if dv.is_object() => {
                        let existing = req.remove(k).unwrap_or(Value::Object(Map::new()));
                        req.insert(k.clone(), merge_defaults(existing, Some(dv)));
                    }
                    Some(_) => {}
                }
            }
            Value::Object(req)
        }
        (Value::Null, d) => d.clone(),
        (req, _) => req,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_wins_over_defaults() {
        let req = json!({"a": 1, "b": null});
        let def = json!({"a": 9, "b": 2, "c": 3});
        assert_eq!(
            merge_defaults(req, Some(&def)),
            json!({"a": 1, "b": null, "c": 3})
        );
    }

    #[test]
    fn nested_merge_preserves_request() {
        let req = json!({"x": {"y": 1}});
        let def = json!({"x": {"y": 9, "z": 2}, "w": 0});
        assert_eq!(
            merge_defaults(req, Some(&def)),
            json!({"x": {"y": 1, "z": 2}, "w": 0})
        );
    }
}
