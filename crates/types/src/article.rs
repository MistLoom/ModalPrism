//! 文章投影。`on_commit` 不产出变更集。

use serde_json::{Value, json};
use strata_model::{FacetKind, FacetRef, FacetSelector, Query, QueryFrom, Rid, VersionSel};

use crate::error::Error;
use crate::read::{empty_args, text_field};
use crate::root::ReadRoot;

pub struct Article;

impl Article {
    pub const NAME: &'static str = "Article";
    pub const VERSION: &'static str = "1";

    pub fn schema() -> Value {
        serde_json::from_str(include_str!("../../../config/schemas/article.json"))
            .expect("article schema")
    }

    pub fn selector() -> Query {
        facet_query("article")
    }

    pub async fn project(root: &ReadRoot, rid: &Rid) -> Result<Value, Error> {
        let Some(structure) = facet(root, rid, FacetKind::Struct, Some("article")).await? else {
            return Ok(Value::Null);
        };
        let readable = match facet(root, rid, FacetKind::Md, Some("readable")).await? {
            Some(facet) => facet.text.map(Value::String).unwrap_or(Value::Null),
            None => Value::Null,
        };
        let cover = match facet(root, rid, FacetKind::Image, Some("cover")).await? {
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

pub(crate) async fn project_call(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    let rid = text_field(args, "rid")?;
    if args.as_object().is_some_and(|object| object.len() != 1) {
        return Err(Error::new("spec 无效"));
    }
    let rid = Rid::parse(&rid).map_err(|_| Error::new("spec 无效"))?;
    Article::project(root, &rid).await
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
    root: &ReadRoot,
    rid: &Rid,
    kind: FacetKind,
    variant: Option<&str>,
) -> Result<Option<strata_client::Facet>, Error> {
    let reference = FacetRef::new(
        rid.clone(),
        kind,
        variant.map(str::to_string),
        VersionSel::Latest,
    )
    .map_err(|source| Error::with_source(source.to_string(), source))?;
    Ok(root.strata().get(&reference, true).await?)
}
