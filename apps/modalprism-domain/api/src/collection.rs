//! 采集计划与采集运行：自动与手动共用同一条任务创建路径。

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::TaskFilter;
use strata_model::QueueName;

use crate::error::ApiError;
use crate::operations::accepted;
use crate::util::{self, cursor, page_total};
use crate::{AppState, Shared};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/collection/plans", get(list_plans).post(create_plan))
        .route(
            "/collection/plans/{planId}",
            get(get_plan).patch(update_plan).delete(delete_plan),
        )
        .route("/collection/plans/{planId}/pause", post(pause_plan))
        .route("/collection/plans/{planId}/resume", post(resume_plan))
        .route("/collection/jobs", post(create_job))
        .route("/collection/runs", get(list_runs).post(create_run))
        .route("/collection/runs/{runId}", get(get_run))
        .route(
            "/collection/strategies",
            get(list_strategies).post(create_strategy),
        )
        .route(
            "/collection/strategies/{strategyId}",
            get(get_strategy)
                .patch(update_strategy)
                .delete(delete_strategy),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    offset: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    /// 采集计划：只看该计划派发的运行。
    #[serde(default)]
    plan_id: Option<String>,
    /// `all` 取消项目隔离过滤（管理与迁移用）。
    #[serde(default)]
    scope: Option<String>,
}

pub(crate) async fn plan_rows(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let list = config::list(&state.config_dir)?;
    let sources = crate::catalog::source_rows(state).await?;
    // 计划维度的投递目标存在内容投递域：按计划 id 回填，计划与投递只单向关联。
    let delivery = crate::delivery::plan_rule_views(state).await?;
    let source_status = |id: &str| -> Option<Value> {
        sources
            .iter()
            .find(|row| row["id"].as_str() == Some(id))
            .cloned()
    };
    let mut plans = Vec::new();
    // 运行中判定共用一次队列查询：有处于 running 状态任务的计划投影为 running。
    let filter = TaskFilter::default();
    let task_rows = state
        .services
        .queue_list(&filter, None, 200)
        .await
        .map_err(ApiError::from)?
        .rows;
    let running_plans: std::collections::BTreeSet<String> = task_rows
        .iter()
        .filter(|row| row.status == "leased")
        .filter_map(|row| {
            row.plan_id
                .as_deref()
                .map(|id| id.strip_prefix("plan/").unwrap_or(id).to_string())
        })
        .collect();
    for plan in list["plans"].as_array().cloned().unwrap_or_default() {
        let slug = plan["slug"].as_str().unwrap_or_default().to_string();
        // 单一语义：targets 与行为字段由所引策略承载，计划只携带引用与状态。
        let strategy_doc = plan["strategy"]
            .as_str()
            .map(|reference| reference.trim_start_matches("strategy/"))
            .and_then(|slug| {
                list["strategies"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|item| item["slug"].as_str() == Some(slug))
            })
            .unwrap_or(Value::Null);
        let targets: Vec<String> = strategy_doc["targets"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(util::source_slug))
            .collect();
        let enabled = plan["enabled"].as_bool().unwrap_or(true);
        // paused 与 enabled 是两个可持久化字段（切片 2）：enabled 表示是否投入调度，
        // paused 只在 enabled 为 true 时表示主动中断（保留游标与已落盘数据）。
        let paused = plan["paused"]
            .as_bool()
            .or_else(|| plan["doc"]["paused"].as_bool())
            .unwrap_or(false);
        // 计划归属：声明优先，其次按首个目标信源的归属派生，最后回落默认项目。
        let project = plan_project(&plan, &targets, &sources);
        // 运行态借首个目标信源的调度视图（节律按目标数复制派发，视图取其一即足）。
        let first = targets.first().and_then(|id| source_status(id));
        let exhausted = first
            .as_ref()
            .and_then(|row| row["skipReason"].as_str())
            .is_some_and(|reason| reason == "quota-paused" || reason == "quota-exhausted");
        let status = if !enabled {
            // enabled=false：草稿/停用，未投入调度；与主动中断的 paused 可区分。
            "disabled"
        } else if paused {
            "paused"
        } else if running_plans.contains(&slug) {
            "running"
        } else if exhausted {
            // 已有的 exhausted：计划累计额度耗尽。
            "exhausted"
        } else {
            "active"
        };
        let etag = util::etag(&[&slug, &util::hash_short(&plan["doc"].to_string())]);
        let rule = delivery.get(&slug).cloned().unwrap_or(Value::Null);
        let mut row = json!({
            "id": slug,
            "name": plan["name"],
            "description": plan["description"],
            "project": project,
            "targets": targets,
            "strategy": plan["strategy"],
            "trigger": strategy_doc["trigger"],
            "queue": strategy_doc["queue"],
            "priority": strategy_doc["priority"],
            "quota": strategy_doc["quota"],
            "stopPolicy": strategy_doc["stopPolicy"],
            // 管道与向量化声明：plan.ls / plan.get 的宣传面承诺透出。
            // 未声明 pipeline 时按缺省语义回空数组（沿用信源默认路径），vectorize 缺省 false。
            "pipeline": strategy_doc["pipeline"].as_array().cloned().unwrap_or_default(),
            "vectorize": strategy_doc["vectorize"].as_bool().unwrap_or(false),
            "enabled": enabled,
            "paused": paused,
            "status": status,
            "health": first.as_ref().map(|row| row["health"].clone()).unwrap_or(Value::Null),
            "nextRunAt": first.as_ref().map(|row| row["nextRunAt"].clone()).unwrap_or(Value::Null),
            "lastRunAt": first.as_ref().map(|row| row["lastRunAt"].clone()).unwrap_or(Value::Null),
            "todayYield": first.as_ref().map(|row| row["todayYield"].clone()).unwrap_or(Value::Null),
            "deliveryRuleId": rule["ruleId"].clone(),
            "deliveryTargets": rule["targets"].clone(),
            "deliveryEnabled": rule["enabled"].clone(),
            "deliveryLastRunAt": rule["lastRunAt"].clone(),
            "etag": etag,
        });
        crate::dictionary::decorate(&mut row, &[("queue", "task.queue")]);
        plans.push(row);
    }
    Ok(plans)
}

/// 计划归属：声明优先，其次按首个目标信源的归属派生，最后回落默认项目。
/// 计划自身不存 project 的旧数据也能落到正确的项目，不需要一次性迁移。
fn plan_project(plan: &Value, targets: &[String], sources: &[Value]) -> String {
    if let Some(project) = plan
        .get("project")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return project.to_string();
    }
    for target in targets {
        if let Some(row) = sources
            .iter()
            .find(|row| row["id"].as_str() == Some(target.as_str()))
            && let Some(project) = row["project"].as_str()
        {
            return project.to_string();
        }
    }
    crate::projects::DEFAULT_PROJECT_ID.to_string()
}

/// 计划级管道校验（切片 2 预留）：形状必须是 `[{ "run": [适配器id…] }]`，引用的
/// 适配器必须存在于当前配置清单；不兼容的管道在 new/set 时拒绝。缺省（null / 空数组）
/// 表示沿用信源默认路径；本切片不实现内容处理器或向量写入，只做声明校验与透传。
fn validate_pipeline(state: &AppState, value: Option<&Value>) -> Result<(), ApiError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let items = value
        .as_array()
        .ok_or_else(|| ApiError::validation("pipeline", "type", "pipeline 必须是数组"))?;
    if items.is_empty() {
        return Ok(());
    }
    let snapshot = config::validate(&state.config_dir, &crate::catalog::options())?;
    let known: std::collections::BTreeSet<&str> = snapshot
        .modules
        .iter()
        .filter(|(_, module)| module.get("kind").and_then(Value::as_str) == Some("adapter"))
        .map(|(id, _)| id.as_str())
        .collect();
    for item in items {
        let run = item.get("run").and_then(Value::as_array).ok_or_else(|| {
            ApiError::validation(
                "pipeline",
                "invalid",
                format!("管道步骤必须是 {{\"run\": [适配器…]}}: {item}"),
            )
        })?;
        for adapter in run {
            let Some(id) = adapter.as_str() else {
                return Err(ApiError::validation(
                    "pipeline",
                    "invalid",
                    "run 数组元素必须是适配器 id 字符串",
                ));
            };
            if !known.contains(id) {
                let available = known.iter().copied().collect::<Vec<_>>().join(" / ");
                return Err(ApiError::validation(
                    "pipeline",
                    "unknownAdapter",
                    format!("未知适配器: {id}（已声明: {available}）"),
                ));
            }
        }
    }
    Ok(())
}

/// 取属于当前项目的计划；他项目的计划一律按不存在处理，不泄露存在性。
pub(crate) async fn plan_row_owned(
    state: &AppState,
    plan_id: &str,
    project_id: &str,
) -> Result<Value, ApiError> {
    let plan = plan_rows(state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    if plan["project"].as_str() != Some(project_id) {
        return Err(ApiError::not_found(format!("采集计划 {plan_id}")));
    }
    Ok(plan)
}

/// 作业计划的行为基座：幂等确保一次性作业策略存在（有计划必有策略）。
/// 作业的一次性语义由计划覆写 trigger 承载，不构成第二套行为定义。
fn ensure_oneshot_audit_strategy(state: &AppState) -> Result<(), ApiError> {
    let marker = state.config_dir.join("strategies/oneshot-audit.yaml");
    if marker.exists() {
        return Ok(());
    }
    // 幂等：并发首次建作业时可能同时触达 create，落盘成功（或文件已存在）即视为就绪。
    match config::create(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "strategy".into(),
            slug: "oneshot-audit".into(),
            doc: json!({
                "name": "一次性采集作业",
                "description": "批量采集作业计划的行为基座：作业派发即时任务，仅留痕审计。",
                "trigger": {"kind": "oneShot", "fireAt": "immediate"},
            }),
        },
    ) {
        Ok(_) => {}
        // 并发/重复创建：文件已存在则视为就绪。
        Err(error) if marker.exists() => {}
        Err(error) => return Err(ApiError::from(error)),
    }
    state.services.config_reload()?;
    Ok(())
}

/// 计划只能引用归属当前项目的信源：共享信源只放开读，不参与他项目的调度。
async fn ensure_targets_owned(
    state: &AppState,
    project_id: &str,
    targets: &[String],
) -> Result<(), ApiError> {
    let rows = crate::catalog::source_rows(state).await?;
    for target in targets {
        let id = util::source_slug(target);
        let row = rows
            .iter()
            .find(|row| row["id"].as_str() == Some(id.as_str()))
            .ok_or_else(|| ApiError::validation("targets", "notFound", format!("信源 {id} 不存在")))?;
        if !crate::catalog::source_editable(row, project_id) {
            return Err(ApiError::validation(
                "targets",
                "forbidden",
                format!("信源 {id} 不属于当前项目"),
            ));
        }
    }
    Ok(())
}

/// 当前项目自有（非共享）的信源集合；`scope=all` 时返回 None（不过滤）。
/// 运行记录属于采集产物，只能跟自有信源走：共享信源只读声明，不带运行数据。
async fn visible_sources_opt(
    state: &AppState,
    project: &crate::ActiveProject,
    scope: Option<&str>,
) -> Result<Option<std::collections::HashSet<String>>, ApiError> {
    if scope == Some("all") {
        Ok(None)
    } else {
        Ok(Some(
            crate::catalog::owned_source_ids(state, project.id()).await?,
        ))
    }
}

async fn list_plans(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "plans:{}:{}",
        query.status.clone().unwrap_or_default(),
        project.id()
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let mut plans = plan_rows(&state).await?;
    // 项目隔离：计划归属按声明的 project（或目标信源归属）判定；scope=all 取消过滤。
    if query.scope.as_deref() != Some("all") {
        let active = project.id();
        plans.retain(|plan| plan["project"].as_str() == Some(active));
    }
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        plans.retain(|plan| plan["status"] == status);
    }
    let total = plans.len();
    let items: Vec<Value> = plans.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

async fn get_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
) -> Result<Response, ApiError> {
    let plan = plan_row_owned(&state, &plan_id, project.id()).await?;
    let mut response = util::ok(&plan);
    if let Ok(value) = plan["etag"].as_str().unwrap_or("").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    Ok(response)
}

/// 由目标信源与请求内容派生一个合法 plan slug（slug 只允许字母数字与 `._-`）。
fn derive_plan_slug(body: &serde_json::Map<String, Value>, targets: &[String]) -> String {
    if let Some(id) = body.get("id").and_then(Value::as_str) {
        return id.trim_start_matches("plan/").to_string();
    }
    let first = targets
        .first()
        .map(|id| util::source_slug(id))
        .unwrap_or_else(|| "plan".to_string());
    format!(
        "{first}-{}",
        util::hash_short(&Value::Object(body.clone()).to_string())
    )
}

fn extract_targets(map: &serde_json::Map<String, Value>) -> Vec<String> {
    let key = if map.contains_key("targets") {
        "targets"
    } else {
        "sourceIds"
    };
    map.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .map(|id| util::source_ref(&id))
        .collect()
}

async fn create_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let plan = create_plan_value(&state, project.id(), &body).await?;
    Ok(util::ok(&plan))
}

/// M-05：storage 声明校验。root 需配合 classify: true——独立库路由（自有
/// StoragePaths）未实现，当前只有可读文件树物化；拒绝「设了根却以为数据在根下」的误配。
fn validate_storage(storage: Option<&Value>) -> Result<(), ApiError> {
    let Some(storage) = storage else {
        return Ok(());
    };
    let root = storage
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if root.is_empty() {
        return Ok(());
    }
    let classify = storage
        .get("classify")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !classify {
        return Err(ApiError::validation(
            "storage",
            "invalid",
            "storage.root 当前需配合 storage.classify: true 使用（独立库路由未实现，只落可读文件树）",
        ));
    }
    Ok(())
}

/// 可复用的新建计划逻辑：HTTP `POST /collection/plans` 与工具面 `plan.new` 共用，
/// 返回计划行值（信封由调用方包）。
///
/// 单一语义：计划是策略在项目里的**有状态实例**——只携带引用与状态
/// （enabled/paused/一次性 URL 数据），行为与目标全部由所引策略声明。
pub(crate) async fn create_plan_value(
    state: &AppState,
    project_id: &str,
    body: &Value,
) -> Result<Value, ApiError> {
    let map = util::object(body)?;
    util::unknown(map, &["id", "name", "description", "strategy", "enabled", "urls", "fromSearch"])?;
    // 单一语义：有计划必有策略。节律/额度等行为字段的定义点只有策略一处。
    let strategy = match map
        .get("strategy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(slug) => slug.to_string(),
        None => {
            return Err(ApiError::validation(
                "strategy",
                "required",
                "计划必须引用策略（有计划必有策略）：请先创建策略或传入 strategy",
            ))
        }
    };
    // URL 目标（切片 2）：`mp plan new --from-search` 的产物——一次性计划可以只有
    // URL 目标，运行时逐条走 once 的统一摄入路径；不引入浏览器自动化。
    let urls = match map.get("urls") {
        Some(value) if value.is_array() => {
            let urls: Vec<String> = value
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect();
            for url in &urls {
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err(ApiError::validation(
                        "urls",
                        "invalid",
                        format!("URL 目标仅支持 http/https: {url}"),
                    ));
                }
            }
            urls
        }
        Some(_) => {
            return Err(ApiError::validation(
                "urls",
                "invalid",
                "urls 必须是字符串数组",
            ))
        }
        None => Vec::new(),
    };
    // M-06：`plan.new --from-search <searchId>` —— 从持久化检索记录取 URL 生成
    // oneShot 计划（与 CLI 同语义）；记录缺失给出引导性 404。
    let from_search_urls = match map.get("fromSearch").and_then(Value::as_str) {
        Some(search_id) if !search_id.trim().is_empty() => {
            let record = crate::search::load_search_record(&state.data_dir, search_id.trim())
                .map_err(ApiError::not_found)?;
            record
                .pointer("/result/results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|hit| hit.get("url").and_then(Value::as_str).map(str::to_string))
                .collect::<Vec<String>>()
        }
        _ => Vec::new(),
    };
    let urls: Vec<String> = urls.into_iter().chain(from_search_urls).collect();
    // 归属与存在性校验：策略必须存在；recurring 策略在同一项目内至多实例化一个计划
    // （重复实例化会重复采集）。oneshot-audit 作业实例不受此限（每次运行各留痕一个）。
    let list = config::list(&state.config_dir)?;
    let strategy_doc = list["strategies"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|item| {
            item["slug"].as_str() == Some(strategy.as_str())
                || item["id"].as_str() == Some(format!("strategy/{}", strategy).as_str())
        })
        .ok_or_else(|| {
            ApiError::validation("strategy", "notFound", format!("策略 {strategy} 不存在"))
        })?;
    let recurring = strategy_doc["trigger"]["kind"].as_str() != Some("oneShot");
    // 单一语义：recurring 策略必须有目标信源，否则计划会静默产出 0 条 SourcePlan。
    if recurring
        && strategy_doc
            .get("targets")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0)
            == 0
    {
        return Err(ApiError::validation(
            "strategy",
            "invalid",
            format!("recurring 策略 {strategy} 缺少 targets，请先在策略上配置目标信源"),
        ));
    }
    if recurring {
        for plan in list["plans"].as_array().cloned().unwrap_or_default() {
            let same_strategy = plan["strategy"].as_str()
                .map(|value| {
                    value.trim_start_matches("strategy/") == strategy.as_str()
                })
                .unwrap_or(false);
            if same_strategy && plan["project"].as_str() == Some(project_id) {
                return Err(ApiError::validation(
                    "strategy",
                    "conflict",
                    format!("策略 {strategy} 已在本项目实例化为计划 {}", plan["id"]),
                ));
            }
        }
    }
    let slug = derive_plan_slug(map, &[]);
    let mut doc = json!({
        "id": format!("plan/{slug}"),
        "kind": "plan",
        "name": map.get("name").cloned().unwrap_or(json!("采集计划")),
        "project": project_id,
        // 引用形态统一：无论入参是裸 slug 还是带前缀引用，落盘一律 `strategy/<slug>`。
        "strategy": format!("strategy/{}", strategy.trim_start_matches("strategy/")),
    });
    if let Some(object) = doc.as_object_mut() {
        for key in ["description", "enabled"] {
            if let Some(value) = map.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
        if !urls.is_empty() {
            object.insert("urls".to_string(), json!(urls));
        }
    }
    config::create(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    // 投递目标随策略声明走：计划实例化时按所引策略的投递配置同步规则。
    crate::delivery::sync_plan_rule(
        state,
        &slug,
        map.get("name").and_then(Value::as_str).unwrap_or("采集计划"),
        strategy_doc.get("deliveryTargets"),
        strategy_doc.get("deliveryEnabled").and_then(Value::as_bool),
    )
    .await?;
    crate::observe::append_audit(
        state,
        "collection.plan.create",
        "succeeded",
        "新建采集计划",
        json!({"planId": slug}),
        None,
    )
    .await?;
    plan_with_runs(state, &slug).await
}

async fn delete_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
) -> Result<Response, ApiError> {
    plan_row_owned(&state, &plan_id, project.id()).await?;
    config::remove(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "plan",
        &plan_id,
    )?;
    state.services.config_reload()?;
    // 计划没了，它名下的自动投递规则也随之失效，一并清掉避免孤儿。
    crate::delivery::remove_plan_rule(&state, &plan_id).await?;
    crate::observe::append_audit(
        &state,
        "collection.plan.delete",
        "succeeded",
        "删除采集计划",
        json!({"planId": plan_id}),
        None,
    )
    .await?;
    Ok(util::ok(&json!({"id": plan_id, "deleted": true})))
}

// ---------------------------------------------------------------------------
// 采集策略：可复用的行为定义（节律/队列/额度/停止策略），无状态、可被多个计划引用。
// 计划通过 `strategy` 字段引用策略，并可在引用处做字段级覆写；运行状态（游标、
// paused、配额计数）始终归属计划自身，两个计划共用一条策略互不干扰。
// ---------------------------------------------------------------------------

pub(crate) fn strategy_rows(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let list = config::list(&state.config_dir)?;
    Ok(list["strategies"].as_array().cloned().unwrap_or_default())
}

pub(crate) fn strategy_row_owned(state: &AppState, strategy_id: &str) -> Result<Value, ApiError> {
    strategy_rows(state)?
        .into_iter()
        .find(|row| {
            row["slug"].as_str() == Some(strategy_id) || row["id"].as_str() == Some(strategy_id)
        })
        .ok_or_else(|| ApiError::not_found(format!("采集策略 {strategy_id}")))
}

async fn list_strategies(State(state): State<Shared>) -> Result<Response, ApiError> {
    // 与其它列表端点同形：`{ items: [...] }`，前端按 ListResponse 解析。
    let items = strategy_rows(&state)?;
    Ok(util::ok(&json!({ "items": items, "total": items.len() })))
}

async fn get_strategy(
    State(state): State<Shared>,
    Path(strategy_id): Path<String>,
) -> Result<Response, ApiError> {
    Ok(util::ok(&strategy_row_owned(&state, &strategy_id)?))
}

/// 可复用的新建策略逻辑：HTTP `POST /collection/strategies` 与工具面 `strategy.new` 共用。
pub(crate) async fn create_strategy_value(
    state: &AppState,
    project_id: &str,
    body: &Value,
) -> Result<Value, ApiError> {
    let map = util::object(body)?;
    util::unknown(
        map,
        &[
            "id",
            "name",
            "description",
            "targets",
            "trigger",
            "queue",
            "priority",
            "quota",
            "stopPolicy",
            "storage",
            "enabled",
            "pipeline",
            "vectorize",
            "deliveryTargets",
            "deliveryEnabled",
        ],
    )?;
    validate_storage(map.get("storage"))?;
    validate_pipeline(state, map.get("pipeline"))?;
    // 归属校验：策略 targets 引用的信源必须属于本策略所在项目（项目隔离不下放计划）。
    if let Some(targets) = map.get("targets").and_then(Value::as_array) {
        let owned: Vec<String> = targets
            .iter()
            .filter_map(Value::as_str)
            .map(util::source_slug)
            .collect();
        ensure_targets_owned(state, project_id, &owned).await?;
    }
    let trigger = map
        .get("trigger")
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            ApiError::validation("trigger", "required", "策略至少要定义 trigger（节律）")
        })?;
    if trigger.get("kind").and_then(Value::as_str).is_none() {
        return Err(ApiError::validation(
            "trigger.kind",
            "required",
            "trigger.kind 必须是 recurring 或 oneShot",
        ));
    }
    let slug = match map.get("id").and_then(Value::as_str) {
        Some(id) => id.trim_start_matches("strategy/").to_string(),
        None => format!("strategy-{}", util::hash_short(&json!(map).to_string())),
    };
    let mut doc = json!({
        "id": format!("strategy/{slug}"),
        "kind": "strategy",
        "name": map.get("name").cloned().unwrap_or(json!("采集策略")),
        "project": project_id,
        "trigger": trigger,
    });
    if let Some(object) = doc.as_object_mut() {
        for key in [
            "description",
            "targets",
            "queue",
            "priority",
            "quota",
            "stopPolicy",
            "storage",
            "enabled",
            "pipeline",
            "vectorize",
            "deliveryTargets",
            "deliveryEnabled",
        ] {
            if let Some(value) = map.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
    }
    config::create(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "strategy".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "collection.strategy.create",
        "succeeded",
        "新建采集策略",
        json!({"strategyId": slug}),
        None,
    )
    .await?;
    strategy_row_owned(state, &slug)
}

async fn create_strategy(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let strategy = create_strategy_value(&state, project.id(), &body).await?;
    Ok(util::ok(&strategy))
}

/// 可复用的修改策略逻辑：HTTP `PATCH` 与工具面 `strategy.set` 共用。
pub(crate) async fn update_strategy_value(
    state: &AppState,
    strategy_id: &str,
    body: &Value,
) -> Result<Value, ApiError> {
    let map = util::object(body)?;
    util::unknown(
        map,
        &[
            "name",
            "description",
            "targets",
            "trigger",
            "queue",
            "priority",
            "quota",
            "stopPolicy",
            "storage",
            "enabled",
            "deliveryTargets",
            "deliveryEnabled",
            "pipeline",
            "vectorize",
        ],
    )?;
    validate_storage(map.get("storage"))?;
    validate_pipeline(state, map.get("pipeline"))?;
    let entry = config::entries(&state.config_dir, "strategy")?
        .into_iter()
        .find(|entry| entry.slug == strategy_id)
        .ok_or_else(|| ApiError::not_found(format!("采集策略 {strategy_id}")))?;
    let mut doc = entry.doc.as_object().cloned().unwrap_or_default();
    if let Some(trigger) = util::optional_object(map, "trigger")? {
        let mut next = doc
            .get("trigger")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in trigger {
            next.insert(key, value);
        }
        doc.insert("trigger".into(), Value::Object(next));
    }
    for key in [
        "name",
        "description",
        "targets",
        "queue",
        "priority",
        "quota",
        "stopPolicy",
        "storage",
        "enabled",
        "deliveryTargets",
        "deliveryEnabled",
        "pipeline",
        "vectorize",
    ] {
        if let Some(value) = map.get(key) {
            doc.insert(key.into(), value.clone());
        }
    }
    // 归属校验：改 targets 时引用的信源必须属于策略所在项目（项目隔离不下放计划）。
    if map.get("targets").is_some() {
        let owned: Vec<String> = doc
            .get("targets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(util::source_slug))
            .collect();
        let strategy_project = doc
            .get("project")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        ensure_targets_owned(state, &strategy_project, &owned).await?;
    }
    let delivery_targets = map.get("deliveryTargets").cloned();
    let delivery_enabled = map.get("deliveryEnabled").and_then(Value::as_bool);
    config::update(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "strategy".into(),
            slug: strategy_id.to_string(),
            doc: Value::Object(doc),
        },
    )?;
    state.services.config_reload()?;
    // 投递目标随策略声明走：策略变化后，为所有引用它的计划重新同步投递规则。
    if delivery_targets.is_some() || delivery_enabled.is_some() {
        let plans = config::list(&state.config_dir)?["plans"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for plan in plans {
            let reference = plan["strategy"].as_str().unwrap_or_default();
            if reference.trim_start_matches("strategy/") != strategy_id {
                continue;
            }
            let plan_id = plan["slug"].as_str().unwrap_or_default();
            let plan_name = plan["name"].as_str().unwrap_or_default();
            crate::delivery::sync_plan_rule(
                state,
                plan_id,
                plan_name,
                delivery_targets.as_ref(),
                delivery_enabled,
            )
            .await?;
        }
    }
    crate::observe::append_audit(
        state,
        "collection.strategy.update",
        "succeeded",
        "调整采集策略",
        json!({"strategyId": strategy_id}),
        None,
    )
    .await?;
    strategy_row_owned(state, strategy_id)
}

async fn update_strategy(
    State(state): State<Shared>,
    Path(strategy_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let strategy = update_strategy_value(&state, &strategy_id, &body).await?;
    Ok(util::ok(&strategy))
}

/// 可复用的删除策略逻辑：HTTP `DELETE` 与工具面 `strategy.remove` 共用。
pub(crate) async fn delete_strategy_value(
    state: &AppState,
    strategy_id: &str,
) -> Result<Value, ApiError> {
    strategy_row_owned(state, strategy_id)?;
    // 引用护栏：还有计划在引用的策略直接拒删并回显占用方，避免计划静默失去节律。
    let list = config::list(&state.config_dir)?;
    let referring: Vec<String> = list["plans"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|plan| {
            let plan_id = plan["slug"].as_str()?;
            let strategy = plan
                .pointer("/doc/strategy")
                .or_else(|| plan.get("strategy"))?
                .as_str()?
                .trim_start_matches("strategy/");
            (strategy == strategy_id).then(|| plan_id.to_string())
        })
        .collect();
    if !referring.is_empty() {
        return Err(ApiError::validation(
            "strategy",
            "inUse",
            format!("策略仍被计划引用: {}", referring.join(", ")),
        ));
    }
    config::remove(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        "strategy",
        strategy_id,
    )?;
    state.services.config_reload()?;
    crate::observe::append_audit(
        state,
        "collection.strategy.delete",
        "succeeded",
        "删除采集策略",
        json!({"strategyId": strategy_id}),
        None,
    )
    .await?;
    Ok(json!({"id": strategy_id, "deleted": true}))
}

async fn delete_strategy(
    State(state): State<Shared>,
    Path(strategy_id): Path<String>,
) -> Result<Response, ApiError> {
    let payload = delete_strategy_value(&state, &strategy_id).await?;
    Ok(util::ok(&payload))
}

/// 批量作业：一次多信源单次采集。建一份临时 oneShot 计划留痕，并即时派发各目标任务。
async fn create_job(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["targets", "sourceIds", "name", "queue", "priority"])?;
    let targets = extract_targets(map);
    if targets.is_empty() {
        return Err(ApiError::validation(
            "targets",
            "required",
            "作业至少要有一个信源",
        ));
    }
    ensure_targets_owned(&state, project.id(), &targets).await?;
    // 单一语义：作业计划同样引用策略（有计划必有策略），一次性语义由专用策略
    // `strategy/oneshot-audit`（trigger.kind = oneShot）承载，计划上不写任何行为字段。
    ensure_oneshot_audit_strategy(&state)?;
    let slug = derive_plan_slug(map, &targets);
    let doc = json!({
        "id": format!("plan/{slug}"),
        "kind": "plan",
        "name": map.get("name").cloned().unwrap_or(json!("一次性采集作业")),
        "project": project.id(),
        "strategy": "strategy/oneshot-audit",
        // 作业计划仅留痕审计：即时 task_create 已派发，禁用可避免调度轮次二次派发同一 oneShot。
        "enabled": false,
    });
    // 临时作业计划落盘留痕，便于审计；派发即时、不参与后续定时节律。
    config::create(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: slug.clone(),
            doc,
        },
    )?;
    state.services.config_reload()?;
    let queue = match map.get("queue").and_then(Value::as_str) {
        Some("paid") => Some(QueueName::Paid),
        Some("render") => Some(QueueName::Render),
        Some("push") => Some(QueueName::Push),
        Some("free") => Some(QueueName::Free),
        _ => None,
    };
    let mut dispatched = Vec::new();
    let mut failed = Vec::new();
    for target in &targets {
        // 回包统一裸 slug：`/collection/runs`、信源清单对外都是裸 slug，别在这里换形态。
        let source_id = util::source_slug(target);
        match state.services.task_create(target, queue, true, None).await {
            Ok(row) => dispatched.push(
                json!({"sourceId": source_id, "taskId": row.task_id, "status": row.status}),
            ),
            Err(error) => failed.push(json!({"sourceId": source_id, "error": error.to_string()})),
        }
    }
    crate::observe::append_audit(
        &state,
        "collection.job.create",
        if failed.is_empty() {
            "succeeded"
        } else {
            "partial"
        },
        "批量采集作业",
        json!({"planId": slug, "dispatched": dispatched.len(), "failed": failed.len()}),
        None,
    )
    .await?;
    crate::notifications::emit(
        &state,
        "plan",
        "plan.started",
        if failed.is_empty() { "info" } else { "warn" },
        format!(
            "采集作业已启动：{}",
            map.get("name")
                .and_then(Value::as_str)
                .unwrap_or("一次性采集作业")
        ),
        format!(
            "派发 {} 个信源 · 失败 {} 个",
            dispatched.len(),
            failed.len()
        ),
        Some("tasks"),
    )
    .await;
    Ok(util::ok(&json!({
        "planId": slug,
        "dispatched": dispatched,
        "failed": failed,
    })))
}

async fn update_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let etag = headers.get("if-match").and_then(|v| v.to_str().ok());
    let plan = update_plan_value(&state, project.id(), &plan_id, etag, &body).await?;
    Ok(util::ok(&plan))
}

/// 可复用的修改计划逻辑：HTTP `PATCH /collection/plans/{id}` 与工具面 `plan.set` 共用。
/// `etag`：HTTP 面从 If-Match 头取；工具面传 `None` 时用当前计划版本自洽通过。
pub(crate) async fn update_plan_value(
    state: &AppState,
    project_id: &str,
    plan_id: &str,
    etag: Option<&str>,
    body: &Value,
) -> Result<Value, ApiError> {
    let map = util::object(body)?;
    // 单一语义：计划是纯状态实例，可改的只有状态与元数据（策略引用、enabled、
    // 一次性 URL 数据、名称/说明）；行为字段全部在策略上。
    util::unknown(
        map,
        &["strategy", "enabled", "urls", "appendUrls", "name", "description"],
    )?;
    let plan = plan_row_owned(state, plan_id, project_id).await?;
    match etag {
        Some(value) => util::if_match_value(value, plan["etag"].as_str().unwrap_or(""))?,
        None => {}
    }
    let entry = config::entries(&state.config_dir, "plan")?
        .into_iter()
        .find(|entry| entry.slug == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    let mut doc = entry.doc.as_object().cloned().unwrap_or_default();
    for key in ["name", "description", "enabled"] {
        if let Some(value) = map.get(key) {
            doc.insert(key.into(), value.clone());
        }
    }
    // 改引策略：新策略必须存在；recurring 策略在同一项目内至多一个实例（排除自身）。
    let mut changed_strategy: Option<(Value, bool)> = None;
    if let Some(reference) = map
        .get("strategy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // 规范化为裸 slug：与 author 行的 slug/id 双形态对齐，避免前缀形态错位。
        let strategy_slug = reference.trim_start_matches("strategy/").to_string();
        let strategy_id = format!("strategy/{strategy_slug}");
        let list = config::list(&state.config_dir)?;
        let strategy_doc = list["strategies"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .find(|item| {
                item["slug"].as_str() == Some(strategy_slug.as_str())
                    || item["id"].as_str() == Some(strategy_id.as_str())
            })
            .ok_or_else(|| {
                ApiError::validation(
                    "strategy",
                    "notFound",
                    format!("策略 {strategy_id} 不存在"),
                )
            })?;
        // 单一语义：改引策略即更换行为基座；投递目标随新策略重新同步（见更新后）。
        let recurring = strategy_doc["trigger"]["kind"].as_str() != Some("oneShot");
        // recurring 策略必须有目标信源，否则计划会静默产出 0 条 SourcePlan。
        if recurring
            && strategy_doc
                .get("targets")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
                == 0
        {
            return Err(ApiError::validation(
                "strategy",
                "invalid",
                format!("recurring 策略 {strategy_id} 缺少 targets，请先在策略上配置目标信源"),
            ));
        }
        for other in list["plans"].as_array().cloned().unwrap_or_default() {
            let other_id = other["id"].as_str().unwrap_or_default();
            // 自排除：比对剥离 `plan/` 前缀后的裸 slug。
            if other_id.trim_start_matches("plan/") == plan_id {
                continue;
            }
            let same_strategy = other["strategy"]
                .as_str()
                .map(|value| value.trim_start_matches("strategy/") == strategy_slug.as_str())
                .unwrap_or(false);
            if same_strategy && other["project"].as_str() == Some(project_id) {
                return Err(ApiError::validation(
                    "strategy",
                    "conflict",
                    format!("策略 {strategy_id} 已在本项目实例化为计划 {other_id}"),
                ));
            }
        }
        doc.insert("strategy".into(), json!(strategy_id));
        changed_strategy = Some((strategy_doc, recurring));
    }
    // URL 目标追加（`mp search --into <计划>` / `plan.set appendUrls`）：去重、保序。
    if let Some(append) = util::optional_array(map, "appendUrls")? {
        let mut urls: Vec<String> = doc
            .get("urls")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        for value in append {
            let Some(url) = value.as_str().map(str::to_string) else {
                return Err(ApiError::validation(
                    "appendUrls",
                    "invalid",
                    "appendUrls 必须是字符串数组",
                ));
            };
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ApiError::validation(
                    "appendUrls",
                    "invalid",
                    format!("URL 目标仅支持 http/https: {url}"),
                ));
            }
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        doc.insert("urls".into(), json!(urls));
    }
    let plan_name = doc
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("采集计划")
        .to_string();
    config::update(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: plan_id.to_string(),
            doc: Value::Object(doc),
        },
    )?;
    state.services.config_reload()?;
    // 改引策略后，投递规则随新策略的投递配置重新同步。
    if let Some((strategy_doc, _recurring)) = changed_strategy {
        crate::delivery::sync_plan_rule(
            state,
            plan_id,
            &plan_name,
            strategy_doc.get("deliveryTargets"),
            strategy_doc.get("deliveryEnabled").and_then(Value::as_bool),
        )
        .await?;
    }
    crate::observe::append_audit(
        state,
        "collection.plan.update",
        "succeeded",
        "调整采集计划",
        json!({"planId": plan_id, "strategy": map.get("strategy")}),
        None,
    )
    .await?;
    plan_with_runs(state, plan_id).await
}

/// 计划行 + 进度摘要：plan 写动词与 `plan.get` 共用的返回形状。
async fn plan_with_runs(state: &AppState, plan_id: &str) -> Result<Value, ApiError> {
    let bare = plan_id.strip_prefix("plan/").unwrap_or(plan_id);
    let mut plan = plan_rows(state)
        .await?
        .into_iter()
        .find(|plan| plan["id"] == bare)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {bare}")))?;
    plan["recentRuns"] = Value::Array(runs_for_plan(state, bare, 5).await?);
    Ok(plan)
}

/// `plan.get`：计划详情 + 进度 + 每信源游标。
///
/// 游标语义：accepted 是恢复时续取的位置，visible 是快照可见位置；只支持游标的
/// 访问器（rss / api / paid）才有值，http、browser、bundle、push、search 返回 null。
/// 配置指纹（configHash）变化会丢弃游标（迁移代除外）——这是已有行为，这里原样透出。
pub(crate) async fn plan_get_value(state: &AppState, project_id: &str, plan_id: &str) -> Result<Value, ApiError> {
    let mut plan = plan_with_runs(state, plan_id).await?;
    if plan["project"].as_str() != Some(project_id) {
        return Err(ApiError::not_found(format!("采集计划 {plan_id}")));
    }
    let sources = crate::catalog::source_rows(state).await?;
    let mut cursors = Vec::new();
    for target in plan["targets"].as_array().cloned().unwrap_or_default() {
        let Some(source_id) = target.as_str().map(util::source_slug) else {
            continue;
        };
        let (accepted, visible, cumulative) = state
            .services
            .source_cursor(&util::source_ref(&source_id))
            .await
            .map_err(ApiError::from)?;
        let row = sources.iter().find(|row| row["id"].as_str() == Some(source_id.as_str()));
        cursors.push(json!({
            "sourceId": source_id,
            // 累计产出条数（跨全部日桶），与派发侧 totalItems 上限判定同源。
            "itemCount": cumulative,
            "acceptedCursor": accepted.unwrap_or(Value::Null),
            "visibleCursor": visible.unwrap_or(Value::Null),
            "lastRunAt": row.map(|row| row["lastRunAt"].clone()).unwrap_or(Value::Null),
            "lastError": row.map(|row| row["lastError"].clone()).unwrap_or(Value::Null),
            "health": row.map(|row| row["health"].clone()).unwrap_or(Value::Null),
            "nextRunAt": row.map(|row| row["nextRunAt"].clone()).unwrap_or(Value::Null),
        }));
    }
    plan["cursors"] = Value::Array(cursors);
    // M-05：计划级存储根与分类规则可查询。effectiveRoot 相对路径相对进程工作目录解析，
    // 与执行侧落盘解析一致；classifyRule 是约定目录的固定说明串。
    // 单一语义：storage 由策略承载，经计划引用的策略解析。
    if let Ok(entries) = config::entries(&state.config_dir, "plan") {
        let slug = plan["id"].as_str().unwrap_or_default().trim_start_matches("plan/");
        if let Some(entry) = entries.into_iter().find(|entry| entry.slug == slug) {
            let strategy_entries = config::entries(&state.config_dir, "strategy")
                .unwrap_or_default();
            let strategy_slug = entry
                .doc
                .get("strategy")
                .and_then(Value::as_str)
                .map(|value| value.trim_start_matches("strategy/").to_string())
                .unwrap_or_default();
            let storage = strategy_entries
                .into_iter()
                .find(|item| item.slug == strategy_slug)
                .and_then(|item| item.doc.get("storage").cloned())
                .unwrap_or(Value::Null);
            let root = storage
                .get("root")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let classify = storage
                .get("classify")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (effective_root, rule) = if root.is_empty() {
                (Value::Null, Value::Null)
            } else {
                // M-05：相对 root 相对 --data 解析，与执行侧落盘（materialize）一致。
                let effective = std::path::Path::new(root)
                    .is_absolute()
                    .then(|| root.to_string())
                    .unwrap_or_else(|| {
                        state
                            .data_dir
                            .join(root)
                            .to_string_lossy()
                            .into_owned()
                    });
                (
                    json!(effective),
                    json!(format!(
                        "{{effectiveRoot}}/{{sourceId}}/{{kind}}/{{itemId}}{{ext}}"
                    )),
                )
            };
            plan["storage"] = json!({
                "root": storage.get("root").cloned().unwrap_or(Value::Null),
                "classify": classify,
                "effectiveRoot": effective_root,
                "classifyRule": rule,
            });
        }
    }
    Ok(plan)
}

/// 计划级立即执行一轮：不重置游标，从上一已接受游标继续；已暂停或停用的计划拒绝。
///
/// 与 HTTP /collection/runs 的手动触发共用派发入口（dispatch_round），跑一轮后照常
/// 提交游标；对带 `urls` 的 oneShot 计划（来自 `mp plan new --from-search`），逐条
/// URL 走 once 的统一摄入路径（直接 HTTP，不经浏览器自动化）。
pub(crate) async fn plan_run_value(state: &Shared, project_id: &str, plan_id: &str) -> Result<Value, ApiError> {
    let plan = plan_row_owned(state, plan_id, project_id).await?;
    if !plan["enabled"].as_bool().unwrap_or(true) {
        return Err(ApiError::validation(
            "enabled",
            "conflict",
            format!("采集计划 {plan_id} 已停用（enabled=false），不能运行；先启用计划"),
        ));
    }
    if plan["paused"].as_bool().unwrap_or(false) {
        return Err(ApiError::validation(
            "paused",
            "conflict",
            format!("采集计划 {plan_id} 已暂停；先 resume 再运行（暂停是主动中断，不悄悄解除）"),
        ));
    }
    // 带显式目标列表的 oneShot 计划（`mp plan new --from-search` 产物）不走
    // 信源派发：逐条 URL 走 once 的统一摄入路径（直接 HTTP，不经浏览器自动化）。
    let urls: Vec<String> = config::entries(&state.config_dir, "plan")?
        .into_iter()
        .find(|entry| entry.slug == plan_id)
        .and_then(|entry| entry.doc.get("urls").cloned())
        .and_then(|value| value.as_array().cloned())
        .map(|items| {
            items
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut results = Vec::new();
    for url in &urls {
        // 直接走 once 的统一摄入实现，不经工具面分发（避免 plan.run → tools.call 的
        // 互相递归）；mode 固定 store，无 --file 面。
        let args = json!({ "url": url, "mode": "store" });
        let outcome = async {
            let request = assembly::tools::OnceRequest::parse_for_surface(&state.services, &args, false)
                .await
                .map_err(crate::tools::tool_error)?;
            assembly::tools::once(&state.services, request)
                .await
                .map_err(crate::tools::tool_error)
        };
        match outcome.await {
            Ok(value) => results.push(json!({ "url": url, "ok": true, "result": value })),
            Err(error) => results.push(json!({ "url": url, "ok": false, "error": error.body() })),
        }
    }
    let mut dispatched = Vec::new();
    let mut failed = Vec::new();
    for target in plan["targets"].as_array().cloned().unwrap_or_default() {
        let Some(source_id) = target.as_str().map(util::source_slug) else {
            continue;
        };
        match state
            .services
            .dispatch_round(Some(&util::source_ref(&source_id)))
            .await
        {
            Ok(round) => dispatched.push(json!({
                "sourceId": source_id,
                "leased": round.leased,
                "executed": round.executed.len(),
                "failed": round.failed.len(),
            })),
            Err(error) => failed.push(json!({ "sourceId": source_id, "error": error.to_string() })),
        }
    }
    crate::observe::append_audit(
        state,
        "collection.plan.run",
        if failed.is_empty() { "succeeded" } else { "partial" },
        "计划立即执行一轮",
        json!({"planId": plan_id, "targets": dispatched.len(), "urls": urls.len()}),
        None,
    )
    .await?;
    Ok(json!({
        "planId": plan_id,
        "dispatched": dispatched,
        "failed": failed,
        "urls": results,
    }))
}

async fn pause_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let etag = headers.get("if-match").and_then(|v| v.to_str().ok());
    let plan = pause_resume_plan_value(&state, project.id(), &plan_id, etag, true).await?;
    Ok(util::ok(&plan))
}

async fn resume_plan(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Path(plan_id): Path<String>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiError> {
    let etag = headers.get("if-match").and_then(|v| v.to_str().ok());
    let plan = pause_resume_plan_value(&state, project.id(), &plan_id, etag, false).await?;
    Ok(util::ok(&plan))
}

/// 可复用的暂停/恢复逻辑：HTTP pause/resume 与工具面 `plan.pause` / `plan.resume` 共用。
/// `etag`：HTTP 面从 If-Match 头取；工具面传 `None` 时用当前计划版本自洽通过。
///
/// 切片 2 语义变更：暂停不再把 `enabled` 改成 false，而是写入独立的 `paused` 字段
/// （仅当 enabled 为 true 时有意义）。暂停期间不派发新任务；已开始的任务允许做完
/// 并提交游标后停；不回滚已提交数据，下次从上一已接受游标继续（去重兜底）。
/// 对停用（enabled=false）计划调用 pause/resume 返回明确错误。
pub(crate) async fn pause_resume_plan_value(
    state: &AppState,
    project_id: &str,
    plan_id: &str,
    etag: Option<&str>,
    pause: bool,
) -> Result<Value, ApiError> {
    let plan = plan_row_owned(state, plan_id, project_id).await?;
    if let Some(value) = etag {
        util::if_match_value(value, plan["etag"].as_str().unwrap_or(""))?;
    }
    if !plan["enabled"].as_bool().unwrap_or(true) {
        return Err(ApiError::validation(
            "enabled",
            "conflict",
            format!("采集计划 {plan_id} 已停用（enabled=false），不能暂停或恢复；先启用计划"),
        ));
    }
    if plan["paused"].as_bool().unwrap_or(false) == pause {
        // 幂等：重复 pause/resume 不改文档、不触发重载，直接返回当前视图。
        return plan_with_runs(state, plan_id).await;
    }
    let entry = config::entries(&state.config_dir, "plan")?
        .into_iter()
        .find(|entry| entry.slug == plan_id)
        .ok_or_else(|| ApiError::not_found(format!("采集计划 {plan_id}")))?;
    let mut doc = entry.doc.as_object().cloned().unwrap_or_default();
    doc.insert("paused".into(), json!(pause));
    config::update(
        &state.config_dir,
        &config::LoadOptions::from_process(),
        &config::Draft {
            kind: "plan".into(),
            slug: plan_id.to_string(),
            doc: Value::Object(doc),
        },
    )?;
    state.services.config_reload()?;
    let (kind, summary) = if pause {
        ("collection.plan.pause", "暂停采集计划")
    } else {
        ("collection.plan.resume", "恢复采集计划")
    };
    crate::observe::append_audit(
        state,
        kind,
        "succeeded",
        summary,
        json!({"planId": plan_id, "paused": pause}),
        None,
    )
    .await?;
    plan_with_runs(state, plan_id).await
}

/// 计划维度的近期运行：工具面 `plan.get` / `plan.new` 的进度摘要，与运行记录同源。
pub(crate) async fn runs_for_plan(
    state: &AppState,
    plan_id: &str,
    limit: usize,
) -> Result<Vec<Value>, ApiError> {
    let filter = TaskFilter::default();
    let result = state
        .services
        .queue_list(&filter, None, 200)
        .await
        .map_err(ApiError::from)?;
    let mut runs: Vec<Value> = result.rows.iter().map(run_view).collect();
    let owner = plan_owner_map(state).await?;
    for run in &mut runs {
        attribute_plan(run, &owner);
    }
    let plan = plan_id.strip_prefix("plan/").unwrap_or(plan_id);
    runs.retain(|run| run["planId"].as_str() == Some(plan));
    runs.truncate(limit);
    Ok(runs)
}

async fn list_runs(
    State(state): State<Shared>,
    project: crate::ActiveProject,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!(
        "runs:{}:{}:{}:{}",
        query.status.clone().unwrap_or_default(),
        query.source_id.clone().unwrap_or_default(),
        query.plan_id.clone().unwrap_or_default(),
        project.id()
    );
    let offset = util::page_offset(query.offset.as_deref(), query.cursor.as_deref(), &filter)?;
    let task_filter = TaskFilter {
        // 任务表按 `source/<slug>` 存归属，接口对外统一用裸 slug。
        source: query
            .source_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(util::source_ref),
        ..TaskFilter::default()
    };
    let result = state
        .services
        .queue_list(&task_filter, None, 200)
        .await
        .map_err(ApiError::from)?;
    let mut runs: Vec<Value> = result.rows.iter().map(run_view).collect();
    // 项目隔离：运行按其信源归属过滤。
    let allowed = visible_sources_opt(&state, &project, query.scope.as_deref()).await?;
    if let Some(allowed) = &allowed {
        runs.retain(|run| {
            let id = run["sourceId"].as_str().unwrap_or_default();
            !id.is_empty() && allowed.contains(id)
        });
    }
    // 历史任务把计划归属写成了信源 slug（旧版一信源一计划）；按当前计划的目标回填一等计划，
    // 否则计划维度的采集记录永远是空的。
    let owner = plan_owner_map(&state).await?;
    for run in &mut runs {
        attribute_plan(run, &owner);
    }
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        runs.retain(|run| run["status"] == status);
    }
    if let Some(plan) = query.plan_id.as_deref().filter(|value| !value.is_empty()) {
        let plan = plan.strip_prefix("plan/").unwrap_or(plan);
        runs.retain(|run| run["planId"].as_str() == Some(plan));
    }
    let total = runs.len();
    let items: Vec<Value> = runs.into_iter().skip(offset).take(limit + 1).collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page_total(items, next, total)))
}

fn run_view(row: &strata_client::TaskRow) -> Value {
    // 接口对外统一裸 slug（与信源/计划清单一致）；任务表内部带 `source/`、`plan/` 前缀。
    let source_id = row
        .source_id
        .strip_prefix("source/")
        .unwrap_or(&row.source_id)
        .to_string();
    let plan_id = row
        .plan_id
        .as_deref()
        .map(|id| id.strip_prefix("plan/").unwrap_or(id).to_string());
    let mut view = json!({
        "id": format!("run_{}", row.task_id),
        "sourceId": source_id,
        // 计划归属来自任务 payload：信源与计划是采集记录仅有的两条上游线索。
        "planId": plan_id,
        "mode": if row.reason == "manual" { "manual" } else { "scheduled" },
        "status": match row.status.as_str() {
            "done" => "succeeded",
            "leased" => "running",
            "cancelled" => "cancelled",
            other => other,
        },
        "taskId": row.task_id,
        "queue": row.queue,
        "attempt": row.attempt,
        "locator": row.locator,
        "createdAt": row.created_at,
        "updatedAt": row.lease_until.clone().unwrap_or_else(|| row.created_at.clone()),
        "error": row.last_error,
    });
    crate::dictionary::decorate(
        &mut view,
        &[
            ("mode", "run.mode"),
            ("status", "run.status"),
            ("queue", "task.queue"),
        ],
    );
    view
}

async fn get_run(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
) -> Result<Response, ApiError> {
    let task_id = run_id.trim_start_matches("run_");
    let filter = TaskFilter {
        query: Some(task_id.to_string()),
        ..TaskFilter::default()
    };
    let page = state
        .services
        .queue_list(&filter, None, 5)
        .await
        .map_err(ApiError::from)?;
    let row = page
        .rows
        .iter()
        .find(|row| row.task_id == task_id)
        .ok_or_else(|| ApiError::not_found(format!("采集运行 {run_id}")))?;
    let mut view = run_view(row);
    let owner = plan_owner_map(&state).await?;
    attribute_plan(&mut view, &owner);
    Ok(util::ok(&view))
}

/// 信源 slug → 计划 id：按当前计划的目标反查，只给历史遗留归属用。
async fn plan_owner_map(
    state: &AppState,
) -> Result<std::collections::BTreeMap<String, String>, ApiError> {
    let mut owner = std::collections::BTreeMap::new();
    for plan in plan_rows(state).await? {
        let Some(id) = plan["id"].as_str() else {
            continue;
        };
        for target in plan["targets"].as_array().cloned().unwrap_or_default() {
            if let Some(value) = target.as_str() {
                let bare = value.strip_prefix("source/").unwrap_or(value);
                owner
                    .entry(bare.to_string())
                    .or_insert_with(|| id.to_string());
            }
        }
    }
    Ok(owner)
}

/// 归属为空或等于信源 slug 时，按目标反查补成计划 id；已经是计划 id 的不动。
fn attribute_plan(
    run: &mut Value,
    owner: &std::collections::BTreeMap<String, String>,
) {
    let source = run["sourceId"].as_str().unwrap_or_default().to_string();
    let plan = run["planId"].as_str().unwrap_or_default().to_string();
    if !(plan.is_empty() || plan == source) {
        return;
    }
    if let Some(id) = owner.get(&source) {
        run["planId"] = json!(id);
    }
}

async fn create_run(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let operation = create_run_value(&state, &body).await?;
    Ok(accepted(&operation))
}

/// 可复用的手动触发逻辑：HTTP `POST /collection/runs` 与工具面 `plan.run` 共用，
/// 返回异步操作值（202 信封由调用方包）。
pub(crate) async fn create_run_value(state: &Shared, body: &Value) -> Result<Value, ApiError> {
    let map = util::object(body)?;
    util::unknown(map, &["sourceId", "mode", "locator", "reason"])?;
    // 信源标识两种形态都接受：其它接口回的是带前缀的模块 id，回传时不能再叠一层前缀。
    let source_id = util::source_slug(&util::non_empty(map, "sourceId")?);
    if source_id.is_empty() {
        return Err(ApiError::validation("sourceId", "required", "不能为空"));
    }
    let mode = util::text(map, "mode")?;
    if !matches!(mode.as_str(), "normal" | "dryRun" | "singleLocator") {
        return Err(ApiError::validation(
            "mode",
            "enum",
            "只支持 normal、dryRun 或 singleLocator",
        ));
    }
    let locator = util::optional_text(map, "locator")?;
    if mode == "singleLocator" && locator.is_none() {
        return Err(ApiError::validation(
            "locator",
            "required",
            "单选地址模式需要 locator",
        ));
    }
    let reason = util::optional_text(map, "reason")?;
    let run_id = crate::new_id("run");
    crate::operations::start(
        state,
        "collection.run",
        vec![json!({"kind": "collectionRun", "id": run_id})],
        json!({
            "kind": "collection.run",
            "runId": run_id,
            "sourceId": source_id,
            "mode": mode,
            "locator": locator,
            "reason": reason,
        }),
    )
    .await
}

pub async fn execute_run(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let source_id = action["sourceId"].as_str().unwrap_or_default();
    let mode = action["mode"].as_str().unwrap_or("normal");
    let locator = action["locator"].as_str().map(str::to_string);
    if crate::operations::check_cancel(state, operation_id)
        .await
        .is_err()
    {
        return Err(ApiError::conflict("操作已取消"));
    }
    let row = state
        .services
        .task_create(
            &util::source_ref(source_id),
            Some(QueueName::Free),
            true,
            locator,
        )
        .await
        .map_err(ApiError::from)?;
    let task_id = row.task_id.clone();
    crate::observe::append_event(
        state,
        "collection.run",
        "采集运行已创建",
        json!({"runId": action["runId"], "sourceId": source_id, "mode": mode, "taskId": task_id, "status": row.status}),
        crate::observe::Refs {
            operation_id: Some(operation_id.to_string()),
            source_id: Some(source_id.to_string()),
            task_id: Some(task_id.clone()),
            ..crate::observe::Refs::default()
        },
    )
    .await?;
    Ok(json!({
        "partial": false,
        "runId": action["runId"],
        "sourceId": util::source_slug(source_id),
        "mode": mode,
        "taskId": task_id,
        "status": row.status,
        // 失败原因随结果回显：通知与操作详情都不必再回查队列。
        "error": row.last_error,
    }))
}
