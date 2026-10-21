use ability_support::{AbilityCtx, Error, MethodId, Result};
use serde_json::{Value, json};
use strata_model::Rid;

pub async fn project(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let rid = args
        .get("rid")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid_args("spec 无效"))?;
    if args.as_object().is_some_and(|object| object.len() != 1) {
        return Err(Error::invalid_args("spec 无效"));
    }
    let rid = Rid::parse(rid).map_err(|_| Error::invalid_args("spec 无效"))?;
    project_via_ctx(ctx, &rid).await
}

async fn project_via_ctx(ctx: &dyn AbilityCtx, rid: &Rid) -> Result<Value> {
    let article_ref = format!("{}@struct:article#latest", rid);
    let article = ctx
        .call(
            "strata_read",
            &MethodId::new("get").unwrap(),
            json!({ "facet": article_ref, "lazy": true }),
        )
        .await?;
    if article.is_null() {
        return Ok(Value::Null);
    }
    let readable_ref = format!("{}@md:readable#latest", rid);
    let readable = ctx
        .call(
            "strata_read",
            &MethodId::new("get").unwrap(),
            json!({ "facet": readable_ref, "lazy": true }),
        )
        .await
        .ok();
    let cover_ref = format!("{}@image:cover#latest", rid);
    let cover = ctx
        .call(
            "strata_read",
            &MethodId::new("get").unwrap(),
            json!({ "facet": cover_ref, "lazy": true }),
        )
        .await
        .ok();
    Ok(json!({
        "rid": rid.to_string(),
        "struct": article.get("struct").cloned().unwrap_or(Value::Null),
        "readable": readable
            .and_then(|v| v.get("text").cloned())
            .unwrap_or(Value::Null),
        "cover": cover.and_then(|v| {
            v.get("part").map(|part| json!({
                "sha256": part.get("sha256"),
                "size": part.get("size"),
            }))
        }).unwrap_or(Value::Null),
    }))
}
