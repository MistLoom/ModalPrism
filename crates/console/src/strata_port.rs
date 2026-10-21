//! 经 `strata_read` Gateway 访问存储。

use serde_json::{Value, json};
use strata_model::{AsOf, FacetKind, FacetRef, Query as StrataQuery, Rid, VersionSel};

use crate::error::{ConsoleError, ConsoleResult};
use crate::invoke;
use crate::ConsoleContext;

pub struct FacetView {
    pub value: Option<Value>,
}

pub async fn query(
    ctx: &ConsoleContext,
    query: StrataQuery,
) -> ConsoleResult<(Vec<Value>, Option<String>)> {
    let page = invoke::call(
        &ctx.gateway,
        "strata_read.query",
        json!({ "query": query }),
    )
    .await
    .map_err(|e| ConsoleError::internal(e))?;
    let rows = page
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let next = page
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((rows, next))
}

pub async fn facet_get_at(
    ctx: &ConsoleContext,
    rid: &Rid,
    kind: FacetKind,
    variant: Option<&str>,
    as_of: Option<AsOf>,
) -> ConsoleResult<Option<FacetView>> {
    let variant_owned = variant.map(str::to_string);
    let version = match as_of {
        None => VersionSel::Latest,
        Some(AsOf::Snapshot(seq)) => {
            let latest = FacetRef::new(
                rid.clone(),
                kind.clone(),
                variant_owned.clone(),
                VersionSel::Latest,
            )
            .map_err(|_| ConsoleError::bad_request("记录 ID 无效"))?;
            let history = invoke::call(
                &ctx.gateway,
                "strata_read.history",
                json!({ "facet": latest.to_string() }),
            )
            .await
            .map_err(|e| ConsoleError::internal(e))?;
            let picked = history
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item.get("seq").and_then(Value::as_u64).unwrap_or(0) <= seq)
                .max_by_key(|item| item.get("seq").and_then(Value::as_u64).unwrap_or(0));
            picked
                .and_then(|item| item.get("version").and_then(Value::as_u64))
                .map(VersionSel::Exact)
                .unwrap_or(VersionSel::Latest)
        }
        Some(_) => VersionSel::Latest,
    };
    let reference = FacetRef::new(rid.clone(), kind.clone(), variant_owned, version)
        .map_err(|_| ConsoleError::bad_request("记录 ID 无效"))?;
    let data = invoke::call(
        &ctx.gateway,
        "strata_read.get",
        json!({ "facet": reference.to_string(), "lazy": true }),
    )
    .await
    .map_err(|e| ConsoleError::internal(e))?;
    if data.is_null() {
        return Ok(None);
    }
    Ok(Some(FacetView {
        value: data.get("struct").cloned(),
    }))
}
