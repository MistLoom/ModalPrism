//! `strata_read`、`health.status` 与 `config_inspect`。

use serde_json::{Value, json};

use crate::ctx::ReadCtx;
use crate::error::Error;

pub fn arguments(args: Value) -> Result<Value, Error> {
    match args {
        Value::Null => Ok(Value::Object(Default::default())),
        Value::Object(_) => Ok(args),
        _ => Err(Error::new("spec 无效")),
    }
}

pub fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, Error> {
    serde_json::from_value(value.clone()).map_err(|_| Error::new("spec 无效"))
}

fn field<'a>(args: &'a Value, key: &str) -> Result<&'a Value, Error> {
    args.get(key).ok_or_else(|| Error::new("spec 无效"))
}

pub fn text_field(args: &Value, key: &str) -> Result<String, Error> {
    field(args, key)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::new("spec 无效"))
}

pub fn empty_args(args: &Value) -> Result<(), Error> {
    if args.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err(Error::new("spec 无效"))
    }
}

pub async fn kinds(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        rid: String,
        #[serde(default)]
        include_variants: bool,
    }
    let args: Args = parse(args)?;
    let rid = strata_model::Rid::parse(&args.rid).map_err(|_| Error::new("spec 无效"))?;
    let kinds = ctx.strata().kinds(&rid, args.include_variants).await?;
    serde_json::to_value(kinds).map_err(|source| Error::with_source("spec 无效", source))
}

pub async fn has(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
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
    let found = ctx
        .strata()
        .has(&rid, &args.kind, args.variant.as_deref())
        .await?;
    Ok(Value::Bool(found))
}

pub async fn get(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
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
    let Some(facet) = ctx.strata().get(&reference, args.lazy).await? else {
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

pub async fn slice(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        span: String,
    }
    let args: Args = parse(args)?;
    let span = strata_model::SpanRef::parse(&args.span).map_err(|_| Error::new("spec 无效"))?;
    let Some(facet) = ctx.strata().get(&span.facet, true).await? else {
        return Err(Error::new("gateway"));
    };
    if facet.text.is_none() {
        return Err(Error::new("gateway"));
    }
    let bytes = ctx.strata().slice(&span).await?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| Error::new("gateway"))?;
    Ok(Value::String(text))
}

pub async fn resolve(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    let identity = text_field(args, "identity")?;
    let rid = ctx.strata().resolve(&identity).await?;
    Ok(match rid {
        Some(rid) => Value::String(rid.to_string()),
        None => Value::Null,
    })
}

pub async fn history(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    let facet = text_field(args, "facet")?;
    let reference = strata_model::FacetRef::parse(&facet).map_err(|_| Error::new("spec 无效"))?;
    let versions = ctx.strata().history(&reference).await?;
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

pub async fn lineage(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
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
    let graph = ctx
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

pub async fn related(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
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
    let rows = ctx
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

pub async fn query(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        query: strata_model::Query,
    }
    let args: Args = parse(args)?;
    let page = ctx.strata().query(args.query).await?;
    Ok(json!({
        "rows": page.rows,
        "nextCursor": page.next_cursor,
    }))
}

pub async fn changes(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        after: u64,
        #[serde(default)]
        limit: Option<u32>,
    }
    let args: Args = parse(args)?;
    let rows = ctx.strata().changes(args.after, args.limit).await?;
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

pub async fn health_status(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    let source = text_field(args, "source")?;
    let key = format!("source/{source}");
    let state = ctx.strata().state("health", &key).await?;
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

/// 通道与解析器的运行态。
///
/// 健康信号已按主体存成三条前缀 key，这里一次取全量后按前缀分组，
/// 控制台不必逐信源查一次。
pub async fn health_report(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    let mut sources = Vec::new();
    let mut accessors = Vec::new();
    let mut adapters = Vec::new();
    for (key, state) in ctx.strata().states("health").await? {
        let Some((scope, id)) = key.split_once('/') else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let item = health_item(id, &state.value);
        match scope {
            "source" => sources.push(item),
            "accessor" => accessors.push(item),
            "adapter" => adapters.push(item),
            _ => {}
        }
    }
    Ok(json!({
        "sources": sources,
        "accessors": accessors,
        "adapters": adapters,
    }))
}

fn health_item(id: &str, value: &Value) -> Value {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let count = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    json!({
        "id": id,
        "status": text("status").unwrap_or_else(|| "ok".into()),
        "alert": value.get("alert").and_then(Value::as_bool).unwrap_or(false),
        "lastError": text("last_error"),
        "lastOkAt": text("last_ok_at"),
        "blockedHits": count("blocked_hits"),
        "selectorMiss": count("selector_miss"),
    })
}

/// 内容库总览，服务端聚合，不再靠采样。
pub async fn summary(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    let summary = ctx.strata().summary().await?;
    serde_json::to_value(summary).map_err(|source| Error::with_source("spec 无效", source))
}

pub async fn config_validate(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    config::validate(ctx.config_dir(), ctx.load_options())?;
    Ok(json!({ "ok": true }))
}

pub async fn config_diff(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    let fresh = config::validate(ctx.config_dir(), ctx.load_options())?;
    let changes = config::diff(&ctx.config().snapshot(), &fresh);
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

pub async fn config_modules(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    Ok(config::modules(&ctx.config().snapshot()))
}

pub async fn config_explain(ctx: &dyn ReadCtx, args: &Value) -> Result<Value, Error> {
    let module = text_field(args, "module")?;
    let fields = config::explain(&ctx.config().snapshot(), &module)?;
    let items = fields
        .into_iter()
        .map(|field| {
            json!({
                "path": field.path,
                "domain": config::domain_of_path(&field.path),
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

/// 只读方法的唯一分发表。
///
/// 两个进程（采集与只读）都走这里，因此同一数据目录下的输出逐字段一致。
pub async fn call(ctx: &dyn ReadCtx, method: &str, args: Value) -> Result<Value, Error> {
    let args = arguments(args)?;
    match method {
        "strata_read.kinds" => kinds(ctx, &args).await,
        "strata_read.has" => has(ctx, &args).await,
        "strata_read.get" => get(ctx, &args).await,
        "strata_read.slice" => slice(ctx, &args).await,
        "strata_read.resolve" => resolve(ctx, &args).await,
        "strata_read.history" => history(ctx, &args).await,
        "strata_read.lineage" => lineage(ctx, &args).await,
        "strata_read.related" => related(ctx, &args).await,
        "strata_read.query" => query(ctx, &args).await,
        "strata_read.changes" => changes(ctx, &args).await,
        "strata_read.summary" => summary(ctx, &args).await,
        "health.status" => health_status(ctx, &args).await,
        "health.report" => health_report(ctx, &args).await,
        "config_inspect.validate" => config_validate(ctx, &args).await,
        "config_inspect.diff" => config_diff(ctx, &args).await,
        "config_inspect.explain" => config_explain(ctx, &args).await,
        "config_inspect.modules" => config_modules(ctx, &args).await,
        "type_article.project" => crate::article::project_call(ctx, &args).await,
        "type_article.on_commit" => crate::article::on_commit_call(&args),
        "type_hotlist.project" => crate::hotlist::project_call(ctx, &args).await,
        "type_hotlist.on_commit" => crate::hotlist::on_commit_call(ctx, &args).await,
        _ => Err(Error::new("spec 无效")),
    }
}
