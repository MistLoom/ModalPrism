//! `strata_read`、`health.status` 与 `config_inspect`。

use serde_json::{Value, json};

use crate::error::Error;
use crate::root::ReadRoot;

pub(crate) fn arguments(args: Value) -> Result<Value, Error> {
    match args {
        Value::Null => Ok(Value::Object(Default::default())),
        Value::Object(_) => Ok(args),
        _ => Err(Error::new("spec 无效")),
    }
}

pub(crate) fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, Error> {
    serde_json::from_value(value.clone()).map_err(|_| Error::new("spec 无效"))
}

fn field<'a>(args: &'a Value, key: &str) -> Result<&'a Value, Error> {
    args.get(key).ok_or_else(|| Error::new("spec 无效"))
}

pub(crate) fn text_field(args: &Value, key: &str) -> Result<String, Error> {
    field(args, key)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::new("spec 无效"))
}

pub(crate) fn empty_args(args: &Value) -> Result<(), Error> {
    if args.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err(Error::new("spec 无效"))
    }
}

pub(crate) async fn kinds(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        include_variants: bool,
    }
    let args: Args = parse(args)?;
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let kinds = root.strata().kinds(&rid, args.include_variants).await?;
    serde_json::to_value(kinds).map_err(|source| Error::with_source("spec 无效", source))
}

pub(crate) async fn has(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        kind: strata_model::FacetKind,
        #[serde(default)]
        variant: Option<String>,
    }
    let args: Args = parse(args)?;
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let found = root
        .strata()
        .has(&rid, &args.kind, args.variant.as_deref())
        .await?;
    Ok(Value::Bool(found))
}

pub(crate) async fn get(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        facet: String,
        #[serde(default = "lazy_default")]
        lazy: bool,
    }
    fn lazy_default() -> bool {
        true
    }
    let args: Args = parse(args)?;
    let reference =
        strata_model::FacetRef::parse(&args.facet).map_err(|_| Error::new("spec 无效"))?;
    let Some(facet) = root.strata().get(&reference, args.lazy).await? else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "version": facet.version,
        "contentHash": facet.content_hash,
        "mediaType": facet.media_type,
        "size": facet.size,
        "text": facet.text,
        "struct": facet.value,
        "part": facet.blob,
    }))
}

pub(crate) async fn slice(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        span: String,
    }
    let args: Args = parse(args)?;
    let span = strata_model::SpanRef::parse(&args.span).map_err(|_| Error::new("spec 无效"))?;
    let Some(facet) = root.strata().get(&span.facet, true).await? else {
        return Err(Error::new("gateway"));
    };
    if facet.text.is_none() {
        return Err(Error::new("gateway"));
    }
    let bytes = root.strata().slice(&span).await?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| Error::new("gateway"))?;
    Ok(Value::String(text))
}

pub(crate) async fn resolve(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    let identity = text_field(args, "identity")?;
    let rid = root.strata().resolve(&identity).await?;
    Ok(match rid {
        Some(rid) => Value::String(rid.to_string()),
        None => Value::Null,
    })
}

pub(crate) async fn history(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    let facet = text_field(args, "facet")?;
    let reference = strata_model::FacetRef::parse(&facet).map_err(|_| Error::new("spec 无效"))?;
    let versions = root.strata().history(&reference).await?;
    let items: Vec<Value> = versions
        .into_iter()
        .map(|item| {
            json!({
                "version": item.version,
                "contentHash": item.content_hash,
                "createdAt": item.created_at,
                "seq": item.seq,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

pub(crate) async fn lineage(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        facet: String,
        #[serde(default)]
        direction: Option<String>,
        #[serde(default = "one")]
        depth: u32,
    }
    fn one() -> u32 {
        1
    }
    let args: Args = parse(args)?;
    let direction = match args.direction.as_deref() {
        None | Some("up") => strata_model::LineageDir::Up,
        Some("down") => strata_model::LineageDir::Down,
        Some(_) => return Err(Error::new("spec 无效")),
    };
    let reference =
        strata_model::FacetRef::parse(&args.facet).map_err(|_| Error::new("spec 无效"))?;
    let graph = root
        .strata()
        .lineage(&reference, direction, args.depth)
        .await?;
    Ok(json!({
        "nodes": graph.nodes.iter().map(|node| json!({
            "facet": node.facet.to_string(),
            "module": node.module,
            "version": node.version,
            "paramsHash": node.params_hash,
        })).collect::<Vec<_>>(),
        "edges": graph.edges.iter().map(|edge| json!({
            "from": edge.from.to_string(),
            "to": edge.to.to_string(),
        })).collect::<Vec<_>>(),
    }))
}

pub(crate) async fn related(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        rel: Option<String>,
        #[serde(default = "out_dir")]
        direction: String,
    }
    fn out_dir() -> String {
        "out".into()
    }
    let args: Args = parse(args)?;
    let direction = match args.direction.as_str() {
        "out" => strata_model::EdgeDir::Out,
        "in" => strata_model::EdgeDir::In,
        _ => return Err(Error::new("spec 无效")),
    };
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let rows = root
        .strata()
        .related(&rid, args.rel.as_deref(), direction)
        .await?;
    let items = rows
        .into_iter()
        .map(|row| {
            json!({
                "src": row.src.to_string(),
                "rel": row.rel,
                "dst": row.dst.to_string(),
                "attrs": row.attrs,
                "commitId": row.commit_id,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

pub(crate) async fn query(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        query: strata_model::Query,
    }
    let args: Args = parse(args)?;
    let page = root.strata().query(args.query).await?;
    Ok(json!({
        "rows": page.rows,
        "nextCursor": page.next_cursor,
    }))
}

pub(crate) async fn changes(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        after: u64,
        #[serde(default)]
        limit: Option<u32>,
    }
    let args: Args = parse(args)?;
    let rows = root.strata().changes(args.after, args.limit).await?;
    let items = rows
        .into_iter()
        .map(|row| {
            json!({
                "snapshot": row.snapshot,
                "commitIds": row.commit_ids,
                "publishedAt": row.published_at,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

pub(crate) async fn health_status(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    let key = format!("source/{source}");
    let state = root.strata().state("health", &key).await?;
    let (status, alert) = match state {
        Some(state) => (
            state
                .value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string(),
            state
                .value
                .get("alert")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        None => ("ok".to_string(), false),
    };
    Ok(json!({
        "key": key,
        "status": status,
        "alert": alert,
    }))
}

pub(crate) async fn config_validate(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    config::validate(root.config_dir(), root.load_options())?;
    Ok(json!({ "ok": true }))
}

pub(crate) async fn config_diff(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    let fresh = config::validate(root.config_dir(), root.load_options())?;
    let changes = config::diff(&root.config().snapshot(), &fresh);
    let items = changes
        .into_iter()
        .map(|change| {
            json!({
                "path": change.path,
                "before": change.before,
                "after": change.after,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

pub(crate) async fn config_explain(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    let module = text_field(args, "module")?;
    let fields = config::explain(&root.config().snapshot(), &module)?;
    let items = fields
        .into_iter()
        .map(|field| {
            json!({
                "path": field.path,
                "value": field.value,
                "file": field.file,
                "line": field.line,
                "presetId": field.preset_id,
                "viaPreset": field.via_preset,
                "systemScope": field.system_scope,
            })
        })
        .collect();
    Ok(Value::Array(items))
}
