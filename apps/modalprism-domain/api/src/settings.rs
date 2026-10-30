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
use axum::routing::{get, patch, post};
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
        .route("/settings/models/test", post(test_model))
        .route("/settings/models/discover", post(discover_models))
        .route("/settings/models/cliCandidates", get(cli_candidates))
        .route(
            "/settings/models/{modelId}",
            patch(update_model).delete(delete_model),
        )
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
            note: "模型资源未声明温度时使用；已声明的按模型自己的缺省值执行。",
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
            note: "模型资源未声明输出上限时使用；已声明的按模型自己的缺省值执行。",
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
            key: "delivery.retries",
            label: "投递重试次数",
            group: "delivery",
            kind: FieldType::Integer,
            default: json!(2),
            minimum: Some(0.0),
            maximum: Some(10.0),
            options: &[],
            hot: true,
            editable: true,
            note: "单个目标失败后的自动重试次数；仍失败时在投递记录里逐目标标记。",
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
        {"id": "ai", "label": "AI 与模型", "note": "投递默认模型、采样参数与请求超时。"},
        {"id": "delivery", "label": "投递", "note": "投递失败重试次数。"},
        {"id": "outbound", "label": "抓取限制", "note": "站点规则与限速。固定项只读。"},
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
        "delivery.retries" => &[
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
        "skill-fault-explanation",
        "故障解释",
        "依据日志与健康历史解释失败原因，不修改状态。",
        "输入约束：脱敏后的日志与健康记录；输出约束：原因、影响范围与下一步。允许工具：只读日志。敏感等级：低。",
    )
    .await?;
    seed_models(state).await?;
    Ok(())
}

/// 模型声明写在 `config/models/*.yaml`：启动时读入存储，界面只做编辑与试调。
///
/// 已有同名模型时跳过，避免用旧声明覆盖界面上的改动；删除后下次启动会重新读入。
async fn seed_models(state: &AppState) -> Result<(), ApiError> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(state.config_dir.join("models"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()).is_some_and(is_yaml))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_yaml::from_str::<Value>(&text) else {
            continue;
        };
        let file = path.file_name().and_then(|name| name.to_str()).unwrap_or("model.yaml");
        let Ok(spec) = workflow::ModelSpec::parse(file, &value) else {
            continue;
        };
        let slug = spec
            .id
            .as_deref()
            .and_then(|id| id.rsplit('/').next())
            .filter(|slug| !slug.is_empty())
            .map(str::to_string)
            .or_else(|| path.file_stem().and_then(|stem| stem.to_str()).map(str::to_string))
            .unwrap_or_else(|| "model".into());
        let id = format!("model/{slug}");
        if state.store.get_document("model", &id).await?.is_some() {
            continue;
        }
        let (kind, endpoint) = match spec.mode {
            workflow::Mode::Api => ("remoteApi", spec.endpoint.clone().unwrap_or_default()),
            workflow::Mode::Local => ("localService", spec.endpoint.clone().unwrap_or_default()),
            workflow::Mode::Cli => ("localCli", spec.command.join(" ")),
        };
        let timeout_ms = spec
            .timeout
            .as_deref()
            .and_then(config::duration)
            .map(|span| span.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(90_000);
        let doc = json!({
            "id": id,
            "name": spec.name.clone().unwrap_or_else(|| slug.clone()),
            "kind": kind,
            "endpoint": endpoint,
            "secretRef": spec.key_env,
            "secretValue": Value::Null,
            "secretPresent": spec.key_env.is_some(),
            "enabled": true,
            "model": spec.model.clone().unwrap_or_else(|| "default".into()),
            "timeoutMs": timeout_ms,
            "maxTokens": spec.max_tokens.unwrap_or(1200),
            "temperature": spec.temperature.unwrap_or(0.3),
            "headers": Value::Object(
                spec.headers
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect()
            ),
            "builtin": true,
            "createdAt": now(),
        });
        state.store.insert_document("model", &id, &doc).await?;
    }
    Ok(())
}

fn is_yaml(ext: &str) -> bool {
    matches!(ext, "yaml" | "yml")
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

/// 读取设置文档；缺失时按内置默认值重建后再读。
///
/// 设置是默认值的投影，每个读接口都依赖它。清空或恢复出厂中途失败会把它删掉，
/// 此时重建比把「设置尚未初始化」扩散到整站更安全。
async fn settings_doc(state: &AppState) -> Result<(Value, i64), ApiError> {
    if let Some(doc) = state
        .store
        .get_document(SETTINGS_KIND, SETTINGS_ID)
        .await?
    {
        return Ok((doc.body, doc.version));
    }
    seed(state).await?;
    let doc = state
        .store
        .get_document(SETTINGS_KIND, SETTINGS_ID)
        .await?
        .ok_or_else(|| ApiError::internal("设置无法初始化"))?;
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
    // 被跳过的非声明文件：不影响快照，但要让控制台能说清「这些文件没有生效」。
    let warnings: Vec<Value> = store
        .last_warnings()
        .iter()
        .map(|item| json!({"file": item.file, "line": item.line, "message": item.message}))
        .collect();
    // 已下线但还没被退役清理移走的目录（通常只在下一次启动前出现）。
    let retired: Vec<Value> = config::retired_paths()
        .into_iter()
        .filter(|name| state.config_dir.join(name).exists())
        .map(|name| json!({"path": name, "since": config::retired_since(name)}))
        .collect();
    Ok(util::ok(&json!({
        "staleSnapshot": !reload_error.is_empty(),
        "reloadError": reload_error,
        "warnings": warnings,
        "retired": retired,
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

/// 模型可声明的输入模态（闭集）：与前端预设同源，后端只做校验与落库。
const MODALITIES: &[&str] = &["text", "image", "audio", "video"];

/// 模型能力缺省：未显式声明时按通用保守值落库，避免下游拿到空能力。
/// 对应模型的“最佳值”由控制台预设带出，这里只保证任何模型都有可用缺省。
fn capability_defaults() -> Value {
    json!({
        "contextWindow": 32_000,
        "reasoning": false,
        "modalities": ["text"],
    })
}

/// 校验请求里的能力字段（contextWindow / reasoning / modalities），返回已提供的部分。
fn read_capability(map: &Map<String, Value>) -> Result<Map<String, Value>, ApiError> {
    let mut patch = Map::new();
    if let Some(value) = map.get("contextWindow") {
        let window = value.as_u64().ok_or_else(|| {
            ApiError::validation("contextWindow", "type", "上下文窗口必须是整数（tokens）")
        })?;
        if !(1_024..=10_000_000).contains(&window) {
            return Err(ApiError::validation(
                "contextWindow",
                "outOfRange",
                "上下文窗口需要在 1024 到 10000000 tokens 之间",
            ));
        }
        patch.insert("contextWindow".into(), json!(window));
    }
    if let Some(value) = map.get("reasoning") {
        let enabled = value.as_bool().ok_or_else(|| {
            ApiError::validation("reasoning", "type", "思考模式支持与否必须是布尔值")
        })?;
        patch.insert("reasoning".into(), json!(enabled));
    }
    if let Some(value) = map.get("modalities") {
        let list = value.as_array().ok_or_else(|| {
            ApiError::validation("modalities", "type", "输入模态必须是数组")
        })?;
        let mut picked: Vec<String> = Vec::new();
        for item in list {
            let name = item.as_str().ok_or_else(|| {
                ApiError::validation("modalities", "type", "输入模态必须是字符串")
            })?;
            if !MODALITIES.contains(&name) {
                return Err(ApiError::validation(
                    "modalities",
                    "closedSet",
                    format!("未知输入模态: {name}（应为 {}）", MODALITIES.join(", ")),
                ));
            }
            if !picked.iter().any(|existing| existing == name) {
                picked.push(name.to_string());
            }
        }
        if picked.is_empty() {
            return Err(ApiError::validation(
                "modalities",
                "required",
                "至少保留一种输入模态",
            ));
        }
        patch.insert("modalities".into(), json!(picked));
    }
    Ok(patch)
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
            "secretValue",
            "enabled",
            "model",
            "timeoutMs",
            "maxTokens",
            "temperature",
            "headers",
            "contextWindow",
            "reasoning",
            "modalities",
        ],
    )?;
    let capability = read_capability(map)?;
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
    let (secret_ref, secret_value) = read_secret(map)?;
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
    let mut doc = json!({
        "id": id,
        "name": name,
        "kind": kind,
        "endpoint": endpoint,
        "secretRef": secret_ref,
        "secretValue": secret_value,
        "secretPresent": secret_present(
            secret_ref.as_deref(),
            secret_value.as_deref(),
        ),
        "enabled": enabled,
        "model": map.get("model").cloned().unwrap_or(Value::String("default".into())),
        "timeoutMs": map.get("timeoutMs").cloned().unwrap_or(json!(90_000)),
        "maxTokens": map.get("maxTokens").cloned().unwrap_or(json!(2048)),
        "temperature": map.get("temperature").cloned().unwrap_or(json!(0.3)),
        "headers": map.get("headers").cloned().unwrap_or(json!({})),
        "createdAt": now(),
    });
    if let (Some(target), Some(defaults)) = (doc.as_object_mut(), capability_defaults().as_object()) {
        for (key, value) in defaults {
            target.insert(key.clone(), value.clone());
        }
        for (key, value) in capability {
            target.insert(key, value);
        }
    }
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

/// 模型编辑：启停与连接配置共用同一入口；kind 不可变（换类型请重新登记）。
/// `secretValue` 传空串表示清除内置密钥；不传则保持原值。
async fn update_model(
    State(state): State<Shared>,
    Path(model_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "enabled",
            "name",
            "endpoint",
            "model",
            "secretRef",
            "secretValue",
            "timeoutMs",
            "maxTokens",
            "temperature",
            "contextWindow",
            "reasoning",
            "modalities",
        ],
    )?;
    let capability = read_capability(map)?;
    let doc = state
        .store
        .get_document("model", &model_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("模型 {model_id}")))?;
    let mut next = doc.body;
    let mut summary = vec!["模型配置更新".to_string()];
    if let Some(object) = next.as_object_mut() {
        if map.contains_key("enabled") {
            let enabled = util::flag(map, "enabled")?;
            object.insert("enabled".into(), json!(enabled));
            summary.push(if enabled { "启用" } else { "停用" }.to_string());
        }
        for field in ["name", "endpoint", "model"] {
            if let Some(value) = util::optional_text(map, field)? {
                if value.trim().is_empty() {
                    return Err(ApiError::validation(field, "required", "不能为空"));
                }
                object.insert(field.into(), json!(value));
            }
        }
        if map.contains_key("secretRef") || map.contains_key("secretValue") {
            let (secret_ref, secret_value) = read_secret(map)?;
            object.insert("secretRef".into(), json!(secret_ref));
            object.insert("secretValue".into(), json!(secret_value));
            summary.push("密钥".to_string());
        }
        for field in ["timeoutMs", "maxTokens", "temperature"] {
            if let Some(value) = map.get(field) {
                object.insert(field.into(), value.clone());
            }
        }
        for (key, value) in &capability {
            object.insert(key.clone(), value.clone());
        }
        let secret_ref = object
            .get("secretRef")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let secret_value = object
            .get("secretValue")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        object.insert(
            "secretPresent".into(),
            json!(secret_present(secret_ref, secret_value)),
        );
    }
    let saved = state
        .store
        .update_document("model", &model_id, &next, Some(doc.version))
        .await?;
    crate::observe::append_audit(
        &state,
        "settings.model.update",
        "succeeded",
        &summary.join("："),
        json!({"id": model_id}),
        None,
    )
    .await?;
    Ok(util::ok(&model_view(&saved.body)))
}

/// 删除模型：被默认模型设置或工作流版本步骤引用时拒绝，避免运行路径悬空。
async fn delete_model(
    State(state): State<Shared>,
    Path(model_id): Path<String>,
) -> Result<Response, ApiError> {
    state
        .store
        .get_document("model", &model_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("模型 {model_id}")))?;
    let default_id = value(&state, "ai.defaultModelId").await?;
    if default_id.as_str() == Some(model_id.as_str()) {
        return Err(ApiError::dependency(
            "默认模型设置引用了该模型，请先改设置再删模型",
        ));
    }
    for doc in state.store.list_documents("delivery_rule").await? {
        let referenced = doc
            .body
            .get("targets")
            .and_then(Value::as_array)
            .is_some_and(|targets| {
                targets.iter().any(|target| {
                    target["kind"].as_str() == Some("model")
                        && target["id"].as_str() == Some(model_id.as_str())
                })
            });
        if referenced {
            return Err(ApiError::dependency(format!(
                "自动投递规则 {} 引用了该模型",
                doc.body["name"].as_str().unwrap_or(&doc.id)
            )));
        }
    }
    state.store.delete_document("model", &model_id).await?;
    crate::observe::append_audit(
        &state,
        "settings.model.delete",
        "succeeded",
        "删除模型资源",
        json!({"id": model_id}),
        None,
    )
    .await?;
    Ok(util::ok(&json!({"id": model_id, "deleted": true})))
}

/// 连通测试：`{id}` 测已登记模型，或直接给一份未保存的草稿配置（endpoint/model/密钥…）。
/// remoteApi/localService 发一轮最小补全（16 token 上限、30s 超时），走的就是生产同款调用栈；
/// localCli 只验证命令可执行（--version），不触发真实推理轮。
async fn test_model(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &[
            "id",
            "kind",
            "endpoint",
            "model",
            "secretRef",
            "secretValue",
            "headers",
            "timeoutMs",
        ],
    )?;
    let mut spec_value = match map.get("id").and_then(Value::as_str) {
        Some(id) => {
            state
                .store
                .get_document("model", id)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("模型 {id}")))?
                .body
        }
        None => json!({}),
    };
    if let Some(object) = spec_value.as_object_mut() {
        for field in ["kind", "endpoint", "model", "headers", "timeoutMs"] {
            if let Some(value) = map.get(field) {
                object.insert(field.into(), value.clone());
            }
        }
        // 密钥覆盖仅在显式提供时生效；空串表示本次测试不带密钥。
        for field in ["secretRef", "secretValue"] {
            if let Some(value) = map.get(field) {
                object.insert(field.into(), value.clone());
            }
        }
        object.entry("enabled").or_insert(json!(true));
    }
    let mut spec = crate::models::model_spec(&spec_value)?;
    let started = std::time::Instant::now();
    if spec.mode == workflow::Mode::Cli {
        let program = spec.command.first().cloned().unwrap_or_default();
        let version = probe_cli_version(&program).await?;
        return Ok(util::ok(&json!({
            "ok": true,
            "mode": "cli",
            "latencyMs": started.elapsed().as_millis(),
            "version": version,
            "note": "CLI 只验证命令可执行；推理轮由生产运行触发",
        })));
    }
    spec.max_tokens = Some(16);
    spec.temperature = Some(0.0);
    spec.timeout = Some("30s".into());
    let reply = workflow::complete_model(
        &spec,
        "连通测试：请只回复 ok 两个字母。",
        &state.services.proxy_setting(),
    )
    .await
    .map_err(|error| ApiError::dependency(error.to_string()).retryable())?;
    let sample: String = reply.chars().take(80).collect();
    Ok(util::ok(&json!({
        "ok": true,
        "mode": spec.mode.label(),
        "latencyMs": started.elapsed().as_millis(),
        "sample": sample,
    })))
}

/// 拉取平台模型清单：`GET {endpoint}/models`，用表单里尚未保存的密钥直接鉴权。
/// 只回候选 id，不落库；平台更新型号时控制台不必等发版。
async fn discover_models(
    State(state): State<Shared>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = util::object(&body)?;
    util::unknown(
        map,
        &["kind", "endpoint", "secretRef", "secretValue", "headers"],
    )?;
    let kind = util::optional_text(map, "kind")?.unwrap_or_else(|| "remoteApi".into());
    if kind == "localCli" {
        return Err(ApiError::validation(
            "kind",
            "unsupported",
            "命令行模型没有模型清单接口",
        ));
    }
    let endpoint = util::non_empty(map, "endpoint")?;
    let parsed = url::Url::parse(&endpoint)
        .map_err(|_| ApiError::validation("endpoint", "format", "不是有效地址"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::validation("endpoint", "scheme", "只接受 http/https"));
    }
    let inline = util::optional_text(map, "secretValue")?.filter(|value| !value.trim().is_empty());
    let secret_ref = util::optional_text(map, "secretRef")?.filter(|value| !value.trim().is_empty());
    let key = inline.or_else(|| secret_ref.and_then(|name| std::env::var(name).ok()));
    let mut headers = vec![("accept".to_string(), "application/json".to_string())];
    if let Some(key) = key.filter(|value| !value.trim().is_empty()) {
        // OpenAI 兼容层用 Bearer；Anthropic/Gemini 兼容层也接受 x-api-key，一并带上。
        headers.push(("authorization".into(), format!("Bearer {key}")));
        headers.push(("x-api-key".into(), key));
    }
    if let Some(extra) = util::optional_object(map, "headers")? {
        for (name, value) in extra {
            if let Some(value) = value.as_str() {
                headers.push((name, value.to_string()));
            }
        }
    }
    let url = format!("{}/models", endpoint.trim_end_matches('/'));
    let reply = workflow::http_request(
        "GET",
        &url,
        &headers,
        None,
        std::time::Duration::from_secs(20),
        &state.services.proxy_setting(),
    )
    .await
    .map_err(|error| ApiError::dependency(error.to_string()).retryable())?;
    let text = reply
        .ok("模型列表")
        .map_err(|error| ApiError::dependency(error.to_string()))?;
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|_| ApiError::dependency("平台没有返回 JSON 模型清单"))?;
    let items = discovered_ids(&parsed);
    Ok(util::ok(&json!({"items": items, "count": items.len()})))
}

/// 兼容三种常见清单形状：`{data:[{id}]}`、`{models:[{id|name}]}` 与字符串数组；
/// Gemini 兼容层会带 `models/` 前缀，统一去掉后再交给控制台。
fn discovered_ids(body: &Value) -> Vec<String> {
    let list = body
        .get("data")
        .or_else(|| body.get("models"))
        .or_else(|| body.get("items"))
        .unwrap_or(body);
    let mut out = Vec::new();
    if let Some(items) = list.as_array() {
        for item in items {
            let id = item
                .as_str()
                .or_else(|| item.get("id").and_then(Value::as_str))
                .or_else(|| item.get("name").and_then(Value::as_str));
            if let Some(id) = id {
                let id = id.strip_prefix("models/").unwrap_or(id);
                if !id.is_empty() {
                    out.push(id.to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 探测 CLI：直接执行 `program --version`；Windows 上 npm 分发常是 .cmd shim，
/// 直接执行失败时回落 `cmd /c`。
async fn probe_cli_version(program: &str) -> Result<String, ApiError> {
    if program.trim().is_empty() {
        return Err(ApiError::validation("endpoint", "required", "CLI 命令为空"));
    }
    let direct = run_version_probe(program, None).await;
    let outcome = match direct {
        Probe::Ran(output) => output,
        Probe::SpawnFailed(error) => {
            if cfg!(windows) {
                match run_version_probe(program, Some("cmd")).await {
                    Probe::Ran(output) => output,
                    Probe::SpawnFailed(_) => {
                        return Err(
                            ApiError::dependency(format!("未找到 {program}：{error}")).retryable()
                        );
                    }
                }
            } else {
                return Err(ApiError::dependency(format!("未找到 {program}：{error}")).retryable());
            }
        }
    };
    match outcome {
        Ok(text) => Ok(text),
        Err(message) => Err(ApiError::dependency(message).retryable()),
    }
}

enum Probe {
    Ran(Result<String, String>),
    SpawnFailed(String),
}

/// 子进程输出解码：UTF-8 优先，失败按 GBK 回落（中文 Windows 的 cmd 输出）。
fn decode_console(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => encoding_rs::GBK.decode(bytes).0.into_owned(),
    }
}

/// `wrapper` 为 Some 时经 `cmd.exe /c` 中转（Windows .cmd shim 场景）。
async fn run_version_probe(program: &str, wrapper: Option<&str>) -> Probe {
    let (exe, args): (String, Vec<String>) = match wrapper {
        Some(_) => (
            "cmd.exe".into(),
            vec!["/c".into(), program.to_string(), "--version".into()],
        ),
        None => (program.to_string(), vec!["--version".into()]),
    };
    let running = tokio::process::Command::new(&exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(5), running).await {
        Err(_) => Probe::Ran(Err(format!("{program} --version 超时"))),
        Ok(Err(error)) => Probe::SpawnFailed(error.to_string()),
        Ok(Ok(output)) => {
            let stdout = decode_console(&output.stdout);
            let stderr = decode_console(&output.stderr);
            let first = stdout
                .trim()
                .lines()
                .next()
                .or_else(|| stderr.trim().lines().next())
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect::<String>();
            if output.status.success() {
                Probe::Ran(Ok(first))
            } else {
                Probe::Ran(Err(format!(
                    "{program} 退出码 {:?}：{}",
                    output.status.code(),
                    if first.is_empty() {
                        "无输出"
                    } else {
                        &first
                    }
                )))
            }
        }
    }
}

/// 常见 agent CLI 登记表：id、展示名、建议命令（提示词经标准输入送入）。
const KNOWN_AGENT_CLIS: &[(&str, &str, &str)] = &[
    ("claude", "Claude Code", "claude -p"),
    ("cursor-agent", "Cursor CLI", "cursor-agent -p"),
    ("atomcode", "AtomCode CLI", "atomcode"),
    ("qoder", "Qoder CLI", "qoder"),
    ("codex", "OpenAI Codex CLI", "codex exec -"),
    ("gemini", "Gemini CLI", "gemini -p"),
    ("llm", "llm（命令行推理工具）", "llm"),
];

/// 扫描发现本机已安装的 agent CLI：逐个执行 `--version` 探测，返回全部候选与命中状态。
async fn cli_candidates() -> Result<Response, ApiError> {
    let mut items = Vec::with_capacity(KNOWN_AGENT_CLIS.len());
    for (id, label, command) in KNOWN_AGENT_CLIS {
        let program = command.split_whitespace().next().unwrap_or(id);
        let outcome = match run_version_probe(program, None).await {
            Probe::Ran(result) => result,
            Probe::SpawnFailed(_) if cfg!(windows) => {
                match run_version_probe(program, Some("cmd")).await {
                    Probe::Ran(result) => result,
                    _ => Err("未找到".into()),
                }
            }
            Probe::SpawnFailed(_) => Err("未找到".into()),
        };
        match outcome {
            Ok(version) => items.push(json!({
                "id": id, "label": label, "command": command, "found": true, "version": version,
            })),
            Err(reason) => items.push(json!({
                "id": id, "label": label, "command": command, "found": false, "version": reason,
            })),
        }
    }
    Ok(util::ok(&json!({"items": items})))
}

/// 密钥输入对：`secretRef` 是环境变量名，`secretValue` 是直接登记的密钥值。
/// 两者可任选其一或同时提供（值优先）；空串按未提供处理。
fn read_secret(
    map: &serde_json::Map<String, Value>,
) -> Result<(Option<String>, Option<String>), ApiError> {
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
                "只接受环境变量名（大写字母、数字与下划线），密钥值请填 secretValue",
            ));
        }
    }
    let secret_value = util::optional_text(map, "secretValue")?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(value) = secret_value.as_deref() {
        if value.len() > 512 || value.chars().any(|ch| (ch as u32) < 0x20) {
            return Err(ApiError::validation(
                "secretValue",
                "format",
                "密钥不超过 512 字符且不能包含控制字符",
            ));
        }
    }
    Ok((secret_ref, secret_value))
}

/// 密钥是否可用：内置值总是可用；环境变量引用取决于采集进程的环境。
fn secret_present(secret_ref: Option<&str>, secret_value: Option<&str>) -> bool {
    secret_value.is_some() || secret_ref.is_some_and(|name| std::env::var(name).is_ok())
}

/// 模型投影：请求头只回传名称（值可能是用户误填的凭据，不进入列表、差异或导出），
/// 密钥值只写不读，仅暴露来源与可用性。
fn model_view(doc: &Value) -> Value {
    let mut view = doc.clone();
    if let Some(map) = view.as_object_mut() {
        // 密钥值只写不读：投影里永远剔除，只暴露来源与可用性。
        let inline = map
            .remove("secretValue")
            .and_then(|value| value.as_str().map(str::to_string))
            .is_some_and(|value| !value.trim().is_empty());
        map.insert("secretInline".into(), json!(inline));
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

#[cfg(test)]
mod tests {
    use super::discovered_ids;
    use serde_json::json;

    #[test]
    fn discovers_openai_style_ids() {
        let body = json!({"data": [{"id": "gpt-6-astra"}, {"id": "gpt-6-luna"}]});
        assert_eq!(discovered_ids(&body), vec!["gpt-6-astra", "gpt-6-luna"]);
    }

    #[test]
    fn discovers_models_prefix_and_names() {
        let body = json!({"models": [{"name": "models/gemini-4-argon"}, {"id": "gemini-3.8-flash"}]});
        assert_eq!(
            discovered_ids(&body),
            vec!["gemini-3.8-flash", "gemini-4-argon"]
        );
    }
}
