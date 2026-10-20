//! 管道匹配与依赖分层。

use serde_json::{Map, Value};
use strata_model::{PartRole, TaskReason};

use crate::error::AdapterError;
use crate::model::Detection;

#[derive(Clone, Debug)]
pub struct When {
    pub part_role: Option<String>,
    pub declared: Option<Vec<String>>,
    pub detected: Option<Vec<String>>,
    pub traits: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub when: When,
    pub run: Vec<String>,
    pub probe: Vec<String>,
    pub follow_only: bool,
    pub after: Option<String>,
    pub spec: Value,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub part_index: usize,
    pub adapter_id: String,
    pub spec: Value,
    pub probe: bool,
    pub after: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub part_index: usize,
    pub adapter_id: String,
    pub spec: Value,
    pub layer: usize,
    pub after: Option<String>,
}

pub struct PartMatch<'a> {
    pub index: usize,
    pub role: &'a PartRole,
    pub declared: &'a str,
    pub detection: Option<&'a Detection>,
}

pub fn parse_pipeline(value: &Value) -> Result<Vec<Rule>, AdapterError> {
    let Some(items) = value.as_array() else {
        return Err(AdapterError::config("pipeline 必须是数组"));
    };
    items.iter().map(parse_rule).collect()
}

pub fn needs_detection(parts: &[PartMatch<'_>], rules: &[Rule], reason: TaskReason) -> bool {
    rules.iter().any(|rule| {
        (rule.when.detected.is_some() || rule.when.traits.is_some())
            && parts.iter().any(|part| base_match(rule, part, reason))
    })
}

pub fn candidates(
    parts: &[PartMatch<'_>],
    rules: &[Rule],
    reason: TaskReason,
    specs: &serde_json::Map<String, Value>,
) -> Result<Vec<Candidate>, AdapterError> {
    let mut out = Vec::new();
    for part in parts {
        for rule in rules {
            if !full_match(rule, part, reason) {
                continue;
            }
            push_ids(part, rule, &rule.run, false, specs, &mut out)?;
            push_ids(part, rule, &rule.probe, true, specs, &mut out)?;
        }
    }
    Ok(out)
}

pub fn schedule(mut nodes: Vec<Candidate>) -> Result<Vec<Step>, AdapterError> {
    loop {
        let present: Vec<(usize, String)> = nodes
            .iter()
            .map(|node| (node.part_index, node.adapter_id.clone()))
            .collect();
        let before = nodes.len();
        nodes.retain(|node| {
            node.after.as_ref().is_none_or(|after| {
                present
                    .iter()
                    .any(|(part, id)| *part == node.part_index && id == after)
            })
        });
        if nodes.len() == before {
            break;
        }
    }
    let mut layers = Vec::with_capacity(nodes.len());
    let mut pending: Vec<usize> = (0..nodes.len()).collect();
    let mut assigned = vec![None; nodes.len()];
    let mut guard = 0;
    while !pending.is_empty() {
        guard += 1;
        if guard > nodes.len() + 1 {
            return Err(AdapterError::config("依赖成环"));
        }
        let mut ready = Vec::new();
        for index in &pending {
            let after_layer = match &nodes[*index].after {
                None => Some(0),
                Some(after) => nodes
                    .iter()
                    .position(|node| {
                        node.part_index == nodes[*index].part_index && node.adapter_id == *after
                    })
                    .and_then(|pred| assigned[pred].map(|layer| layer + 1)),
            };
            if let Some(layer) = after_layer {
                ready.push((*index, layer));
            }
        }
        if ready.is_empty() {
            return Err(AdapterError::config("依赖成环"));
        }
        ready
            .sort_by_key(|(index, _)| (nodes[*index].part_index, nodes[*index].adapter_id.clone()));
        for (index, layer) in ready {
            assigned[index] = Some(layer);
            pending.retain(|item| *item != index);
            layers.push(Step {
                part_index: nodes[index].part_index,
                adapter_id: nodes[index].adapter_id.clone(),
                spec: nodes[index].spec.clone(),
                layer,
                after: nodes[index].after.clone(),
            });
        }
    }
    layers.sort_by(|left, right| {
        left.layer
            .cmp(&right.layer)
            .then(left.part_index.cmp(&right.part_index))
            .then(left.adapter_id.cmp(&right.adapter_id))
    });
    Ok(layers)
}

fn push_ids(
    part: &PartMatch<'_>,
    rule: &Rule,
    ids: &[String],
    probe: bool,
    specs: &Map<String, Value>,
    out: &mut Vec<Candidate>,
) -> Result<(), AdapterError> {
    for id in ids {
        if id == "adapter/detect" {
            continue;
        }
        if out
            .iter()
            .any(|item| item.part_index == part.index && item.adapter_id == *id)
        {
            continue;
        }
        out.push(Candidate {
            part_index: part.index,
            adapter_id: id.clone(),
            spec: merged_spec(specs, &rule.spec, id)?,
            probe,
            after: rule.after.clone(),
        });
    }
    Ok(())
}

fn merged_spec(
    specs: &Map<String, Value>,
    rule_spec: &Value,
    id: &str,
) -> Result<Value, AdapterError> {
    if !rule_spec.is_null() && !rule_spec.is_object() {
        return Err(AdapterError::config("spec 必须是对象"));
    }
    let base = specs
        .get(id)
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let Some(overlay) = rule_spec.get(id) else {
        return Ok(base);
    };
    if !overlay.is_object() {
        return Err(AdapterError::config(format!("{id} 的 spec 必须是对象")));
    }
    Ok(merge_value(&base, overlay))
}

fn merge_value(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(left), Value::Object(right)) => {
            let mut out = left.clone();
            for (key, value) in right {
                let merged = match out.get(key) {
                    Some(previous) => merge_value(previous, value),
                    None => value.clone(),
                };
                out.insert(key.clone(), merged);
            }
            Value::Object(out)
        }
        _ => overlay.clone(),
    }
}

fn base_match(rule: &Rule, part: &PartMatch<'_>, reason: TaskReason) -> bool {
    reason_ok(rule.follow_only, reason)
        && role_ok(&rule.when, part)
        && declared_ok(&rule.when, part)
}

fn full_match(rule: &Rule, part: &PartMatch<'_>, reason: TaskReason) -> bool {
    base_match(rule, part, reason) && detected_ok(&rule.when, part) && traits_ok(&rule.when, part)
}

fn reason_ok(follow_only: bool, reason: TaskReason) -> bool {
    if follow_only {
        reason == TaskReason::Follow
    } else {
        reason != TaskReason::Follow
    }
}

fn role_ok(when: &When, part: &PartMatch<'_>) -> bool {
    when.part_role
        .as_ref()
        .is_none_or(|role| role == part.role.as_str())
}

fn declared_ok(when: &When, part: &PartMatch<'_>) -> bool {
    let Some(declared) = &when.declared else {
        return true;
    };
    let essence = crate::textutil::essence(part.declared);
    declared
        .iter()
        .any(|item| crate::textutil::essence(item) == essence)
}

fn detected_ok(when: &When, part: &PartMatch<'_>) -> bool {
    let Some(detected) = &when.detected else {
        return true;
    };
    let Some(detection) = part.detection else {
        return false;
    };
    let essence = crate::textutil::essence(&detection.media_type);
    detected
        .iter()
        .any(|item| crate::textutil::essence(item) == essence)
}

fn traits_ok(when: &When, part: &PartMatch<'_>) -> bool {
    let Some(wanted) = &when.traits else {
        return true;
    };
    let Some(detection) = part.detection else {
        return false;
    };
    wanted
        .iter()
        .all(|trait_name| detection.traits.iter().any(|item| item == trait_name))
}

fn parse_rule(value: &Value) -> Result<Rule, AdapterError> {
    let object = value
        .as_object()
        .ok_or_else(|| AdapterError::config("管道规则必须是对象"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "when" | "run" | "run_if_accepts" | "for" | "after" | "spec"
        ) {
            return Err(AdapterError::config(format!("未知管道字段: {key}")));
        }
    }
    let when = match object.get("when") {
        None => When {
            part_role: None,
            declared: None,
            detected: None,
            traits: None,
        },
        Some(when) => parse_when(when)?,
    };
    let follow_only = match object.get("for") {
        None => false,
        Some(Value::String(value)) if value == "follow" => true,
        Some(_) => return Err(AdapterError::config("for 只接受 follow")),
    };
    let after = match object.get("after") {
        None => None,
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        Some(_) => return Err(AdapterError::config("after 必须是适配器 id")),
    };
    let spec = object.get("spec").cloned().unwrap_or(Value::Null);
    Ok(Rule {
        when,
        run: string_list(object.get("run"))?,
        probe: string_list(object.get("run_if_accepts"))?,
        follow_only,
        after,
        spec,
    })
}

fn parse_when(value: &Value) -> Result<When, AdapterError> {
    let object = value
        .as_object()
        .ok_or_else(|| AdapterError::config("when 必须是对象"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "part_role" | "declared" | "detected" | "traits"
        ) {
            return Err(AdapterError::config(format!("未知 when 字段: {key}")));
        }
    }
    Ok(When {
        part_role: optional_string(object.get("part_role"))?,
        declared: optional_strings(object.get("declared"))?,
        detected: optional_strings(object.get("detected"))?,
        traits: optional_strings(object.get("traits"))?,
    })
}

fn optional_string(value: Option<&Value>) -> Result<Option<String>, AdapterError> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(_) => Err(AdapterError::config("字段必须是非空字符串")),
    }
}

fn optional_strings(value: Option<&Value>) -> Result<Option<Vec<String>>, AdapterError> {
    match value {
        None => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(AdapterError::config("列表项必须是字符串"));
                };
                out.push(text.to_string());
            }
            Ok(Some(out))
        }
        Some(_) => Err(AdapterError::config("字段必须是字符串列表")),
    }
}

fn string_list(value: Option<&Value>) -> Result<Vec<String>, AdapterError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| AdapterError::config("适配器 id 必须是非空字符串"))
            })
            .collect(),
        Some(_) => Err(AdapterError::config("适配器列表必须是数组")),
    }
}
