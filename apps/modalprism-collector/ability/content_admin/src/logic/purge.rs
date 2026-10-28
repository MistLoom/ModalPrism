//! 方法逻辑：purge（物理删除）

use std::sync::Arc;

use assembly::domain::Rid;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    source: Option<String>,
    before: Option<String>,
    after: Option<String>,
    /// 选中行的 rid；给了就不再按筛选走。
    rids: Option<Vec<String>>,
    /// 仅用于审计的删除原因，删除后不再保留记录。
    #[allow(dead_code)]
    reason: Option<String>,
    dry_run: Option<bool>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let dry_run = args.dry_run.unwrap_or(true);
    let rids = parse_rids(args.rids.unwrap_or_default())?;
    if !rids.is_empty() {
        if dry_run {
            return Ok(serde_json::json!({
                "matched": rids.len(),
                "deleted": rids.len(),
                "truncated": false,
                "dryRun": true,
            }));
        }
        let deleted = services
            .content_delete_rids(&rids)
            .await
            .map_err(crate::failure)?;
        return Ok(serde_json::json!({
            "matched": rids.len(),
            "deleted": deleted,
            "truncated": false,
            "dryRun": false,
        }));
    }
    let filter = crate::filter(args.source, args.before, args.after)?;
    crate::map(services.content_delete(&filter, dry_run).await)
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
