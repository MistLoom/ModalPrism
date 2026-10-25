//! 方法逻辑：label —— 用户分类的打标与摘标，墓碑之外的第二种内容治理动作。

use std::collections::BTreeMap;
use std::sync::Arc;

use assembly::domain::Rid;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    rids: Vec<String>,
    #[serde(default)]
    set: BTreeMap<String, String>,
    #[serde(default)]
    unset: Vec<String>,
}

pub async fn run(
    services: Arc<assembly::Services>,
    args: serde_json::Value,
) -> ability_support::Result<serde_json::Value> {
    let args: Args = crate::parse(args)?;
    let mut rids = Vec::with_capacity(args.rids.len());
    for value in args.rids {
        let Ok(rid) = Rid::parse(&value) else {
            return crate::invalid();
        };
        rids.push(rid);
    }
    if rids.is_empty() {
        return crate::invalid();
    }
    let labeled = services
        .content_label(&rids, &args.set, &args.unset)
        .await
        .map_err(crate::failure)?;
    Ok(serde_json::json!({ "labeled": labeled }))
}
