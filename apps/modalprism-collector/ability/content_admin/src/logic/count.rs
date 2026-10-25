//! 方法逻辑：count

use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    source: Option<String>,
    before: Option<String>,
    after: Option<String>,
    include_tombstoned: Option<bool>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let filter = crate::filter(args.source, args.before, args.after)?;
    let include_tombstoned = args.include_tombstoned.unwrap_or(false);
    let counts = services
        .content_count(&filter)
        .await
        .map_err(crate::failure)?;
    let matched = if include_tombstoned {
        counts.matched
    } else {
        counts.matched.saturating_sub(counts.tombstoned)
    };
    Ok(serde_json::json!({ "matched": matched, "tombstoned": counts.tombstoned }))
}
