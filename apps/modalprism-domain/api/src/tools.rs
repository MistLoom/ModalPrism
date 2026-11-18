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

/// 入口面标记：CLI（`x-modalprism-surface: cli`）与进程同主体，允许 `--file`
/// 走统一摄入函数；MCP / 其它 HTTP 调用方一律不接受本地文件路径参数。
const CLI_SURFACE: &str = "cli";

async fn run_tool(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    headers: axum::http::HeaderMap,
    AxPath(verb): AxPath<String>,
    body: Option<axum::Json<Value>>,
) -> Result<Response, ApiError> {
    let args = body.map(|axum::Json(value)| value).unwrap_or(Value::Null);
    let surface = headers
        .get("x-modalprism-surface")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let payload = call(&state, project.id(), &verb, args, surface == CLI_SURFACE).await?;
    Ok(util::ok(&payload))
}

/// 动词分发。每个分支都转调既有实现，不在这里复制业务逻辑。
pub async fn call(
    state: &Shared,
    project_id: &str,
    verb: &str,
    args: Value,
    allow_file: bool,
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
        "once" => once(app, &args, allow_file).await,
        "probe" => probe(app, &args, allow_file).await,
        "save" => save(app, &project_id, &args).await,
        "fetch" => crate::surface::fetch_value(state, args).await,
        // scope=content：检索已采集内容（含向量开启后的语义检索），与网页检索同一动词区分。
        "search" if args.get("scope").and_then(Value::as_str) == Some("content") => {
            crate::content::search_items_value(app, &project_id, &args).await
        }
        "search" => crate::search::search_value(state, args).await,
        "parse" => crate::surface::parse_value(state, args).await,
        "read" => read(app, &project_id, &args).await,
        "ls" => ls(app, &args).await,
        "plan.ls" => plan_ls(app, &args).await,
        "plan.get" => {
            let map = util::object(&args)?;
            let plan_id = util::text(map, "planId")?.trim().to_string();
            crate::collection::plan_get_value(state, &project_id, &plan_id).await
        }
        "plan.new" => crate::collection::create_plan_value(app, &project_id, &args).await,
        "plan.set" => {
            let map = util::object(&args)?;
            let plan_id = util::text(map, "planId")?.trim().to_string();
            let etag = args.get("etag").and_then(Value::as_str);
            // planId 与 etag 是工具面的定位/乐观锁参数，不属于计划文档字段；
            // 进复用逻辑前剥掉，否则会被未知字段校验拒绝。
            let mut body = args.clone();
            if let Some(object) = body.as_object_mut() {
                object.remove("planId");
                object.remove("etag");
            }
            crate::collection::update_plan_value(app, &project_id, &plan_id, etag, &body).await
        }
        "plan.pause" | "plan.resume" => {
            let map = util::object(&args)?;
            let plan_id = util::text(map, "planId")?.trim().to_string();
            let etag = args.get("etag").and_then(Value::as_str);
            crate::collection::pause_resume_plan_value(
                app,
                &project_id,
                &plan_id,
                etag,
                // 切片 2：paused 独立于 enabled，pause=true / resume=false。
                verb == "plan.pause",
            )
            .await
        }
        "plan.run" => {
            // 计划级一轮（planId）与单信源手动触发（sourceId，旧契约）二选一：
            // 带 planId 时立即跑一轮不重置游标；旧客户端的 sourceId 路径不变。
            let map = util::object(&args)?;
            if let Some(plan_id) = util::optional_text(map, "planId")?.filter(|id| !id.is_empty())
            {
                return crate::collection::plan_run_value(state, &project_id, plan_id.trim())
                    .await;
            }
            crate::collection::create_run_value(state, &args).await
        }
        "strategy.ls" => crate::collection::strategy_rows(app).map(|rows| json!(rows)),
        "strategy.get" => {
            let map = util::object(&args)?;
            let strategy_id = util::text(map, "strategyId")?.trim().to_string();
            crate::collection::strategy_row_owned(app, &strategy_id)
        }
        "strategy.new" => {
            crate::collection::create_strategy_value(app, &project_id, &args).await
        }
        "strategy.set" => {
            let map = util::object(&args)?;
            let strategy_id = util::text(map, "strategyId")?.trim().to_string();
            let mut body = args.clone();
            // strategyId 是工具面的定位参数，不属于策略文档字段，进复用逻辑前剥掉。
            if let Some(object) = body.as_object_mut() {
                object.remove("strategyId");
            }
            crate::collection::update_strategy_value(app, &strategy_id, &body).await
        }
        "strategy.remove" => {
            let map = util::object(&args)?;
            let strategy_id = util::text(map, "strategyId")?.trim().to_string();
            crate::collection::delete_strategy_value(app, &strategy_id).await
        }
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

async fn once(state: &AppState, args: &Value, allow_file: bool) -> Result<Value, ApiError> {
    // M-06：`once --from-search <searchId>` —— 从已持久化的检索记录取 URL，
    // 逐条走 once 的统一摄入路径（缺省 preview 不落盘）。MCP/HTTP 与 CLI 同语义。
    if let Some(search_id) = args
        .get("fromSearch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return once_from_search(state, args, search_id, allow_file).await;
    }
    let request = assembly::tools::OnceRequest::parse_for_surface(&state.services, args, allow_file)
        .await
        .map_err(tool_error)?;
    assembly::tools::once(&state.services, request)
        .await
        .map_err(tool_error)
}

/// 从检索记录取 URL 逐条执行 once：`index` 缺省按序全部（串行），指定则只跑一条。
async fn once_from_search(
    state: &AppState,
    args: &Value,
    search_id: &str,
    allow_file: bool,
) -> Result<Value, ApiError> {
    let record = crate::search::load_search_record(&state.data_dir, search_id)
        .map_err(ApiError::not_found)?;
    let urls: Vec<String> = record
        .pointer("/result/results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|hit| hit.get("url").and_then(Value::as_str).map(str::to_string))
        .collect();
    if urls.is_empty() {
        return Err(ApiError::not_found(format!(
            "检索记录 {search_id} 里没有可用 URL"
        )));
    }
    let index = args.get("index").and_then(Value::as_u64).map(|value| value as usize);
    let targets: Vec<(usize, &String)> = urls
        .iter()
        .enumerate()
        .filter(|(position, _)| index.is_none_or(|index| *position == index))
        .collect();
    let mut ran = Vec::new();
    let mut request = args.clone();
    if let Some(object) = request.as_object_mut() {
        object.remove("fromSearch");
        object.remove("index");
    }
    for (position, url) in targets {
        let mut once_args = request.clone();
        once_args["url"] = json!(url);
        let parsed = assembly::tools::OnceRequest::parse_for_surface(
            &state.services,
            &once_args,
            allow_file,
        )
        .await
        .map_err(tool_error)?;
        let outcome = assembly::tools::once(&state.services, parsed)
            .await
            .map_err(tool_error)?;
        ran.push(json!({"index": position, "url": url, "once": outcome}));
    }
    Ok(json!({"fromSearch": search_id, "count": ran.len(), "runs": ran}))
}

async fn probe(state: &AppState, args: &Value, allow_file: bool) -> Result<Value, ApiError> {
    assembly::tools::probe(&state.services, args, allow_file)
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
    // 管道缺省用智能发现：这一份声明由 config::builtin_adapters 兜底，引用不会悬空。
    // 传了但不是数组要直接拒：静默丢掉调用方的管道会让它以为按自己写的跑了。
    let pipeline = match map.get("pipeline") {
        None | Some(Value::Null) => json!([{ "run": ["adapter/smart-discover"] }]),
        Some(value) if value.is_array() => value.clone(),
        Some(_) => {
            return Err(ApiError::validation(
                "pipeline",
                "type",
                "管道必须是数组；留空则由内置智能适配器处理",
            ));
        }
    };
    let mut doc = json!({
        "name": name,
        "access": { "accessor": accessor, "locator": { "url": url } },
        "pipeline": pipeline,
        "project": project_id,
        "shared": false,
    });
    // 节律只由计划表达：信源自带 schedule 会被合成成一条落不了盘的计划，
    // 计划页看不见它，采集却按它跑。调用方没写就不给缺省；传了但不是对象
    // 要直接拒，理由与 pipeline 同一套：静默丢掉会让调用方以为按自己写的跑了。
    if let Some(schedule) = map.get("schedule").filter(|value| !value.is_null()) {
        if !schedule.is_object() {
            return Err(ApiError::validation(
                "schedule",
                "type",
                "节律请写成对象（如 {\"interval\": \"2h\"}），或留空交由采集计划表达",
            ));
        }
        doc["schedule"] = schedule.clone();
    }
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

/// 发现：信源清单与可挂载组件目录。`kind` 过滤组件类型，`tag` 只过滤目录、不参与调度。
async fn ls(state: &AppState, args: &Value) -> Result<Value, ApiError> {
    let kind = args
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tag = args
        .get("tag")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let sources = crate::catalog::source_rows(state).await?;
    // 可挂载组件：内置访问器与适配器（声明在 config 模块表里，带 labels/tags）。
    let snapshot = config::validate(&state.config_dir, &crate::catalog::options())?;
    let modules: Vec<Value> = snapshot
        .modules
        .iter()
        .filter(|(_, module)| {
            let module_kind = module.get("kind").and_then(Value::as_str).unwrap_or("");
            matches!(module_kind, "accessor" | "adapter" | "type")
        })
        .filter(|(_, module)| match kind {
            Some(kind) => module.get("kind").and_then(Value::as_str) == Some(kind),
            None => true,
        })
        .filter(|(_, module)| match tag {
            Some(tag) => module
                .get("labels")
                .and_then(Value::as_object)
                .map(|labels| {
                    labels
                        .values()
                        .filter_map(Value::as_str)
                        .any(|value| value == tag)
                        || labels.contains_key(tag)
                })
                .unwrap_or(false),
            None => true,
        })
        .map(|(id, module)| {
            json!({
                "id": id,
                "kind": module.get("kind"),
                "name": module.get("name"),
                "description": module.get("description"),
                "labels": module.get("labels"),
            })
        })
        .collect();
    Ok(json!({
        "sources": sources,
        "components": modules,
    }))
}

/// 计划清单：含 pipeline、vectorize 与运行态摘要；可按状态过滤。
async fn plan_ls(state: &AppState, args: &Value) -> Result<Value, ApiError> {
    let status = args
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut plans = crate::collection::plan_rows(state).await?;
    if let Some(status) = status {
        plans.retain(|plan| plan["status"].as_str() == Some(status));
    }
    Ok(json!({ "items": plans, "total": plans.len() }))
}

/// 计划详情：pipeline、vectorize 与进度（近期运行）一并返回。
async fn plan_get(state: &AppState, project_id: &str, args: &Value) -> Result<Value, ApiError> {
    let map = util::object(args)?;
    let plan_id = util::text(map, "planId")?.trim().to_string();
    let mut plan = crate::collection::plan_row_owned(state, &plan_id, project_id).await?;
    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "recentRuns".into(),
            Value::Array(crate::collection::runs_for_plan(state, &plan_id, 5).await?),
        );
    }
    Ok(plan)
}

/// 工具面错误：输入类错误保留字段语义，其余按依赖失败返回。
pub(crate) fn tool_error(error: assembly::Error) -> ApiError {
    if assembly::invalid_input(&error.message) {
        ApiError::validation("args", "invalid", error.message)
    } else {
        ApiError::dependency(error.to_string())
    }
}
