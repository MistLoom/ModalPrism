//! 文章投影。`on_commit` 不产出变更集。

use serde_json::{Value, json};
use strata_model::{FacetKind, FacetRef, FacetSelector, Query, QueryFrom, Rid, VersionSel};

use crate::ctx::ReadCtx;
use crate::error::Error;
use crate::read::{empty_args, text_field};

pub struct Article;

impl Article {
    pub const NAME: &'static str = "Article";
    pub const VERSION: &'static str = "1";

    pub fn schema() -> Value {
        serde_json::from_str(include_str!("../../../../config/schemas/article.json"))
            .expect("article schema")
    }

    pub fn selector() -> Query {
        facet_query("article")
    }

    pub async fn project(ctx: &dyn ReadCtx, rid: &Rid) -> Result<Value, Error> {
        let strata = ctx.strata();
        let Some(structure) = facet(strata, rid, FacetKind::Struct, Some("article"), true).await?
        else {
            return Ok(Value::Null);
        };
        let readable = match facet(strata, rid, FacetKind::Md, Some("readable"), false).await? {
            Some(facet) => readable_text(&facet),
            None => Value::Null,
        };
        let cover = match facet(strata, rid, FacetKind::Image, Some("cover"), true).await? {
            Some(facet) => match facet.blob {
                Some(part) => json!({ "sha256": part.sha256(), "size": part.size() }),
                None => Value::Null,
            },
            None => Value::Null,
        };
        Ok(json!({
            "rid": rid.to_string(),
            "struct": structure.value,
            "readable": readable,
            "cover": cover,
        }))
    }
}

pub(crate) async fn project_call(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    let rid = text_field(args, "rid")?;
    if args.as_object().is_some_and(|object| object.len() != 1) {
        return Err(Error::new("spec 无效"));
    }
    let rid = Rid::parse(&rid).map_err(|_| Error::new("spec 无效"))?;
    Article::project(ctx, &rid).await
}

pub(crate) fn on_commit_call(args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    Ok(Value::Null)
}

pub(crate) fn facet_query(variant: &str) -> Query {
    Query {
        from: QueryFrom::Resources,
        r#where: None,
        has_facets: vec![FacetSelector {
            kind: FacetKind::Struct,
            variant: Some(variant.to_string()),
            fields: Vec::new(),
        }],
        select: Vec::new(),
        expand: Vec::new(),
        search: None,
        as_of: None,
        order_by: Vec::new(),
        limit: None,
        cursor: None,
    }
}

pub(crate) async fn facet(
    strata: &strata_client::Strata,
    rid: &Rid,
    kind: FacetKind,
    variant: Option<&str>,
    lazy: bool,
) -> Result<Option<strata_client::Facet>, Error> {
    let reference = FacetRef::new(
        rid.clone(),
        kind,
        variant.map(str::to_string),
        VersionSel::Latest,
    )
    .map_err(|source| Error::with_source(source.to_string(), source))?;
    Ok(strata.get(&reference, lazy).await?)
}

fn readable_text(facet: &strata_client::Facet) -> Value {
    if let Some(text) = &facet.text {
        return Value::String(text.clone());
    }
    match facet
        .bytes
        .as_ref()
        .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok())
    {
        Some(text) => Value::String(text),
        None => Value::Null,
    }
}
