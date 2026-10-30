//! 外部调用：按协议列出本进程真正提供的入口，并给出可直接复制的快速指引。
//!
//! 接口清单是静态表（[`endpoints`]），由 `tests/external_docs.rs` 与路由表逐条核对：
//! 加了路由却忘了补文档，测试会失败。未启用的协议明确标注，不给不可用的指令。

mod endpoints;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::util;
use crate::{Shared, now};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/external/protocols", get(list_protocols))
        .route("/external/apiDocs", get(api_docs))
}

/// 一条 HTTP 接口：方法、路径、用途，以及调用时要带的参数。
pub(crate) struct Endpoint {
    pub(crate) group: &'static str,
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
    pub(crate) summary: &'static str,
    /// 关键查询参数，逗号分隔；没有则 `None`。
    pub(crate) query: Option<&'static str>,
    /// 请求体要点（或占位 `{}`）；GET 为 `None`。
    pub(crate) body: Option<&'static str>,
}

/// 一个功能分组。控制台按组自上而下展示。
pub(crate) struct Group {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) note: &'static str,
}

/// (分组, 方法, 路径)：给一致性测试与脚本核对用。
pub fn documented() -> Vec<(&'static str, &'static str, &'static str)> {
    endpoints::ENDPOINTS
        .iter()
        .map(|item| (item.group, item.method, item.path))
        .collect()
}

/// 分组 id 清单，顺序即页面顺序。
pub fn group_ids() -> Vec<&'static str> {
    endpoints::GROUPS.iter().map(|group| group.id).collect()
}

fn http_groups() -> Vec<Value> {
    endpoints::GROUPS
        .iter()
        .map(|group| {
            let items: Vec<Value> = endpoints::ENDPOINTS
                .iter()
                .filter(|item| item.group == group.id)
                .map(|item| {
                    json!({
                        "method": item.method,
                        "path": item.path,
                        "summary": item.summary,
                        "query": item.query,
                        "body": item.body,
                    })
                })
                .collect();
            json!({
                "id": group.id,
                "label": group.label,
                "note": group.note,
                "items": items,
            })
        })
        .collect()
}

/// 调用约定：头部、错误、分页与异步语义。放在一处，避免每条接口重复声明。
fn conventions() -> Value {
    json!({
        "headers": [
            {"name": "x-project-id", "optional": true, "note": "项目隔离；不传按默认项目处理。"},
            {"name": "idempotency-key", "optional": true, "note": "非 GET 建议带上；同键重放返回首次结果。"},
            {"name": "if-match", "optional": false, "note": "修改、删除与状态切换必带，取上次响应的 ETag。"},
        ],
        "errors": "失败统一返回 error 对象：code、message、requestId、retryable、fieldErrors；requestId 可在日志页按关联号检索。",
        "paging": "列表返回 items 与 nextCursor，或 items 与 total；用 limit 配合 cursor/offset 翻页。",
        "async": "会改数据的重动作返回 202 与 operationId，轮询 /operations/{operationId} 直到 succeeded、partiallySucceeded 或 failed。",
        "etag": "版本化资源的 PATCH、DELETE 与 enable/disable/pause/resume/approve/reject 都要求 If-Match。",
    })
}

fn cli_items() -> Vec<Value> {
    vec![
        json!({"name": "启动服务", "usage": "modalprism-collector [--data var] [--config config] [--http 127.0.0.1:17880] [--proxy 127.0.0.1:17881] [--push 127.0.0.1:17882]", "summary": "不带动词时启动进程：HTTP 领域 API、代理与推送三个端口"}),
        json!({"name": "search", "usage": "modalprism-collector search <关键词> [--engine <id>] [--project <项目>]", "summary": "调用 /search，按默认引擎搜索并打印 JSON"}),
        json!({"name": "fetch", "usage": "modalprism-collector fetch <url>", "summary": "调用 /fetch，取回网页正文"}),
        json!({"name": "parse", "usage": "modalprism-collector parse <url>", "summary": "调用 /parse，做结构化抽取"}),
        json!({"name": "project list", "usage": "modalprism-collector project list", "summary": "列出项目"}),
        json!({"name": "project create", "usage": "modalprism-collector project create <名称>", "summary": "新建项目"}),
        json!({"name": "project resources", "usage": "modalprism-collector project resources <项目标识>", "summary": "列出项目下的资源归属"}),
        json!({"name": "preconditions", "usage": "modalprism-collector preconditions", "summary": "打印就绪检查结果"}),
        json!({"name": "通用参数", "usage": "--http <地址>", "summary": "动词默认连 127.0.0.1:17880，可用它指向其他实例"}),
    ]
}

fn mcp_items() -> Vec<Value> {
    let mut items = vec![
        json!({"name": "initialize", "usage": "MCP 握手", "summary": "交换协议版本与客户端信息"}),
        json!({"name": "tools/list", "usage": "列出工具", "summary": "工具名形如 ability.method，只读进程只返回只读方法"}),
        json!({"name": "tools/call", "usage": "调用工具", "summary": "按工具名传参，结果原样返回"}),
    ];
    for (ability, method) in mcp_documented() {
        let usage = if surface::mcp_write_allowed(ability, method) {
            format!("tools/call · {} · 可恢复写", ability_label(ability))
        } else {
            format!("tools/call · {}", ability_label(ability))
        };
        items.push(json!({
            "name": format!("{ability}.{method}"),
            "usage": usage,
            "summary": mcp_method_summary(ability, method),
        }));
    }
    items
}

/// MCP 工具清单：只读白名单加可恢复写白名单，与只读进程的治理表同源。
pub fn mcp_documented() -> Vec<(&'static str, &'static str)> {
    let mut items: Vec<(&'static str, &'static str)> = Vec::new();
    for exposure in surface::AGENT_MCP {
        for method in exposure.methods {
            items.push((exposure.id, method));
        }
    }
    for exposure in surface::AGENT_MCP_WRITES {
        for method in exposure.methods {
            items.push((exposure.id, method));
        }
    }
    items
}

fn ability_label(id: &str) -> &'static str {
    match id {
        "catalog_admin" => "信源与模板",
        "config_inspect" => "配置溯源",
        "health" => "健康与状态",
        "strata_read" => "数据读取",
        "type_article" => "文章视图",
        "type_hotlist" => "热榜视图",
        "web" => "即时采集",
        _ => "能力",
    }
}

fn mcp_method_summary(ability: &str, method: &str) -> &'static str {
    match (ability, method) {
        ("catalog_admin", "list") => "信源、访问器与适配器清单",
        ("catalog_admin", "templates") => "建源模板与字段说明",
        ("catalog_admin", "create") => "新建声明草稿（落盘前校验）",
        ("catalog_admin", "update") => "改写声明草稿（落盘前校验）",
        ("catalog_admin", "toggle") => "启停声明，可恢复",
        ("config_inspect", "modules") => "配置模块与字段清单",
        ("config_inspect", "explain") => "某个字段来自哪里",
        ("config_inspect", "validate") => "校验一份配置变更",
        ("config_inspect", "diff") => "与当前配置代际对比",
        ("health", "status") => "进程状态、版本与配置代际",
        ("health", "report") => "信源健康与待处理项",
        ("strata_read", "query") => "类型化查询（过滤、全文、向量）",
        ("strata_read", "get") => "按标识读取资源",
        ("strata_read", "has") => "判断资源是否存在",
        ("strata_read", "history") => "资源的历史版本",
        ("strata_read", "changes") => "变更集与提交记录",
        ("strata_read", "kinds") => "资源类型与数量",
        ("strata_read", "lineage") => "逐层追溯产出来源",
        ("strata_read", "related") => "关联资源",
        ("strata_read", "resolve") => "把外部标识解析为 RID",
        ("strata_read", "slice") => "按区间读取大对象",
        ("strata_read", "summary") => "总量与分布摘要",
        ("type_article", "project") => "文章视图",
        ("type_article", "on_commit") => "提交时投影到类型视图",
        ("type_hotlist", "project") => "热榜视图",
        ("type_hotlist", "on_commit") => "提交时投影到类型视图",
        ("web", "search") => "即时关键词搜索",
        ("web", "fetch") => "URL 取正文",
        ("web", "parse") => "结构化抽取",
        _ => "能力方法",
    }
}

fn grpc_items() -> Vec<Value> {
    vec![
        json!({"name": "LeafGateway.Catalog", "usage": "rpc Catalog(CatalogRequest) returns (CatalogResponse)", "summary": "列出能力目录：instance、method 与描述"}),
        json!({"name": "LeafGateway.Invoke", "usage": "rpc Invoke(InvokeRequest) returns (InvokeResponse)", "summary": "按 instance 与 method 调用，参数与结果都用 JSON 字符串"}),
    ]
}

fn fun_items() -> Vec<Value> {
    vec![
        json!({"name": "Gateway.catalog", "usage": "进程内调用", "summary": "列出宿主已装载的能力与方法"}),
        json!({"name": "Gateway.invoke", "usage": "进程内调用", "summary": "按 instance 与 method 调用；不监听端口"}),
    ]
}

fn quick_start(base: &str) -> Vec<Value> {
    vec![
        json!({
            "title": "确认服务在线",
            "body": "所有调用都指向本机回环地址；返回 200 说明采集进程与领域 API 已就绪。",
            "commands": [format!("curl {base}/system/status")],
        }),
        json!({
            "title": "按项目读写",
            "body": "多项目场景用 x-project-id 指定归属；不传则按默认项目处理。",
            "commands": [format!("curl -H \"x-project-id: proj_default\" \"{base}/content/items?limit=5\"")],
        }),
        json!({
            "title": "读内容",
            "body": "内容 ID 形如 content_xxx，sourceId 形如 source/openai-news；列表默认不含已删除记录。",
            "commands": [format!("curl \"{base}/content/items?status=active&limit=20\"")],
        }),
        json!({
            "title": "计划里声明投递目标",
            "body": "计划负责节律与目标信源；deliveryTargets 决定该计划的新内容投给谁（model、endpoint 或 channel）。",
            "commands": [format!(
                "curl -X POST {base}/collection/plans -H \"content-type: application/json\" -d \"{{\\\"name\\\":\\\"每日资讯\\\",\\\"targets\\\":[\\\"source/openai-news\\\"],\\\"trigger\\\":{{\\\"kind\\\":\\\"recurring\\\",\\\"interval\\\":\\\"2h\\\"}},\\\"deliveryTargets\\\":[{{\\\"kind\\\":\\\"model\\\",\\\"id\\\":\\\"model_xxx\\\"}}]}}\""
            )],
        }),
        json!({
            "title": "手动投递一批内容",
            "body": "非 GET 请求请带 idempotency-key；成功返回 202 与 operationId。",
            "commands": [format!(
                "curl -X POST {base}/delivery/jobs -H \"content-type: application/json\" -H \"idempotency-key: 3f6d2b\" -d \"{{\\\"contentIds\\\":[\\\"content_xxx\\\"],\\\"targets\\\":[{{\\\"kind\\\":\\\"endpoint\\\",\\\"id\\\":\\\"endpoint_xxx\\\"}}]}}\""
            )],
        }),
        json!({
            "title": "改配置带上 If-Match",
            "body": "先读 /settings/effective，从响应头取 ETag；写操作把它放进 If-Match，避免覆盖他人的改动。",
            "commands": [
                format!("curl -i {base}/settings/effective"),
                format!("curl -X POST {base}/settings/apply -H \"if-match: <etag>\" -H \"content-type: application/json\" -d \"{{\\\"changes\\\":{{\\\"delivery.retries\\\":3}},\\\"reason\\\":\\\"调整重试\\\"}}\""),
            ],
        }),
    ]
}

fn protocol_cards(base: &str, groups: Vec<Value>, total: usize) -> Vec<Value> {
    let origin = base.trim_end_matches("/api/v1").to_string();
    vec![
        json!({
            "id": "http",
            "name": "HTTP API",
            "status": "enabled",
            "note": "唯一对外领域契约；控制台、脚本与 SDK 都走它",
            "endpoint": base,
            "command": null,
            "examples": [
                format!("curl {base}/system/status"),
                format!("curl \"{base}/delivery/targets\""),
            ],
            "methods": total,
            "groups": groups,
            "items": [],
        }),
        json!({
            "id": "cli",
            "name": "命令行",
            "status": "enabled",
            "note": "与本机运行实例对话，不重复实现业务逻辑",
            "endpoint": null,
            "command": "modalprism-collector",
            "examples": [
                "modalprism-collector search 关键词",
                "modalprism-collector fetch https://example.com",
            ],
            "methods": cli_items().len(),
            "groups": [],
            "items": cli_items(),
        }),
        json!({
            "id": "mcp",
            "name": "MCP",
            "status": "disabled",
            "note": "只读进程（modalprism-agent）提供的工具面；工具名形如 ability.method，写侧只开可恢复白名单",
            "endpoint": null,
            "command": null,
            "examples": [],
            "methods": mcp_items().len(),
            "groups": [],
            "items": mcp_items(),
        }),
        json!({
            "id": "grpc",
            "name": "gRPC",
            "status": "disabled",
            "note": "框架提供 LeafGateway 服务，当前宿主未监听 gRPC 端口",
            "endpoint": null,
            "command": null,
            "examples": [],
            "methods": grpc_items().len(),
            "groups": [],
            "items": grpc_items(),
        }),
        json!({
            "id": "fun",
            "name": "进程内调用",
            "status": "internal",
            "note": "宿主内部的 Gateway，供能力代理由组合根直接调用；不开放端口",
            "endpoint": origin,
            "command": null,
            "examples": [],
            "methods": fun_items().len(),
            "groups": [],
            "items": fun_items(),
        }),
    ]
}

async fn list_protocols(State(state): State<Shared>) -> Result<Response, ApiError> {
    let base = format!("http://{}/api/v1", state.http_addr);
    let groups = http_groups();
    let total = endpoints::ENDPOINTS.len();
    Ok(util::ok(&json!({
        "generatedAt": now(),
        "baseUrl": base,
        "conventions": conventions(),
        "quickStart": quick_start(&base),
        "protocols": protocol_cards(&base, groups, total),
        "cli": cli_items(),
    })))
}

async fn api_docs() -> Result<Response, ApiError> {
    Ok(util::ok(&json!({
        "total": endpoints::ENDPOINTS.len(),
        "groups": http_groups(),
    })))
}
