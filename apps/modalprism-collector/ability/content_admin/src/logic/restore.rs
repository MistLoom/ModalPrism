//! 方法逻辑：restore

use std::sync::Arc;

use assembly::domain::Rid;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    rids: Vec<String>,
    reason: Option<String>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let reason = crate::non_empty(args.reason).unwrap_or_else(|| "撤销删除".into());
    let rids = parse_rids(args.rids)?;
    if rids.is_empty() {
        return crate::invalid();
    }
    let restored = services
        .content_restore(&rids, &reason)
        .await
        .map_err(crate::failure)?;
    Ok(serde_json::json!({ "restored": restored }))
}

fn parse_rids(values: Vec<String>) -> ability_support::Result<Vec<Rid>> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let Ok(rid) = Rid::parse(&value) else {
            return crate::invalid();
        };
        out.push(rid);
    }
    Ok(out)
}
