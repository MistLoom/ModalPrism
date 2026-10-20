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
    let spec = module
        .pointer("/access/spec")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let bytes = serde_json::to_vec(module).ok()?;
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
