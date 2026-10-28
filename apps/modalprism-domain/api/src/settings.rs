//! 系统设置、模型、提示词与 Skill。
//!
//! 设置是面向用户的有效值，不是配置文件的镜像。分组固定、字段可校验、
//! 应用创建新 generation 并写历史；密钥只允许引用名，不保存值。

use std::collections::BTreeMap;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiError;
use crate::observe::Refs;
use crate::operations::accepted;
use crate::util::{self, cursor, cursor_offset, page};
use crate::{AppState, Shared, new_id, now};

const SETTINGS_KIND: &str = "settings";
const SETTINGS_ID: &str = "effective";
const HISTORY_KIND: &str = "settings_history";

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/settings/schema", get(schema))
        .route("/settings/effective", get(effective))
        .route("/settings/validate", post(validate))
        .route("/settings/diff", post(diff))
        .route("/settings/apply", post(apply))
        .route("/settings/history", get(history))
        .route("/settings/rollback", post(rollback))
        .route("/settings/configStatus", get(config_status))
        .route("/settings/models", get(list_models).post(create_model))
        .route("/settings/prompts", get(list_prompts).post(create_prompt))
        .route(
            "/settings/prompts/{promptId}/versions",
            post(create_prompt_version),
        )
        .route("/settings/skills", get(list_skills).post(create_skill))
        .route(
            "/settings/skills/{skillId}/versions",
            post(create_skill_version),
        )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldType {
    Text,
    Integer,
    Number,
    Flag,
}

struct Field {
    key: &'static str,
    label: &'static str,
    group: &'static str,
    kind: FieldType,
    default: Value,
    minimum: Option<f64>,
    maximum: Option<f64>,
    options: &'static [&'static str],
    hot: bool,
    editable: bool,
    note: &'static str,
}

fn fields() -> Vec<Field> {
    vec![
        Field {
            key: "general.displayName",
            label: "实例名称",
            group: "general",
            kind: FieldType::Text,
            default: json!("ModalPrism"),
            minimum: None,
            maximum: None,
            options: &[],
            hot: true,
            editable: true,
            note: "显示在控制台与通知里的名称。",
        },
        Field {
            key: "collection.defaultIntervalMs",
            label: "默认采集间隔",
            group: "collection",
            kind: FieldType::Integer,
            default: json!(7_200_000),
            minimum: Some(60_000.0),
            maximum: Some(86_400_000.0),
            options: &[],
            hot: true,
            editable: true,
            note: "新建信源未单独指定节奏时使用。",
        },
        Field {
            key: "collection.minimumIntervalMs",
            label: "最小采集间隔",
            group: "collection",
            kind: FieldType::Integer,
            default: json!(1_800_000),
            minimum: Some(60_000.0),
            maximum: Some(86_400_000.0),
            options: &[],
            hot: true,
            editable: true,
            note: "任何信源的自适应节奏不会快于该值。",
        },
        Field {
            key: "collection.taskRetentionDays",
            label: "任务记录保留",
            group: "collection",
            kind: FieldType::Integer,
            default: json!(7),
            minimum: Some(1.0),
            maximum: Some(365.0),
            options: &[],
            hot: true,
            editable: true,
            note: "终态任务超过该天数后由保留策略清理。",
        },
        Field {
            key: "storage.exportTtlHours",
            label: "导出文件有效期",
            group: "storage",
            kind: FieldType::Integer,
            default: json!(24),
            minimum: Some(1.0),
            maximum: Some(720.0),
            options: &[],
            hot: true,
            editable: true,
            note: "日志与内容导出在到期后不可再下载。",
        },
        Field {
            key: "ai.defaultModelId",
            label: "默认模型",
            group: "ai",
            kind: FieldType::Text,
            default: json!(""),
            minimum: None,
            maximum: None,
            options: &[],
            hot: true,
            editable: true,
            note: "生产步骤未指定模型时使用；为空时生产会拒绝启动。",
        },
        Field {
            key: "ai.temperature",
            label: "默认温度",
            group: "ai",
            kind: FieldType::Number,
            default: json!(0.3),
            minimum: Some(0.0),
            maximum: Some(2.0),
            options: &[],
            hot: true,
            editable: true,
            note: "模型步骤未覆盖时使用。",
        },
        Field {
            key: "ai.maxTokens",
            label: "默认输出上限",
            group: "ai",
            kind: FieldType::Integer,
            default: json!(1200),
            minimum: Some(64.0),
            maximum: Some(32_000.0),
            options: &[],
            hot: true,
            editable: true,
            note: "单次模型调用的输出 token 上限。",
        },
        Field {
            key: "ai.requestTimeoutMs",
            label: "模型请求超时",
            group: "ai",
            kind: FieldType::Integer,
            default: json!(90_000),
            minimum: Some(1_000.0),
            maximum: Some(600_000.0),
            options: &[],
            hot: true,
            editable: true,
            note: "一次模型调用的最长等待时间。",
        },
        Field {
            key: "production.reviewRequired",
            label: "候选必须人工审核",
            group: "production",
            kind: FieldType::Flag,
            default: json!(true),
            minimum: None,
            maximum: None,
            options: &[],
            hot: true,
            editable: false,
            note: "本期固定为人工审核，生成不自动发布。",
        },
        Field {
            key: "production.deliveryRetries",
            label: "投递重试次数",
            group: "production",
            kind: FieldType::Integer,
            default: json!(2),
            minimum: Some(0.0),
            maximum: Some(10.0),
            options: &[],
            hot: true,
            editable: true,
            note: "单渠道失败后的自动重试次数；仍失败时进入出站页手动处理。",
        },
        Field {
            key: "outbound.robotsMode",
            label: "站点规则",
            group: "outbound",
            kind: FieldType::Text,
            default: json!("obey-with-exemptions"),
            minimum: None,
            maximum: None,
            options: &["obey", "obey-with-exemptions"],
            hot: false,
            editable: false,
            note: "由采集配置固定；修改需要重启与声明审核。",
        },
        Field {
            key: "outbound.requestPerSecond",
            label: "出站速率",
            group: "outbound",
            kind: FieldType::Integer,
            default: json!(1),
            minimum: Some(1.0),
            maximum: Some(20.0),
            options: &[],
            hot: false,
            editable: false,
            note: "出站限速由采集治理固定。",
        },
        Field {
            key: "retention.logDays",
            label: "普通日志保留",
            group: "retention",
            kind: FieldType::Integer,
            default: json!(14),
            minimum: Some(1.0),
            maximum: Some(365.0),
            options: &[],
            hot: true,
            editable: true,
            note: "超过该天数的普通日志由保留策略清理。",
        },
        Field {
            key: "retention.eventDays",
            label: "事件保留",
            group: "retention",
            kind: FieldType::Integer,
            default: json!(30),
            minimum: Some(1.0),
            maximum: Some(365.0),
            options: &[],
            hot: true,
            editable: true,
            note: "超过该天数的结构化事件由保留策略清理。",
        },
        Field {
            key: "retention.auditDays",
            label: "审计保留",
            group: "retention",
            kind: FieldType::Integer,
            default: json!(365),
            minimum: Some(30.0),
            maximum: Some(3650.0),
            options: &[],
            hot: true,
            editable: true,
            note: "超过该天数的审计事件由保留策略清理。",
        },
        Field {
            key: "retention.healthHistoryLimit",
            label: "健康历史条数",
            group: "retention",
            kind: FieldType::Integer,
            default: json!(500),
            minimum: Some(50.0),
            maximum: Some(5000.0),
            options: &[],
            hot: true,
            editable: true,
            note: "超出后只保留最近记录。",
        },
        Field {
            key: "maintenance.drainTimeoutMs",
            label: "维护排空超时",
            group: "maintenance",
            kind: FieldType::Integer,
            default: json!(30_000),
            minimum: Some(1_000.0),
            maximum: Some(600_000.0),
            options: &[],
            hot: true,
            editable: true,
            note: "进入维护模式后等待活动租约的最长时间，超时后协作取消。",
        },
        Field {
            key: "maintenance.resetTokenTtlMinutes",
            label: "确认令牌有效期",
            group: "maintenance",
            kind: FieldType::Integer,
            default: json!(10),
            minimum: Some(1.0),
            maximum: Some(120.0),
            options: &[],
            hot: true,
            editable: true,
            note: "清空预演确认令牌的短期有效期。",
        },
        Field {
            key: "search.defaultEngine",
            label: "默认搜索引擎",
            group: "search",
            kind: FieldType::Text,
            default: json!("ddg"),
            minimum: None,
            maximum: None,
            options: &["ddg", "bing", "google_cse", "serpapi"],
            hot: true,
            editable: true,
            note: "搜索与检索步骤未指定引擎时使用；密钥在下方按引擎引用。",
        },
        Field {
            key: "search.bingKeyRef",
            label: "Bing 密钥引用名",
            group: "search",
            kind: FieldType::Text,
            default: json!(""),
            minimum: None,
            maximum: None,
            options: &[],
            hot: true,
            editable: true,
            note: "密钥存放在环境或密钥库，这里只填引用名；用 Bing 引擎时必填。",
        },
        Field {
            key: "search.googleCseKeyRef",
            label: "Google CSE 密钥引用名",
            group: "search",
            kind: FieldType::Text,
            default: json!(""),
            minimum: None,
            maximum: None,
            options: &[],
            hot: true,
            editable: true,
            note: "同时需要 CSE ID 时以 ref 名逗号分隔：key,cx。",
        },
        Field {
            key: "search.serpApiKeyRef",
            label: "SerpAPI 密钥引用名",
            group: "search",
            kind: FieldType::Text,
            default: json!(""),
            minimum: None,
            maximum: None,
            options: &[],
            hot: true,
            editable: true,
            note: "用 SerpAPI 引擎时必填。",
        },
    ]
}

fn group_meta() -> Value {
    json!([
        {"id": "general", "label": "常规", "note": "实例名称。"},
        {"id": "collection", "label": "采集", "note": "新建信源的默认节奏、下限与任务保留。"},
        {"id": "storage", "label": "存储与导出", "note": "导出有效期。"},
        {"id": "ai", "label": "AI 与模型", "note": "生产默认模型、采样参数与请求超时。"},
        {"id": "production", "label": "生产与投递", "note": "审核门禁与投递重试。"},
        {"id": "outbound", "label": "出站与通知", "note": "站点规则与限速。固定项只读。"},
        {"id": "retention", "label": "保留策略", "note": "日志、事件、审计与健康历史。"},
        {"id": "maintenance", "label": "维护", "note": "排空超时与确认令牌有效期。"},
        {"id": "search", "label": "搜索", "note": "默认搜索引擎与各引擎密钥引用名。"}
    ])
}

fn schema_value(models: &[(String, String)]) -> Value {
    let fields: Vec<Value> = fields()
        .into_iter()
        .map(|field| {
            let dynamic = if field.key == "ai.defaultModelId" {
                let mut options = vec![json!({"value": "", "label": "不指定"})];
                for (id, name) in models {
                    options.push(json!({"value": id, "label": name}));
                }
                options
            } else {
                field
                    .options
                    .iter()
                    .map(|value| json!({"value": value, "label": option_label(field.key, value)}))
                    .collect()
            };
            let enum_type = field.key == "ai.defaultModelId"
                || field.options.iter().any(|value| !value.is_empty());
            let presets: Vec<Value> = presets_for(field.key)
                .iter()
                .map(|(value, label)| {
                    let number = if value.fract() == 0.0 {
                        json!(*value as i64)
                    } else {
                        json!(*value)
                    };
                    json!({"value": number, "label": label})
                })
                .collect();
            let mut item = json!({
                "key": field.key,
                "label": field.label,
                "group": field.group,
                "type": if enum_type {
                    "enum"
                } else {
                    match field.kind {
                        FieldType::Text => "string",
                        FieldType::Integer => "integer",
                        FieldType::Number => "number",
                        FieldType::Flag => "boolean",
                    }
                },
                "default": field.default,
                "options": dynamic,
                "presets": presets,
                "unit": unit_for(field.key),
                "editable": field.editable,
                "effect": if field.hot { "hotReload" } else { "restartRequired" },
                "description": field.note,
                "secret": false,
            });
            if let Some(minimum) = field.minimum {
                item["minimum"] = json!(minimum);
            }
            if let Some(maximum) = field.maximum {
                item["maximum"] = json!(maximum);
            }
            item
        })
        .collect();
    json!({"version": 2, "groups": group_meta(), "fields": fields})
}

/// 枚举值的业务文案；未登记时原样展示。
fn option_label(key: &str, value: &str) -> String {
    match (key, value) {
        ("outbound.robotsMode", "obey") => "严格遵守",
        ("outbound.robotsMode", "obey-with-exemptions") => "遵守但允许例外",
        _ => value,
    }
    .to_string()
}

/// 数值字段的常用档位，避免用户自己换算单位。
fn presets_for(key: &str) -> &'static [(f64, &'static str)] {
    match key {
        "collection.defaultIntervalMs" => &[
            (1_800_000.0, "30 分钟"),
            (3_600_000.0, "1 小时"),
            (21_600_000.0, "6 小时"),
            (43_200_000.0, "12 小时"),
            (86_400_000.0, "24 小时"),
        ],
        "collection.minimumIntervalMs" => &[
            (900_000.0, "15 分钟"),
            (1_800_000.0, "30 分钟"),
            (3_600_000.0, "1 小时"),
            (7_200_000.0, "2 小时"),
        ],
        "collection.taskRetentionDays" => &[
            (1.0, "1 天"),
            (7.0, "7 天"),
            (30.0, "30 天"),
            (90.0, "90 天"),
        ],
        "storage.exportTtlHours" => &[
            (1.0, "1 小时"),
            (6.0, "6 小时"),
            (24.0, "24 小时"),
            (72.0, "3 天"),
            (168.0, "7 天"),
        ],
        "ai.temperature" => &[
            (0.0, "0 确定"),
            (0.2, "0.2 稳健"),
            (0.3, "0.3 默认"),
            (0.7, "0.7 发散"),
            (1.0, "1.0 自由"),
        ],
        "ai.maxTokens" => &[
            (512.0, "512"),
            (1200.0, "1200"),
            (2000.0, "2000"),
            (4000.0, "4000"),
        ],
        "ai.requestTimeoutMs" => &[
            (30_000.0, "30 秒"),
            (60_000.0, "60 秒"),
            (90_000.0, "90 秒"),
            (180_000.0, "180 秒"),
        ],
        "production.deliveryRetries" => &[
            (0.0, "不重试"),
            (1.0, "1 次"),
            (2.0, "2 次"),
            (3.0, "3 次"),
            (5.0, "5 次"),
        ],
        "retention.logDays" => &[
            (7.0, "7 天"),
            (14.0, "14 天"),
            (30.0, "30 天"),
            (90.0, "90 天"),
            (365.0, "365 天"),
        ],
        "retention.eventDays" => &[
            (7.0, "7 天"),
            (30.0, "30 天"),
            (90.0, "90 天"),
            (180.0, "180 天"),
        ],
        "retention.auditDays" => &[
            (90.0, "90 天"),
            (180.0, "180 天"),
            (365.0, "365 天"),
            (730.0, "730 天"),
        ],
        "retention.healthHistoryLimit" => &[
            (100.0, "100 条"),
            (500.0, "500 条"),
            (1000.0, "1000 条"),
            (2000.0, "2000 条"),
        ],
        "maintenance.drainTimeoutMs" => &[
            (10_000.0, "10 秒"),
            (30_000.0, "30 秒"),
            (60_000.0, "60 秒"),
            (120_000.0, "120 秒"),
        ],
        "maintenance.resetTokenTtlMinutes" => &[
            (5.0, "5 分钟"),
            (10.0, "10 分钟"),
            (30.0, "30 分钟"),
            (60.0, "60 分钟"),
        ],
        _ => &[],
    }
}

fn unit_for(key: &str) -> &'static str {
    if key.ends_with("Ms") {
        "毫秒"
    } else if key.ends_with("Days") {
        "天"
    } else if key.ends_with("Hours") {
        "小时"
    } else if key.ends_with("Minutes") {
        "分钟"
    } else if key.ends_with("Tokens") {
        "token"
    } else if key.ends_with("Limit") {
        "条"
    } else {
        ""
    }
}

pub(crate) async fn seed(state: &AppState) -> Result<(), ApiError> {
    if state
        .store
        .get_document(SETTINGS_KIND, SETTINGS_ID)
        .await?
        .is_none()
    {
        let values = defaults();
        let doc = json!({
            "generation": 1,
            "values": values,
            "appliedAt": now(),
        });
        state
            .store
            .insert_document(SETTINGS_KIND, SETTINGS_ID, &doc)
            .await?;
        state
            .store
            .insert_document(
                HISTORY_KIND,
                "g1",
                &json!({
                    "generation": 1,
                    "values": doc["values"],
                    "summary": "默认设置",
                    "appliedAt": doc["appliedAt"],
                    "rollbackOf": Value::Null,
                }),
            )
            .await?;
    }
    seed_named(
        state,
        "prompt",
        "prompt-source-analysis",
        "信源分析",
        "分析目标地址的可采集结构，输出信源草稿与风险说明。",
        "你是采集工程师。阅读输入的目标地址与用户说明，判断可用的接入方式、适配器与节奏。只输出 JSON：{\"name\":\"\",\"accessMethodId\":\"\",\"adapterIds\":[],\"endpoint\":\"\",\"intervalMs\":0,\"rationale\":\"\",\"risks\":[]}。不得编造不存在的访问方式。",
    )
    .await?;
    seed_named(
        state,
        "prompt",
        "prompt-adapter-draft",
        "适配器草拟",
        "根据页面结构与字段要求生成声明式适配器草稿。",
        "你是解析器工程师。根据给定的站点结构与字段要求，输出声明式适配器草稿 JSON。草稿必须通过静态检查与配置校验，不能包含脚本执行、内网地址或凭据。",
    )
    .await?;
    seed_named(
        state,
        "prompt",
        "prompt-production",
        "内容生产",
        "把冻结输入写成可直接审核的候选内容。",
        "你是内容编辑。严格依据输入材料写作，不补充无法从材料核验的事实。输出正文；如需结构，按步骤参数里的 schema 输出 JSON。",
    )
    .await?;
    seed_named(
        state,
        "skill",
        "skill-source-analysis",
        "信源分析",
        "把目标地址与用户意图转成可复核的信源草稿。",
        "输入约束：地址必须是 http/https；输出必须是 JSON。允许工具：无。敏感等级：低。",
    )
    .await?;
    seed_named(
        state,
        "skill",
        "skill-adapter-generation",
        "适配器生成",
        "生成声明式适配器草稿，批准前不得启用。",
        "输入约束：站点结构说明与字段要求；输出约束：声明式 JSON。允许工具：受限探测。敏感等级：中。",
    )
    .await?;
    seed_named(
        state,
        "skill",
        "skill-content-generation",
        "内容生成",
        "从一个或多个冻结输入生成候选，不自动发布。",
        "输入约束：只读冻结输入；输出约束：正文或 schema 校验通过的 JSON。允许工具：无。敏感等级：中。",
    )
    .await?;
    seed_named(
        state,
        "skill",
        "skill-fault-explanation",
        "故障解释",
        "依据日志与健康历史解释失败原因，不修改状态。",
        "输入约束：脱敏后的日志与健康记录；输出约束：原因、影响范围与下一步。允许工具：只读日志。敏感等级：低。",
    )
    .await?;
    Ok(())
}

async fn seed_named(
    state: &AppState,
    kind: &str,
    id: &str,
    name: &str,
    description: &str,
    content: &str,
) -> Result<(), ApiError> {
    if state.store.get_document(kind, id).await?.is_some() {
        return Ok(());
    }
    let version_id = format!("{id}:v1");
    let doc = json!({
        "id": id,
        "name": name,
        "description": description,
        "builtin": true,
        "latestVersionId": version_id,
        "versions": [{
            "id": version_id,
            "version": 1,
            "content": content,
            "createdAt": now(),
        }],
        "createdAt": now(),
        "updatedAt": now(),
    });
    state.store.insert_document(kind, id, &doc).await?;
    Ok(())
}

fn defaults() -> Map<String, Value> {
    let mut values = Map::new();
    for field in fields() {
        values.insert(field.key.to_string(), field.default);
    }
    values
}

async fn settings_doc(state: &AppState) -> Result<(Value, i64), ApiError> {
    let doc = state
        .store
        .get_document(SETTINGS_KIND, SETTINGS_ID)
        .await?
        .ok_or_else(|| ApiError::internal("设置尚未初始化"))?;
    Ok((doc.body, doc.version))
}

pub async fn effective_values(state: &AppState) -> Result<Value, ApiError> {
    let (doc, _) = settings_doc(state).await?;
    let known = field_map();
    let mut values = doc["values"].clone();
    if let Some(map) = values.as_object_mut() {
        map.retain(|key, _| known.contains_key(key.as_str()));
    }
    Ok(values)
}

pub async fn value(state: &AppState, key: &str) -> Result<Value, ApiError> {
    let values = effective_values(state).await?;
    Ok(values.get(key).cloned().unwrap_or(Value::Null))
}

async fn schema(State(state): State<Shared>) -> Result<Response, ApiError> {
    let mut models = Vec::new();
    for doc in state.store.list_documents("model").await? {
        let name = doc
            .body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&doc.id)
            .to_string();
        models.push((doc.id, name));
    }
    models.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(util::ok(&schema_value(&models)))
}

async fn effective(State(state): State<Shared>) -> Result<Response, ApiError> {
    let (doc, version) = settings_doc(&state).await?;
    let tag = format!("\"g{}\"", doc["generation"].as_i64().unwrap_or(1));
    let known = field_map();
    let mut doc = doc;
    if let Some(map) = doc["values"].as_object_mut() {
        map.retain(|key, _| known.contains_key(key.as_str()));
    }
    let mut response = util::ok(&doc);
    if let Ok(value) = HeaderValue::from_str(&tag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    let _ = version;
    Ok(response)
}

/// 热加载健康（R10.1 / H3）。
///
/// 整库 reload 遇到任一错误会保留旧快照并把错误记在 `last_error`。控制台必须能显式
/// 看到它——否则一份畸形 binding 会让此后所有配置编辑持续重载失败，系统卡在陈旧快照而
/// 无人察觉。这里只读暴露最近一次 reload 的诊断与待重启项，不含内部物理路径。
async fn config_status(State(state): State<Shared>) -> Result<Response, ApiError> {
    let store = state.services.config();
    let reload_error: Vec<Value> = store
        .last_error()
        .map(|diagnostics| {
            diagnostics
                .0
                .iter()
                .map(|item| json!({"file": item.file, "line": item.line, "message": item.message}))
                .collect()
        })
        .unwrap_or_default();
    let pending_restart: Vec<Value> = store
        .pending_restart()
        .into_iter()
        .map(|change| json!({"path": change.path}))
        .collect();
    Ok(util::ok(&json!({
        "staleSnapshot": !reload_error.is_empty(),
        "reloadError": reload_error,
        "pendingRestart": pending_restart,
        "generation": state.services.config_generation(),
    })))
}

fn field_map() -> BTreeMap<&'static str, Field> {
    fields()
        .into_iter()
        .map(|field| (field.key, field))
        .collect()
}

fn check_changes(changes: &Map<String, Value>) -> Result<BTreeMap<String, Value>, ApiError> {
    let known = field_map();
    let mut out = BTreeMap::new();
    for (key, value) in changes {
        let Some(field) = known.get(key.as_str()) else {
            return Err(ApiError::unknown_field(key));
        };
        if !field.editable {
            return Err(ApiError::validation(key, "readOnly", "该字段不可修改"));
        }
        match field.kind {
            FieldType::Text => {
                if !value.is_string() {
                    return Err(ApiError::validation(key, "type", "必须是字符串"));
                }
                let text = value.as_str().unwrap_or_default();
                if !field.options.is_empty() && !text.is_empty() && !field.options.contains(&text) {
                    return Err(ApiError::validation(key, "enum", "值不在候选列表中"));
                }
            }
            FieldType::Integer => {
                let Some(number) = value.as_i64() else {
                    return Err(ApiError::validation(key, "type", "必须是整数"));
                };
                let number = number as f64;
                if field.minimum.is_some_and(|min| number < min)
                    || field.maximum.is_some_and(|max| number > max)
                {
                    return Err(ApiError::validation(key, "outOfRange", "超出允许范围"));
                }
            }
            FieldType::Number => {
                let Some(number) = value.as_f64() else {
                    return Err(ApiError::validation(key, "type", "必须是数字"));
                };
                if field.minimum.is_some_and(|min| number < min)
                    || field.maximum.is_some_and(|max| number > max)
                {
                    return Err(ApiError::validation(key, "outOfRange", "超出允许范围"));
                }
            }
            FieldType::Flag => {
                if !value.is_boolean() {
                    return Err(ApiError::validation(key, "type", "必须是布尔值"));
                }
            }
        }
        out.insert(key.clone(), value.clone());
    }
    Ok(out)
}

fn changes_of(body: &Value) -> Result<(Map<String, Value>, Option<String>), ApiError> {
    let map = util::object(body)?;
    util::unknown(map, &["changes", "reason"])?;
    let changes = util::optional_object(map, "changes")?
        .ok_or_else(|| ApiError::validation("changes", "required", "不能为空"))?;
    let reason = util::optional_text(map, "reason")?;
    Ok((changes, reason))
}

async fn validate(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let (changes, _) = changes_of(&body)?;
    let checked = check_changes(&changes)?;
    let (doc, _) = settings_doc(&state).await?;
    let current = doc["values"].as_object().cloned().unwrap_or_default();
    let mut next = current.clone();
    for (key, value) in &checked {
        next.insert(key.clone(), value.clone());
    }
    let mut field_errors = Vec::new();
    for (key, value) in &checked {
        if key == "ai.defaultModelId"
            && let Some(id) = value.as_str().filter(|value| !value.is_empty())
            && state.store.get_document("model", id).await?.is_none()
        {
            field_errors.push(json!({
                "field": key,
                "code": "unknownReference",
                "message": format!("模型不存在: {id}")
            }));
        }
    }
    Ok(util::ok(&json!({
        "valid": field_errors.is_empty(),
        "fieldErrors": field_errors,
        "changes": checked,
        "nextGeneration": doc["generation"].as_i64().unwrap_or(1) + 1,
    })))
}

async fn diff(State(state): State<Shared>, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let (changes, _) = changes_of(&body)?;
    let checked = check_changes(&changes)?;
    let (doc, _) = settings_doc(&state).await?;
    let current = doc["values"].as_object().cloned().unwrap_or_default();
    let known = field_map();
    let mut items = Vec::new();
    for (key, value) in &checked {
        let old = current.get(key).cloned().unwrap_or(Value::Null);
        if &old == value {
            continue;
        }
        let field = known.get(key.as_str());
        items.push(json!({
            "key": key,
            "label": field.map(|field| field.label).unwrap_or(key),
            "oldValue": old,
            "newValue": value,
            "effect": if field.is_some_and(|field| field.hot) { "hotReload" } else { "restartRequired" },
            "risk": if key.starts_with("maintenance.") || key.starts_with("retention.") { "high" } else { "normal" },
        }));
    }
    let restart: Vec<&str> = checked
        .keys()
        .filter(|key| known.get(key.as_str()).is_some_and(|field| !field.hot))
        .map(String::as_str)
        .collect();
    Ok(util::ok(&json!({
        "generation": doc["generation"],
        "changes": items,
        "hotReload": checked.keys().filter(|key| known.get(key.as_str()).is_some_and(|field| field.hot)).collect::<Vec<_>>(),
        "restartRequired": restart,
    })))
}

async fn apply(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let (changes, reason) = changes_of(&body)?;
    let checked = check_changes(&changes)?;
    let (doc, _) = settings_doc(&state).await?;
    let generation = doc["generation"].as_i64().unwrap_or(1);
    util::if_match(&headers, &format!("g{generation}"))?;
    let operation = crate::operations::start(
        &state,
        "settings.apply",
        vec![json!({"kind": "settings", "id": SETTINGS_ID})],
        json!({
            "kind": "settings.apply",
            "changes": checked,
            "reason": reason,
            "expectedGeneration": generation,
        }),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_apply(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let changes = action["changes"].as_object().cloned().unwrap_or_default();
    let checked = check_changes(&changes)?;
    let (doc, version) = settings_doc(state)
        .await
        .unwrap_or((json!({"generation":1,"values":{}}), 1));
    let expected = action["expectedGeneration"].as_i64().unwrap_or(1);
    let current = doc["generation"].as_i64().unwrap_or(1);
    if current != expected {
        return Err(ApiError::precondition("设置已被其他会话修改"));
    }
    let mut values = doc["values"].as_object().cloned().unwrap_or_default();
    let mut applied = Vec::new();
    for (key, value) in &checked {
        values.insert(key.clone(), value.clone());
        applied.push(key.clone());
    }
    let next_generation = current + 1;
    let next = json!({
        "generation": next_generation,
        "values": values,
        "appliedAt": now(),
    });
    state
        .store
        .update_document(SETTINGS_KIND, SETTINGS_ID, &next, Some(version))
        .await?;
    state
        .store
        .insert_document(
            HISTORY_KIND,
            &format!("g{next_generation}"),
            &json!({
                "generation": next_generation,
                "values": next["values"],
                "summary": action["reason"].as_str().unwrap_or("应用设置"),
                "appliedAt": next["appliedAt"],
                "rollbackOf": action.get("retryOf").cloned().unwrap_or(Value::Null),
            }),
        )
        .await?;
    let restart: Vec<String> = applied
        .iter()
        .filter(|key| {
            field_map()
                .get(key.as_str())
                .is_some_and(|field| !field.hot)
        })
        .cloned()
        .collect();
    state
        .retention_at
        .store(0, std::sync::atomic::Ordering::Relaxed);
    crate::observe::append_audit(
        state,
        "settings.apply",
        "succeeded",
        "应用系统设置",
        json!({"generation": next_generation, "changes": applied, "restartRequired": restart}),
        Some(Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({
        "partial": false,
        "generation": next_generation,
        "applied": applied,
        "restartRequired": restart,
        "status": if restart.is_empty() { "applied" } else { "restartRequired" },
    }))
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

async fn history(
    State(state): State<Shared>,
    Query(query): Query<PageQuery>,
) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = "settings-history";
    let offset = cursor_offset(query.cursor.as_deref(), filter)?;
    let docs = state.store.list_documents(HISTORY_KIND).await?;
    let mut docs: Vec<Value> = docs.into_iter().map(|doc| doc.body).collect();
    docs.sort_by(|left, right| {
        right["generation"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&left["generation"].as_i64().unwrap_or(0))
    });
    let total = docs.len();
    let more = offset + limit < total;
    let items: Vec<Value> = docs.into_iter().skip(offset).take(limit).collect();
    let next = if more {
        Some(cursor(filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn rollback(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["targetGeneration", "reason"])?;
    let target = util::integer(map, "targetGeneration")?;
    let reason = util::non_empty(map, "reason")?;
    let (doc, _) = settings_doc(&state).await?;
    let generation = doc["generation"].as_i64().unwrap_or(1);
    util::if_match(&headers, &format!("g{generation}"))?;
    state
        .store
        .get_document(HISTORY_KIND, &format!("g{target}"))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("generation {target}")))?;
    let operation = crate::operations::start(
        &state,
        "settings.rollback",
        vec![json!({"kind": "settings", "id": SETTINGS_ID})],
        json!({
            "kind": "settings.rollback",
            "targetGeneration": target,
            "reason": reason,
            "expectedGeneration": generation,
        }),
    )
    .await?;
    Ok(accepted(&operation))
}

pub async fn execute_rollback(
    state: &AppState,
    operation_id: &str,
    action: &Value,
) -> Result<Value, ApiError> {
    let target = action["targetGeneration"].as_i64().unwrap_or(1);
    let (doc, version) = settings_doc(state).await?;
    let expected = action["expectedGeneration"].as_i64().unwrap_or(1);
    let current = doc["generation"].as_i64().unwrap_or(1);
    if current != expected {
        return Err(ApiError::precondition("设置已被其他会话修改"));
    }
    let history = state
        .store
        .get_document(HISTORY_KIND, &format!("g{target}"))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("generation {target}")))?;
    let next_generation = current + 1;
    let next = json!({
        "generation": next_generation,
        "values": history.body["values"],
        "appliedAt": now(),
    });
    state
        .store
        .update_document(SETTINGS_KIND, SETTINGS_ID, &next, Some(version))
        .await?;
    state
        .store
        .insert_document(
            HISTORY_KIND,
            &format!("g{next_generation}"),
            &json!({
                "generation": next_generation,
                "values": next["values"],
                "summary": format!("回滚到 g{target}: {}", action["reason"].as_str().unwrap_or("")),
                "appliedAt": next["appliedAt"],
                "rollbackOf": target,
            }),
        )
        .await?;
    state
        .retention_at
        .store(0, std::sync::atomic::Ordering::Relaxed);
    crate::observe::append_audit(
        state,
        "settings.rollback",
        "succeeded",
        "回滚系统设置",
        json!({"from": current, "to": next_generation, "target": target}),
        Some(Refs::op(operation_id)),
    )
    .await?;
    Ok(json!({
        "partial": false,
        "generation": next_generation,
        "rollbackOf": target,
        "status": "applied",
    }))
}

async fn list_models(
    State(state): State<Shared>,
    Query(query): Query<PageQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, "model", query).await
}

async fn list_prompts(
    State(state): State<Shared>,
    Query(query): Query<PageQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, "prompt", query).await
}

async fn list_skills(
    State(state): State<Shared>,
    Query(query): Query<PageQuery>,
) -> Result<Response, ApiError> {
    list_docs(&state, "skill", query).await
}

async fn list_docs(state: &AppState, kind: &str, query: PageQuery) -> Result<Response, ApiError> {
    let limit = util::limit(query.limit.as_deref())?;
    let filter = format!("list:{kind}");
    let offset = cursor_offset(query.cursor.as_deref(), &filter)?;
    let docs = state.store.list_documents(kind).await?;
    let total = docs.len();
    let items: Vec<Value> = docs
        .into_iter()
        .skip(offset)
        .take(limit + 1)
        .map(|doc| {
            let mut body = doc.body;
            if kind == "model" {
                body = model_view(&body);
            }
            if let Some(map) = body.as_object_mut()
                && (kind == "prompt" || kind == "skill")
            {
                map.insert(
                    "etag".into(),
                    Value::String(format!("{kind}-{}-v{}", doc.id, doc.version)),
                );
            }
            body
        })
        .collect();
    let more = items.len() > limit;
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    let next = if more || offset + items.len() < total {
        Some(cursor(&filter, offset + limit))
    } else {
        None
    };
    Ok(util::ok(&page(items, next)))
}

async fn create_model(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "name",
            "kind",
            "endpoint",
            "secretRef",
            "enabled",
            "model",
            "timeoutMs",
            "maxTokens",
            "temperature",
            "headers",
        ],
    )?;
    let name = util::non_empty(map, "name")?;
    let kind = util::text(map, "kind")?;
    if !matches!(kind.as_str(), "remoteApi" | "localService" | "localCli") {
        return Err(ApiError::validation(
            "kind",
            "enum",
            "只支持 remoteApi、localService 或 localCli",
        ));
    }
    let endpoint = util::non_empty(map, "endpoint")?;
    let secret_ref = util::optional_text(map, "secretRef")?;
    if let Some(name) = secret_ref.as_deref() {
        if name.len() > 100
            || !name
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        {
            return Err(ApiError::validation(
                "secretRef",
                "format",
                "只接受环境变量名（大写字母、数字与下划线），不要填写密钥值",
            ));
        }
    }
    let enabled = util::flag(map, "enabled")?;
    if let Some(headers) = map.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            if !value.is_string() {
                return Err(ApiError::validation(
                    "headers",
                    "type",
                    "请求头值必须是字符串",
                ));
            }
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "authorization" | "cookie" | "proxy-authorization" | "x-api-key" | "api-key"
            ) {
                return Err(ApiError::validation(
                    "headers",
                    "sensitive",
                    "敏感凭据请使用 secretRef 环境变量引用",
                ));
            }
        }
    }
    if kind != "localCli" {
        let parsed = url::Url::parse(&endpoint)
            .map_err(|_| ApiError::validation("endpoint", "format", "不是有效地址"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ApiError::validation(
                "endpoint",
                "scheme",
                "只接受 http/https",
            ));
        }
    }
    let id = new_id("model");
    let doc = json!({
        "id": id,
        "name": name,
        "kind": kind,
        "endpoint": endpoint,
        "secretRef": secret_ref,
        "secretPresent": secret_ref
            .as_deref()
            .is_some_and(|name| std::env::var(name).is_ok()),
        "enabled": enabled,
        "model": map.get("model").cloned().unwrap_or(Value::String("default".into())),
        "timeoutMs": map.get("timeoutMs").cloned().unwrap_or(json!(90_000)),
        "maxTokens": map.get("maxTokens").cloned().unwrap_or(json!(1200)),
        "temperature": map.get("temperature").cloned().unwrap_or(json!(0.3)),
        "headers": map.get("headers").cloned().unwrap_or(json!({})),
        "createdAt": now(),
    });
    state.store.insert_document("model", &id, &doc).await?;
    crate::observe::append_audit(
        &state,
        "settings.model.create",
        "succeeded",
        "登记模型资源",
        json!({"id": id, "kind": kind}),
        None,
    )
    .await?;
    Ok(util::created(&model_view(&doc)))
}

/// 模型请求头只回传名称：值可能是用户误填的凭据，不进入列表、差异或导出。
fn model_view(doc: &Value) -> Value {
    let mut view = doc.clone();
    if let Some(map) = view.as_object_mut() {
        let names: Vec<String> = map
            .remove("headers")
            .and_then(|value| value.as_object().cloned())
            .map(|headers| headers.keys().cloned().collect())
            .unwrap_or_default();
        map.insert("headerNames".into(), json!(names));
    }
    view
}

async fn create_prompt(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    create_named(&state, "prompt", body, "settings.prompt.create").await
}

async fn create_skill(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    create_named(&state, "skill", body, "settings.skill.create").await
}

async fn create_named(
    state: &AppState,
    kind: &str,
    body: Value,
    audit: &str,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "content", "description"])?;
    let name = util::non_empty(map, "name")?;
    let content = util::non_empty(map, "content")?;
    let description = util::optional_text(map, "description")?.unwrap_or_default();
    let id = new_id(kind);
    let version = format!("{id}:v1");
    let doc = json!({
        "id": id,
        "name": name,
        "description": description,
        "builtin": false,
        "latestVersionId": version,
        "versions": [{"id": version, "version": 1, "content": content, "createdAt": now()}],
        "createdAt": now(),
        "updatedAt": now(),
    });
    state.store.insert_document(kind, &id, &doc).await?;
    crate::observe::append_audit(
        state,
        audit,
        "succeeded",
        "创建版本化资源",
        json!({"id": id}),
        None,
    )
    .await?;
    Ok(util::created(&doc))
}

async fn create_prompt_version(
    State(state): State<Shared>,
    Path(prompt_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    create_version(&state, "prompt", &prompt_id, headers, body).await
}

async fn create_skill_version(
    State(state): State<Shared>,
    Path(skill_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    create_version(&state, "skill", &skill_id, headers, body).await
}

async fn create_version(
    state: &AppState,
    kind: &str,
    id: &str,
    headers: HeaderMap,
    body: Value,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(map, &["name", "content", "description"])?;
    let content = util::non_empty(map, "content")?;
    let doc = state
        .store
        .get_document(kind, id)
        .await?
        .ok_or_else(|| ApiError::not_found(id))?;
    util::if_match(&headers, &format!("{kind}-{id}-v{}", doc.version))?;
    let mut value = doc.body;
    let versions = value["versions"].as_array().cloned().unwrap_or_default();
    let next = versions.len() + 1;
    let version_id = format!("{id}:v{next}");
    let entry = json!({"id": version_id, "version": next, "content": content, "createdAt": now()});
    if let Some(list) = value["versions"].as_array_mut() {
        list.push(entry);
    }
    value["latestVersionId"] = Value::String(version_id.clone());
    value["updatedAt"] = Value::String(now());
    if let Some(name) = map.get("name").and_then(Value::as_str) {
        value["name"] = Value::String(name.to_string());
    }
    if let Some(description) = map.get("description").and_then(Value::as_str) {
        value["description"] = Value::String(description.to_string());
    }
    state
        .store
        .update_document(kind, id, &value, Some(doc.version))
        .await?;
    crate::observe::append_audit(
        state,
        &format!("settings.{kind}.version"),
        "succeeded",
        "创建新版本",
        json!({"id": id, "version": version_id}),
        None,
    )
    .await?;
    Ok(util::created(&json!({
        "id": version_id,
        "resourceId": id,
        "version": next,
        "content": content,
        "createdAt": now(),
    })))
}

/// 生产执行时解析提示词版本内容。
pub async fn prompt_content(state: &AppState, version_id: &str) -> Result<String, ApiError> {
    resolve_named(state, "prompt", version_id).await
}

pub async fn skill_content(state: &AppState, version_id: &str) -> Result<String, ApiError> {
    resolve_named(state, "skill", version_id).await
}

async fn resolve_named(state: &AppState, kind: &str, version_id: &str) -> Result<String, ApiError> {
    let (id, _) = version_id
        .split_once(":v")
        .ok_or_else(|| ApiError::validation("versionId", "format", "版本标识无效"))?;
    let doc = state
        .store
        .get_document(kind, id)
        .await?
        .ok_or_else(|| ApiError::not_found(version_id))?;
    let versions = doc.body["versions"].as_array().cloned().unwrap_or_default();
    versions
        .iter()
        .find(|item| item["id"] == version_id)
        .and_then(|item| item["content"].as_str())
        .map(str::to_string)
        .ok_or_else(|| ApiError::not_found(version_id))
}

pub async fn model(state: &AppState, id: &str) -> Result<Value, ApiError> {
    state
        .store
        .get_document("model", id)
        .await?
        .map(|doc| doc.body)
        .ok_or_else(|| ApiError::not_found(format!("模型 {id}")))
}

pub async fn default_model(state: &AppState) -> Result<Option<Value>, ApiError> {
    let id = value(state, "ai.defaultModelId").await?;
    let Some(id) = id.as_str().filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    Ok(state
        .store
        .get_document("model", id)
        .await?
        .map(|doc| doc.body))
}
