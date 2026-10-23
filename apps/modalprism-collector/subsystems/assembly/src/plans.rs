//! 从配置快照生成信源计划。推送信源不进入 tick。

use std::time::Duration;

use config::Snapshot;
use planner::SourcePlan;
use serde_json::Value;
use strata_model::{Locator, SourceId, content_hash};

pub const YIELD_DROP_RATIO: f64 = 0.3;
pub const FAILURE_THRESHOLD: u32 = 5;

pub fn plans_of(snapshot: &Snapshot) -> Vec<SourcePlan> {
    snapshot
        .modules
        .iter()
        .filter_map(|(id, module)| one_plan(snapshot, id, module))
        .collect()
}

fn one_plan(snapshot: &Snapshot, id: &str, module: &Value) -> Option<SourcePlan> {
    if module.get("kind").and_then(Value::as_str) != Some("source") {
        return None;
    }
    let locator = locator_of(module.get("access")?.get("locator")?)?;
    if matches!(locator, Locator::PushChannel { .. }) {
        return None;
    }
    let accessor = module
        .pointer("/access/accessor")
        .and_then(Value::as_str)?
        .to_string();
    let accessor_module = snapshot.modules.get(&accessor)?;
    let implementation = accessor_module
        .get("implementation")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !implementation.starts_with("builtin:") {
        return None;
    }
    let source_id = SourceId::new(id).ok()?;
    let schedule = module.get("schedule").cloned().unwrap_or(Value::Null);
    let floor = duration_at(schedule.get("floor"), Duration::from_secs(30 * 60))?;
    let interval = duration_at(schedule.get("interval"), floor)?;
    let ceiling = duration_at(schedule.get("ceiling"), interval)?;
    let timeout = duration_at(
        accessor_module.pointer("/limits/timeout"),
        Duration::from_secs(30),
    )?;
    let expected_period = match schedule.get("expectedPeriod") {
        None => None,
        Some(value) => Some(duration_at(Some(value), Duration::ZERO)?),
    };
    let spec = access_spec(accessor_module, module);
    let bytes = serde_json::to_vec(module).ok()?;
    let budget = budget_of(module);
    Some(SourcePlan {
        source_id,
        locator,
        accessor,
        queue: queue_of(implementation, module),
        spec,
        generation: snapshot.generation,
        config_hash: content_hash(&bytes),
        timeout,
        paid: module.get("paid").is_some(),
        budget_requests: budget.map(|(requests, _)| requests),
        budget_window: budget.map(|(_, window)| window).unwrap_or(Duration::ZERO),
        adaptive: schedule
            .get("adaptive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        interval,
        floor,
        ceiling,
        batch_key: schedule
            .get("batchKey")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        enabled: module
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        expected_period,
        yield_drop_ratio: YIELD_DROP_RATIO,
        failure_threshold: FAILURE_THRESHOLD,
    })
}

fn access_spec(accessor_module: &Value, source: &Value) -> Value {
    let mut base = match accessor_module.get("spec") {
        Some(Value::Object(map)) => map.clone(),
        _ => Default::default(),
    };
    if let Some(Value::Object(overlay)) = source.pointer("/access/spec") {
        for (key, value) in overlay {
            base.insert(key.clone(), value.clone());
        }
    }
    Value::Object(base)
}

fn locator_of(value: &Value) -> Option<Locator> {
    if let Some(url) = value.get("url").and_then(Value::as_str) {
        return Some(Locator::Url {
            url: url.to_string(),
        });
    }
    if let Some(query) = value.get("query").and_then(Value::as_str) {
        return Some(Locator::Query {
            query: query.to_string(),
        });
    }
    if let Some(endpoint) = value.get("endpoint").and_then(Value::as_str) {
        return Some(Locator::ApiEndpoint {
            endpoint: endpoint.to_string(),
        });
    }
    if let Some(channel) = value.get("channel").and_then(Value::as_str) {
        return Some(Locator::PushChannel {
            channel: channel.to_string(),
        });
    }
    None
}

fn budget_of(module: &Value) -> Option<(u64, Duration)> {
    let requests = module.pointer("/paid/budget/requests")?.as_u64()?;
    let window = parse_duration(module.pointer("/paid/budget/window")?.as_str()?)?;
    if requests == 0 || window.is_zero() {
        return None;
    }
    Some((requests, window))
}

fn queue_of(implementation: &str, module: &Value) -> strata_model::QueueName {
    if implementation == "builtin:browser" {
        strata_model::QueueName::Render
    } else if module.get("paid").is_some() {
        strata_model::QueueName::Paid
    } else {
        strata_model::QueueName::Free
    }
}

fn duration_at(value: Option<&Value>, default: Duration) -> Option<Duration> {
    let Some(value) = value else {
        return Some(default);
    };
    parse_duration(value.as_str()?)
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let unit = &text[split..];
    let scale = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(Duration::from_millis(count.saturating_mul(scale)))
}

pub fn capacities() -> planner::Capacities {
    planner::Capacities {
        free: 4,
        paid: 1,
        render: 2,
        push: 4,
    }
}

pub fn snapshot_interval(snapshot: &Snapshot) -> Duration {
    duration_at(
        snapshot.system.pointer("/policy/state/snapshot_interval"),
        Duration::from_secs(15 * 60),
    )
    .unwrap_or(Duration::from_secs(15 * 60))
}

pub fn flush_interval(snapshot: &Snapshot) -> Duration {
    duration_at(
        snapshot.system.pointer("/policy/commit/flush_interval"),
        Duration::from_secs(30),
    )
    .unwrap_or(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn paid_declaration_enters_the_paid_queue() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "accessor/paid-api".into(),
            json!({
                "kind": "accessor",
                "implementation": "builtin:paid-api",
                "limits": {"timeout": "60s"}
            }),
        );
        modules.insert(
            "source/status-search".into(),
            json!({
                "kind": "source",
                "enabled": false,
                "access": {
                    "accessor": "accessor/paid-api",
                    "locator": {"endpoint": "https://example.com/search"}
                },
                "paid": {"budget": {"requests": 48, "window": "1d"}},
                "schedule": {"interval": "30m"}
            }),
        );
        let snapshot = config::Snapshot {
            generation: 1,
            system: json!({}),
            modules,
            origins: BTreeMap::new(),
        };
        let plans = plans_of(&snapshot);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].queue, strata_model::QueueName::Paid);
        assert!(plans[0].paid);
        assert!(!plans[0].enabled);
        assert_eq!(plans[0].budget_requests, Some(48));
        assert_eq!(plans[0].budget_window, Duration::from_secs(86_400));
    }
}
