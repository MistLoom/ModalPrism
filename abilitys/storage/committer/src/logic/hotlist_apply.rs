use ability_support::{AbilityCtx, MethodId, Result};
use serde_json::{Value, json};

pub async fn run(ctx: &dyn AbilityCtx) -> Result<serde_json::Value> {
    let on_commit = MethodId::new("on_commit").expect("method id");
    let value = ctx
        .call("type_hotlist", &on_commit, json!({}))
        .await?;
    if value.is_null() {
        return Ok(json!({ "applied": false }));
    }
    let apply = MethodId::new("apply").expect("method id");
    ctx.call(
        "strata_write",
        &apply,
        json!({ "changeset": value }),
    )
    .await?;
    let flush = MethodId::new("flush").expect("method id");
    ctx.call("committer", &flush, json!({})).await?;
    Ok(json!({ "applied": true }))
}

pub async fn hotlist_apply(ctx: &dyn AbilityCtx, args: serde_json::Value) -> Result<Value> {
    let _ = args;
    run(ctx).await
}
