//! 面向控制台的业务文案（非内部术语）。

use serde_json::Value;

pub fn health_label(status: &str, alert: bool) -> String {
    match status {
        "ok" if !alert => "正常".into(),
        "ok" => "正常（有提醒）".into(),
        "degraded" => "下降".into(),
        "failed" => "异常".into(),
        _ => status.into(),
    }
}

pub fn schedule_note(
    enabled: bool,
    skip_reason: Option<&str>,
    budget_used: Option<u64>,
    next_at: Option<&Value>,
) -> Option<String> {
    if !enabled {
        return Some("信源已停用，启用后才会调度采集。".into());
    }
    match skip_reason {
        Some("budget") => {
            let used = budget_used.unwrap_or(0);
            Some(format!(
                "本周期付费请求额度已用尽（已用 {used} 次），窗口滚动后会自动恢复。"
            ))
        }
        Some("robots") => Some("该站点 robots 规则不允许抓取，已跳过。".into()),
        Some(other) => Some(format!("当前周期跳过采集：{other}")),
        None => next_at
            .and_then(Value::as_str)
            .map(|t| format!("下次计划采集：{t}")),
    }
}

pub fn circuit_label(state: &str) -> String {
    match state {
        "open" => "已熔断".into(),
        "half-open" => "试探恢复".into(),
        "closed" => "正常".into(),
        _ => state.into(),
    }
}

pub fn change_summary(change: &Value) -> String {
    let snapshot = change.get("snapshot").and_then(Value::as_u64);
    let at = change.get("publishedAt").and_then(Value::as_str);
    match (snapshot, at) {
        (Some(seq), Some(time)) => format!("第 {seq} 次可见更新（{time}）"),
        (Some(seq), None) => format!("第 {seq} 次可见更新"),
        (_, Some(time)) => format!("数据更新于 {time}"),
        _ => "数据已更新".into(),
    }
}
