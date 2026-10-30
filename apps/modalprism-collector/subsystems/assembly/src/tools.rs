//! 工具面的单次采集：取数 → 识别 → 适配 → 落库/返回。
//!
//! 与调度链路共用同一套识别器与适配器（[`adapters::run`]），区别只在于：
//! 任务来自临时请求而不是信源声明，因此没有重试与游标语义，失败按「降级返回」处理——
//! 类型识别不出来也照样产出原始面与区域索引，不让一次临时采集因为识别问题直接失败。

use std::path::Path;
use std::sync::Arc;

use adapters::{
    Admission, DetectorChoice, PartBytes, Registry, RunRequest, empty_identities, parse_pipeline,
    run, source_view,
};
use bytes::Bytes;
use strata_model::{
    ApplyOpts, FacetKind, IdentityKey, Locator, Op, PartId, PartRole, Producer, SourceId,
    Timestamp,
};
use serde_json::{Map, Value, json};

use crate::{Error, Services};

/// 单次采集的正文上限：临时采集不落盘暂存，直接在内存里完成。
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// 临时采集的归属信源前缀。内容行按 `source/<slug>` 归属，与调度链路一致。
const ADHOC_PREFIX: &str = "source/tool-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 只识别与适配，不写入。
    Preview,
    /// 落库后返回内容标识。
    Store,
}

impl Mode {
    fn parse(raw: Option<&str>) -> Result<Self, Error> {
        match raw.map(str::trim).filter(|text| !text.is_empty()) {
            None | Some("preview") | Some("dry-run") => Ok(Self::Preview),
            Some("store") | Some("save") => Ok(Self::Store),
            Some(other) => Err(Error::new(format!(
                "未知模式: {other}（应为 preview 或 store）"
            ))),
        }
    }
}

/// 一次单次采集的入参。
pub struct OnceRequest {
    /// URL / 文本 / 本地文件三选一。
    pub input: Vec<u8>,
    pub media_type: String,
    pub locator: Locator,
    pub source_id: String,
    pub mode: Mode,
    /// 显式 pipeline；缺省按 hint 选智能适配器。
    pub pipeline: Option<Value>,
    /// `content`（默认）或 `discover`。
    pub hint: String,
}

impl OnceRequest {
    /// 从工具参数解析。`file` 读取受工作区当前目录约束，避免任意路径读取。
    pub async fn parse(services: &Arc<Services>, args: &Value) -> Result<Self, Error> {
        let mode = Mode::parse(args.get("mode").and_then(Value::as_str))?;
        let hint = args
            .get("hint")
            .and_then(Value::as_str)
            .unwrap_or("content")
            .to_string();
        let source_id = args
            .get("sourceId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.starts_with("source/") {
                    value.to_string()
                } else {
                    format!("source/{}", value.trim_start_matches("source/"))
                }
            })
            .unwrap_or_else(|| format!("{ADHOC_PREFIX}adhoc"));

        let url = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let text = args.get("text").and_then(Value::as_str);
        let file = args
            .get("file")
            .or_else(|| args.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let (input, media_type, locator) = if let Some(url) = url {
            let parsed = url::Url::parse(url)
                .map_err(|_| Error::new("url 无法解析"))?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(Error::new("仅支持 http/https"));
            }
            let (bytes, media) = fetch_bytes(services, parsed.as_str()).await?;
            (
                bytes,
                media,
                Locator::Url {
                    url: parsed.to_string(),
                },
            )
        } else if let Some(text) = text {
            (
                text.as_bytes().iter().take(MAX_BYTES).copied().collect::<Vec<u8>>(),
                args.get("mediaType")
                    .and_then(Value::as_str)
                    .unwrap_or("text/plain")
                    .to_string(),
                Locator::Url {
                    url: format!("text:{}", short_hash(text)),
                },
            )
        } else if let Some(path) = file {
            let path = Path::new(path);
            let bytes = std::fs::read(path).map_err(|error| {
                Error::new(format!("读取文件失败（{}）: {error}", path.display()))
            })?;
            (
                bytes.iter().take(MAX_BYTES).copied().collect::<Vec<u8>>(),
                guess_media(path),
                Locator::Url {
                    url: format!("file:{}", path.display()),
                },
            )
        } else {
            return Err(Error::new("url / text / file 至少提供一个"));
        };

        Ok(Self {
            input,
            media_type,
            locator,
            source_id,
            mode,
            pipeline: args.get("pipeline").cloned().filter(Value::is_array),
            hint,
        })
    }
}

/// 执行一次单次采集。零产出不报错：降级为「原始面 + 区域索引」并带 warnings。
pub async fn once(services: &Arc<Services>, request: OnceRequest) -> Result<Value, Error> {
    if request.input.is_empty() {
        return Err(Error::new("输入为空"));
    }
    let snapshot = services.config().applied();
    let prism = services.prism();
    let prism_ready = prism.is_some();
    let adapter = match (request.hint.as_str(), request.mode) {
        ("discover", _) => "adapter/smart-discover",
        ("content", Mode::Store) | (_, Mode::Store) => "adapter/smart-content",
        _ => "adapter/smart-discover",
    };
    let pipeline = request.pipeline.clone().unwrap_or_else(|| {
        json!([{ "run": [adapter] }])
    });
    let rules = parse_pipeline(&pipeline)
        .map_err(|error| Error::new(error.to_string()))?;
    let media = request.media_type.clone();
    let parts = vec![PartBytes {
        part_id: PartId::new("body").map_err(|error| Error::new(error.to_string()))?,
        role: PartRole::Document,
        declared: media.clone(),
        charset: None,
        bytes: Bytes::from(request.input.clone()),
        facet: None,
    }];
    let source = SourceId::new(request.source_id.as_str())
        .map_err(|error| Error::new(error.to_string()))?;
    let detector = if prism_ready {
        DetectorChoice::Prism
    } else {
        DetectorChoice::Basic
    };
    // 文档资源在变更集里用别名：临时采集不预生成 rid，也不为它空写一份资源。
    let identity = match &request.locator {
        Locator::Url { url } => Some(format!("url:{url}")),
        Locator::ApiEndpoint { endpoint } => Some(format!("url:{endpoint}")),
        Locator::PushChannel { channel } => Some(format!("ext:push:{channel}")),
        Locator::Query { .. } => None,
    };
    let (resource_alias, ensure) = match identity {
        Some(identity) => {
            IdentityKey::parse(&identity).map_err(|_| Error::new("文档身份无效"))?;
            (
                Some("p0".to_string()),
                Some(Op::Ensure {
                    identity,
                    ns: None,
                    labels: Default::default(),
                    r#as: "p0".into(),
                }),
            )
        }
        None => (None, None),
    };
    let output = run(
        &Registry::builtins_with(prism.clone()),
        RunRequest {
            parts,
            pipeline: rules,
            reason: strata_model::TaskReason::Manual,
            source: source_view(source.as_str(), "tool")
                .map_err(|error| Error::new(error.to_string()))?,
            resource: None,
            resource_alias,
            origin: None,
            locator: request.locator.clone(),
            detector,
            admission: Admission::from_value(&json!({}))
                .map_err(|error| Error::new(error.to_string()))?,
            now: Timestamp::now(),
            initial: true,
            producer: Producer {
                module: "tool".into(),
                version: "1".into(),
                config_generation: snapshot.generation,
            },
            specs: Map::new(),
            identities: empty_identities(),
            prism,
        },
    )
    .await
    .map_err(|error| Error::new(error.to_string()))?;

    let mut changeset = output.changeset;
    let mut warnings = Vec::new();
    // 别名必须先于落到它上面的数据面。
    if let Some(ensure) = ensure {
        changeset.ops.insert(0, ensure);
    }
    // 归属：内容行按信源过滤，临时采集也必须带上来源标签。
    for op in &mut changeset.ops {
        if let Op::Ensure { labels, .. } = op {
            labels
                .entry("source".to_string())
                .or_insert_with(|| request.source_id.clone());
        }
    }
    let facets = summarize(&changeset.ops);
    if facets.is_empty() {
        warnings.push("识别未产出可用数据面，仅返回原始内容".to_string());
    }
    if !prism_ready {
        warnings.push("结构化识别引擎未装载，已按基础检测降级".to_string());
    }
    let degraded = !prism_ready || facets.is_empty();
    let mut seen = std::collections::BTreeSet::new();
    let identities = changeset
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Ensure { identity, .. } => Some(identity.clone()),
            _ => None,
        })
        // 文档资源与发现条目可能指向同一身份，对外只报一次。
        .filter(|identity| seen.insert(identity.clone()))
        .collect::<Vec<_>>();

    if request.mode == Mode::Store {
        let result = services
            .strata()
            .apply(changeset, ApplyOpts::default())
            .await
            .map_err(|error| Error::new(error.to_string()))?;
        return Ok(json!({
            "mode": "store",
            "stored": true,
            "degraded": degraded,
            "warnings": warnings,
            "sourceId": request.source_id,
            "identities": identities,
            "commit": result.commit_id.as_str(),
            "facets": facets,
            "follows": output.follows.len(),
        }));
    }
    Ok(json!({
        "mode": "preview",
        "stored": false,
        "degraded": degraded,
        "warnings": warnings,
        "sourceId": request.source_id,
        "identities": identities,
        "facets": facets,
        "follows": output.follows.len(),
    }))
}

/// 只识别不落库：类型分布、区域与建议的适配器/声明。
pub async fn probe(services: &Arc<Services>, args: &Value) -> Result<Value, Error> {
    let Some(prism) = services.prism() else {
        return Ok(json!({
            "available": false,
            "degraded": true,
            "warnings": ["结构化识别引擎未装载"],
            "types": [],
            "regions": [],
        }));
    };
    let selectors = args
        .get("selectors")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let text = match args.get("text").and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => text.to_string(),
        _ => {
            let request = OnceRequest::parse(services, args).await?;
            String::from_utf8_lossy(&request.input).into_owned()
        }
    };
    let scan = prism
        .scan_with(&text, &selectors, None)
        .await
        .map_err(|error| Error::new(error.to_string()))?;
    let types = scan
        .types
        .iter()
        .map(|item| json!({ "id": item.id, "count": item.count }))
        .collect::<Vec<_>>();
    let regions = scan
        .regions
        .iter()
        .enumerate()
        .map(|(index, region)| {
            json!({
                "typeId": region.type_id,
                "start": region.start,
                "end": region.end,
                "confidence": region.confidence,
                "kind": facet_kind_of(&region.type_id),
                "variant": format!("r{index}"),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "available": true,
        "degraded": types.is_empty(),
        "typeCounts": types,
        "types": types.iter().map(|item| item.get("id").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "regions": regions,
        "truncated": scan.truncated,
        "suggest": suggest(&scan),
    }))
}

/// 按扫描结果给出建议的适配器与声明骨架。
fn suggest(scan: &adapters::ScanOutcome) -> Value {
    let mut warnings = Vec::new();
    if scan.types.is_empty() {
        warnings.push("未识别到已知结构类型：采集将降级为原始内容 + 文本块");
    }
    if scan.truncated {
        warnings.push("区域数量超过上限，索引已截断");
    }
    let regions = scan.regions.len();
    json!({
        "adapter": if regions > 1 { "adapter/smart-content" } else { "adapter/smart-discover" },
        "pipeline": [{ "run": [if regions > 1 { "adapter/smart-content" } else { "adapter/smart-discover" }] }],
        "maxRegions": regions,
        "warnings": warnings,
    })
}

/// RawPrism 的 TypeId 到 facet 种类的映射：与智能适配器保持同一口径。
fn facet_kind_of(type_id: &str) -> &'static str {
    match type_id {
        "struct.json" | "struct.ndjson" => "json",
        "struct.yaml" | "struct.toml" => "yaml",
        "struct.csv" => "csv",
        "markup.html" => "html",
        "text.md" => "md",
        "entity.url" => "links",
        _ => "text",
    }
}

/// 变更集的面向用户摘要：写了哪些数据面，而不是抛原始 ops。
fn summarize(ops: &[Op]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for op in ops {
        let Op::PutFacet {
            kind,
            variant,
            body,
            media_type,
            ..
        } = op
        else {
            continue;
        };
        let kind_text = kind.as_str().to_string();
        if matches!(
            kind,
            FacetKind::Embedding
        ) {
            continue;
        }
        let size = match body {
            strata_model::Body::Text { text } => text.len() as u64,
            strata_model::Body::Struct { .. } => 0,
            strata_model::Body::Staged { part } => part.size(),
            _ => 0,
        };
        out.push(json!({
            "kind": kind_text,
            "variant": variant,
            "mediaType": media_type,
            "size": size,
        }));
    }
    out
}

async fn fetch_bytes(services: &Arc<Services>, url: &str) -> Result<(Vec<u8>, String), Error> {
    let policy = crate::policy_of(&services.config().applied().system);
    let request = accessors::HttpRequest {
        method: "GET".into(),
        url: url.to_string(),
        headers: Vec::new(),
        body: Bytes::new(),
    };
    let response = services
        .egress_gateway()
        .request_governed(&policy, request)
        .await
        .map_err(|error| Error::new(error.message))?;
    if response.status < 200 || response.status >= 300 {
        return Err(Error::new(format!("目标返回 {}", response.status)));
    }
    let media = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let bytes = response
        .body
        .iter()
        .take(MAX_BYTES)
        .copied()
        .collect::<Vec<u8>>();
    Ok((bytes, media))
}

fn guess_media(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => "application/json",
        "ndjson" | "jsonl" => "application/x-ndjson",
        "yaml" | "yml" => "application/yaml",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        _ => "text/plain",
    }
    .to_string()
}

fn short_hash(text: &str) -> String {
    strata_model::content_hash(text.as_bytes())
        .chars()
        .take(12)
        .collect()
}
