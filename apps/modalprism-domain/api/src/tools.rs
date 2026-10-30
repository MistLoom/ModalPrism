//! 工具面：`GET /tools`（自描述目录）与 `POST /tools/{verb}`（单一入口）。
//!
//! 动词表在 `surface::TOOLS`，这里只做两件事：把参数原样交给既有实现，把结果统一成
//! 同一份 JSON 形状。CLI、MCP 与 HTTP 因此共享一套语义，不再各写一份动词。

use std::path::Path;

use axum::Router;
use axum::extract::{Path as AxPath, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/tools", get(list_tools))
        .route("/tools/{verb}", post(run_tool))
}

/// 工具目录：动词语义、参数与读写标记都来自 `surface::TOOLS`。
pub fn catalog() -> Value {
    json!({
        "items": surface::TOOLS
            .iter()
            .map(|spec| json!({
                "verb": spec.id,
                "summary": spec.summary,
                "write": spec.write,
                // 三面（CLI / MCP / HTTP）拿到的目录都带可读标记，调用方不必自己判断。
                "writeLabel": if spec.write { "会写入" } else { "只读取" },
                "params": spec
                    .params
                    .iter()
                    .map(|param| json!({
                        "name": param.name,
                        "type": param.kind,
                        "required": param.required,
                        "summary": param.summary,
                    }))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
        "total": surface::TOOLS.len(),
    })
}

async fn list_tools() -> Response {
    util::ok(&catalog())
}

async fn run_tool(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    AxPath(verb): AxPath<String>,
    body: Option<axum::Json<Value>>,
) -> Result<Response, ApiError> {
    let args = body.map(|axum::Json(value)| value).unwrap_or(Value::Null);
    let payload = call(&state, project.id(), &verb, args).await?;
    Ok(util::ok(&payload))
}

/// 动词分发。每个分支都转调既有实现，不在这里复制业务逻辑。
pub async fn call(
    state: &Shared,
    project_id: &str,
    verb: &str,
    args: Value,
) -> Result<Value, ApiError> {
    let app: &AppState = state.as_ref();
    let args = if args.is_null() { json!({}) } else { args };
    // 项目上下文：显式 projectId 优先，其次取请求头带来的活跃项目。
    let project_id = args
        .get("projectId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(project_id)
        .to_string();
    match verb {
        "catalog" => Ok(catalog()),
        "once" => once(app, &args).await,
        "probe" => probe(app, &args).await,
        "save" => save(app, &project_id, &args).await,
        "fetch" => crate::surface::fetch_value(state, args).await,
        "search" => crate::search::search_value(state, args).await,
        "parse" => crate::surface::parse_value(state, args).await,
        "read" => read(app, &project_id, &args).await,
        other => Err(ApiError::validation(
            "verb",
            "invalid",
            format!(
                "未知工具动词: {other}（可用: {}）",
                surface::tool_ids().join(" / ")
            ),
        )),
    }
}

async fn once(state: &AppState, args: &Value) -> Result<Value, ApiError> {
    let request = assembly::tools::OnceRequest::parse(&state.services, args)
        .await
        .map_err(tool_error)?;
    assembly::tools::once(&state.services, request)
        .await
        .map_err(tool_error)
}

async fn probe(state: &AppState, args: &Value) -> Result<Value, ApiError> {
    assembly::tools::probe(&state.services, args)
        .await
        .map_err(tool_error)
}

/// 把探测结论固化为信源声明：与信源新建走同一套校验与写路径。
async fn save(state: &AppState, project_id: &str, args: &Value) -> Result<Value, ApiError> {
    let map = util::object(args)?;
    let name = util::text(map, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::validation("name", "required", "信源名称不能为空"));
    }
    let url = util::text(map, "url")?.trim().to_string();
    if url.is_empty() {
        return Err(ApiError::validation("url", "required", "信源地址不能为空"));
    }
    let accessor = map
        .get("accessor")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("accessor/http")
        .to_string();
    let accessor = if accessor.starts_with("accessor/") {
        accessor
    } else {
        format!("accessor/{accessor}")
    };
    let pipeline = map
        .get("pipeline")
        .cloned()
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([{ "run": ["adapter/smart-discover"] }]));
    let mut doc = json!({
        "name": name,
        "access": { "accessor": accessor, "locator": { "url": url } },
        "pipeline": pipeline,
        "schedule": map
            .get("schedule")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({ "interval": "2h" })),
        "project": project_id,
        "shared": false,
    });
    if let Some(source_type) = map.get("sourceType").and_then(Value::as_str) {
        doc["sourceType"] = json!(source_type);
    }
    let base = util::slug(&name);
    let mut slug = base.clone();
    for index in 2..20 {
        let candidate = Path::new(&state.config_dir)
            .join("sources")
            .join(format!("{slug}.yaml"));
        if !candidate.exists() {
            break;
        }
        slug = format!("{base}-{index}");
    }
    let saved = config::create(
        &state.config_dir,
        &crate::catalog::options(),
        &config::Draft {
            kind: "source".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "tool.save",
        "succeeded",
        "工具固化信源",
        json!({"id": saved.id, "name": name}),
        None,
    )
    .await?;
    Ok(json!({
        "saved": true,
        "id": saved.id,
        "file": saved.file,
        "slug": slug,
        "projectId": project_id,
    }))
}

/// 内容读取：给 facet 就取单面，否则取详情（含数据面清单与默认正文）。
async fn read(state: &AppState, project_id: &str, args: &Value) -> Result<Value, ApiError> {
    let map = util::object(args)?;
    let content_id = util::text(map, "contentId")?.trim().to_string();
    if content_id.is_empty() {
        return Err(ApiError::validation(
            "contentId",
            "required",
            "内容标识不能为空",
        ));
    }
    match map
        .get("facet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(kind) => {
            let variant = map
                .get("variant")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let version = map.get("version").and_then(Value::as_u64);
            crate::content::facet_value(state, &content_id, kind, variant, project_id, version)
                .await
        }
        None => crate::content::detail_value(state, &content_id, project_id).await,
    }
}

/// 工具面错误：输入类错误保留字段语义，其余按依赖失败返回。
fn tool_error(error: assembly::Error) -> ApiError {
    if assembly::invalid_input(&error.message) {
        ApiError::validation("args", "invalid", error.message)
    } else {
        ApiError::dependency(error.to_string())
    }
}
