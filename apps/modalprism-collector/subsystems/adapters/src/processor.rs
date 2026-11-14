//! 内容处理器（第一期第二刀，只做可跑通的一刀）。
//!
//! 处理器在适配器管道产出数据面之后运行：只对规范化片段（`PutFacet` 文本）追加
//! 派生片段，不改写原始信封、不改写原始片段。确定性抽取仍留在适配器层，这里只做
//! 派生（例如评论过滤）。
//!
//! 去重语义：同一（组件 id、版本、输入单元哈希、有效参数）在一个任务执行内只算一次。
//! 已有成功产出（含"产出为空"）的输入单元按成功跳过；失败不计入备忘，因此同一输入
//! 单元被重复挂载时，后续挂载仍会重试。

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::error::AdapterError;
use crate::textutil::params_hash;
use strata_model::{Body, FacetKind, Op};
/// 处理器元信息。id 约定 `content-processor/<name>`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessorMeta {
    pub id: &'static str,
    pub version: &'static str,
}

/// 内置内容处理器契约：输入文本与有效参数，产出派生文本。
/// 返回 `None` 表示该单元没有派生产出（仍按成功记入备忘）。
pub trait ContentProcessor: Send + Sync {
    fn meta(&self) -> &'static ProcessorMeta;
    fn process(&self, text: &str, spec: &Value) -> Result<Option<String>, AdapterError>;
}

/// 内置文本过滤器：按行过滤，丢弃包含 `dropKeywords`（ASCII 大小写不敏感子串）的行
/// 与短于 `minLength` 的行。参数缺省时原样通过。
pub struct TextFilter;

impl ContentProcessor for TextFilter {
    fn meta(&self) -> &'static ProcessorMeta {
        static META: ProcessorMeta = ProcessorMeta {
            id: "content-processor/text-filter",
            version: "1",
        };
        &META
    }

    fn process(&self, text: &str, spec: &Value) -> Result<Option<String>, AdapterError> {
        let object = match spec {
            Value::Null => None,
            Value::Object(object) => Some(object),
            _ => return Err(AdapterError::config("text-filter 的 spec 必须是对象")),
        };
        let min_length = object
            .and_then(|object| object.get("minLength"))
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    AdapterError::config("text-filter 的 minLength 必须是非负整数")
                })
            })
            .transpose()?;
        let keywords: Vec<String> = object
            .and_then(|object| object.get("dropKeywords"))
            .map(|value| {
                value.as_array().map(|items| {
                    items
                        .iter()
                        .map(|item| item.as_str().unwrap_or_default().to_string())
                        .collect()
                })
            })
            .unwrap_or(Some(Vec::new()))
            .ok_or_else(|| AdapterError::config("text-filter 的 dropKeywords 必须是字符串列表"))?;
        let min_length = min_length.unwrap_or(0);
        let kept: Vec<&str> = text
            .lines()
            .filter(|line| {
                line.chars().count() >= min_length as usize
                    && !keywords
                        .iter()
                        .any(|keyword| {
                            !keyword.is_empty()
                                && crate::textutil::find_ascii_ci(line.as_bytes(), keyword.as_bytes())
                                    .is_some()
                        })
            })
            .collect();
        if kept.is_empty() {
            return Ok(None);
        }
        Ok(Some(kept.join("\n")))
    }
}

/// 内置处理器注册表（文本单元）。图片单元的 OCR 走 [`apply_ocr`] 与
/// [`ocr_engine`]；情绪、转写、因子不在本刀范围。
pub fn builtin(id: &str) -> Option<&'static dyn ContentProcessor> {
    static TEXT_FILTER: TextFilter = TextFilter;
    match id {
        "content-processor/text-filter" => Some(&TEXT_FILTER),
        _ => None,
    }
}

/// 处理器挂载：从管道规则解析出的组件 id 与有效参数（规则 `spec` 里按 id 取）。
#[derive(Clone, Debug)]
pub struct Mount {
    pub id: String,
    pub spec: Value,
}

/// 去重备忘：只记成功（含产出为空）。失败/取消不记，重复挂载可重试。
#[derive(Default)]
pub struct Memo {
    successes: HashSet<String>,
}

impl Memo {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(id: &str, version: &str, unit: &Value, spec: &Value) -> String {
        let unit_hash = params_hash(&json!({
            "component": id,
            "version": version,
            "unit": unit,
            "params": params_hash(spec),
        }));
        format!("{id}@{version}:{unit_hash}")
    }
}

/// 处理器通过判定：该（组件、版本、输入单元、有效参数）组合是否已有成功产出。
fn already_done(memo: &Memo, id: &str, version: &str, unit: &Value, spec: &Value) -> bool {
    memo.successes
        .contains(&Memo::key(id, version, unit, spec))
}

/// 处理器通过：对数据面里的每个文本片段应用每个挂载，返回追加的派生 `PutFacet`。
/// 原始 op 原样保留——本函数只追加，不改写、不删除。
pub fn apply(
    ops: &[Op],
    mounts: &[Mount],
    memo: &mut Memo,
) -> Result<(Vec<Op>, usize), AdapterError> {
    let mut derived = Vec::new();
    let mut processed = 0usize;
    for op in ops {
        let Op::PutFacet {
            target,
            kind,
            variant,
            body: Body::Text { text },
            ..
        } = op
        else {
            continue;
        };
        let unit = json!({
            "target": target,
            "kind": kind,
            "variant": variant,
            "text": text,
        });
        for mount in mounts {
            let processor = builtin(&mount.id).ok_or_else(|| {
                AdapterError::config(format!("未知内容处理器: {}", mount.id))
            })?;
            let meta = processor.meta();
            if already_done(memo, meta.id, meta.version, &unit, &mount.spec) {
                // 已有成功产出：跳过并视为成功。
                continue;
            }
            processed += 1;
            match processor.process(text, &mount.spec) {
                Ok(Some(filtered)) => {
                    memo.successes
                        .insert(Memo::key(meta.id, meta.version, &unit, &mount.spec));
                    derived.push(Op::PutFacet {
                        target: target.clone(),
                        kind: derived_kind(&mount.spec)?,
                        variant: Some(processor_variant(meta.id)),
                        body: Body::Text { text: filtered },
                        media_type: Some("text/plain".into()),
                        lineage: lineage_of(op, &mount.spec),
                    });
                }
                Ok(None) => {
                    // 产出为空同样是成功：记入备忘，重复挂载按成功跳过。
                    memo.successes
                        .insert(Memo::key(meta.id, meta.version, &unit, &mount.spec));
                }
                // 失败不记备忘：同一单元被再次挂载时可重试。
                Err(error) => return Err(error),
            }
        }
    }
    Ok((derived, processed))
}

/// 派生片段种类：spec 里 `kind`（默认 `text`）。
fn derived_kind(spec: &Value) -> Result<FacetKind, AdapterError> {
    let raw = spec
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("text");
    FacetKind::parse(raw).map_err(|_| AdapterError::config(format!("未知 facet 种类: {raw}")))
}

/// 派生片段 variant：处理器 id 的 `<name>` 部分（`content-processor/text-filter` → `text-filter`）。
fn processor_variant(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

/// 派生血缘：沿用输入片段的血缘输入（输入 span），参数哈希取有效参数。
fn lineage_of(op: &Op, spec: &Value) -> strata_model::LineageInput {
    let Op::PutFacet { lineage, .. } = op else {
        unreachable!("lineage_of 只对 PutFacet 调用");
    };
    strata_model::LineageInput {
        inputs: lineage.inputs.clone(),
        params_hash: params_hash(spec),
    }
}

/// OCR 引擎契约：输入磁盘上的图片文件路径与可选语言（BCP-47 或 Tesseract 语言标签），
/// 产出识别文本。返回 `None` 表示没有识别到文字（按"成功且无产出"结算）。
/// 跨平台契约：Linux / macOS / Windows 走同一接口，引擎实现不依赖条件编译。
pub trait OcrEngine: Send + Sync {
    fn recognize(&self, image_path: &std::path::Path, language: Option<&str>)
        -> Result<Option<String>, AdapterError>;
}

/// Tesseract 引擎（开源，三大桌面平台安装方式一致：`tesseract <image> stdout`）。
/// 程序路径可配置（spec 的 `program`，缺省从 PATH 找 `tesseract`）；找不到或不可
/// 执行时明确报配置错误，不静默降级。`psm`（页面分割模式）可选。
pub struct TesseractOcr {
    program: String,
    psm: Option<u32>,
}

impl TesseractOcr {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            psm: None,
        }
    }

    /// 从挂载 spec 取程序路径（`program`）与页面分割模式（`psm`）。
    pub fn from_spec(spec: &Value) -> Result<Self, AdapterError> {
        let mut engine = match spec.get("program") {
            None | Some(Value::Null) => Self::new("tesseract"),
            Some(Value::String(value)) if !value.is_empty() => Self::new(value.as_str()),
            Some(Value::String(_)) => {
                return Err(AdapterError::config("ocr 的 program 不能为空字符串"));
            }
            Some(_) => return Err(AdapterError::config("ocr 的 program 必须是字符串")),
        };
        engine.psm = match spec.get("psm") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value.as_u64().ok_or_else(|| {
                    AdapterError::config("ocr 的 psm 必须是非负整数")
                })? as u32,
            ),
        };
        Ok(engine)
    }
}

/// 组装 Tesseract 命令行参数（不含程序名）：`<image> stdout [-l lang] [--psm n]`。
fn tesseract_args(
    image_path: &std::path::Path,
    language: Option<&str>,
    psm: Option<u32>,
) -> Vec<String> {
    let mut args = vec![
        image_path.to_string_lossy().into_owned(),
        "stdout".to_string(),
    ];
    if let Some(language) = language.filter(|value| !value.is_empty()) {
        args.push("-l".to_string());
        args.push(language.to_string());
    }
    if let Some(psm) = psm {
        args.push("--psm".to_string());
        args.push(psm.to_string());
    }
    args
}

impl OcrEngine for TesseractOcr {
    fn recognize(
        &self,
        image_path: &std::path::Path,
        language: Option<&str>,
    ) -> Result<Option<String>, AdapterError> {
        let output = std::process::Command::new(&self.program)
            .args(tesseract_args(image_path, language, self.psm))
            .output()
            .map_err(|source| {
                // 引擎缺失/不可执行是环境或配置问题：明确失败，不静默当作"无文字"。
                AdapterError::config(format!(
                    "OCR 引擎不可用（{} 无法启动）: {source}",
                    self.program
                ))
            })?;
        if !output.status.success() {
            return Err(AdapterError::permanent(format!(
                "OCR 识别失败（{} 退出码 {:?}）: {}",
                self.program,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }
}

/// 当前环境的默认 OCR 引擎：Tesseract（程序路径由挂载 spec 决定，缺省 PATH）。
pub fn ocr_engine() -> TesseractOcr {
    TesseractOcr::new("tesseract")
}

/// OCR 通过：对数据面里的每个图片片段（`Body::Staged` 且声明为图片）应用每个挂载，
/// 追加识别文本派生 `PutFacet`。`engine_of` 按挂载构造引擎（程序路径等配置取自
/// 挂载 spec）；`resolve` 把暂存部件解析成磁盘路径（内容寻址暂存的文件已在盘上，
/// 直接引用，不复制）。原始 op 原样保留。
pub fn apply_ocr(
    ops: &[Op],
    mounts: &[Mount],
    memo: &mut Memo,
    engine_of: &dyn Fn(&Mount) -> Box<dyn OcrEngine>,
    resolve: &dyn Fn(&str, u64) -> Option<std::path::PathBuf>,
) -> Result<(Vec<Op>, usize), AdapterError> {
    let mut derived = Vec::new();
    let mut processed = 0usize;
    for op in ops {
        let Op::PutFacet {
            target,
            kind,
            variant,
            body: Body::Staged { part },
            media_type,
            ..
        } = op
        else {
            continue;
        };
        let is_image = media_type
            .as_deref()
            .is_some_and(|media| media.starts_with("image/"))
            || matches!(kind, FacetKind::Image);
        if !is_image {
            continue;
        }
        let unit = json!({
            "target": target,
            "kind": kind,
            "variant": variant,
            "sha256": part.sha256(),
            "size": part.size(),
        });
        for mount in mounts {
            let meta = ocr_meta();
            if already_done(memo, meta.id, meta.version, &unit, &mount.spec) {
                continue;
            }
            let language = match mount.spec.get("language") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
                Some(Value::String(_)) => None,
                Some(_) => {
                    return Err(AdapterError::config("ocr 的 language 必须是字符串"));
                }
            };
            let Some(path) = resolve(part.sha256(), part.size()) else {
                return Err(AdapterError::permanent(format!(
                    "图片部件不在暂存区: {}",
                    part.sha256()
                )));
            };
            processed += 1;
            match engine_of(mount).recognize(&path, language) {
                Ok(Some(text)) => {
                    memo.successes
                        .insert(Memo::key(meta.id, meta.version, &unit, &mount.spec));
                    derived.push(Op::PutFacet {
                        target: target.clone(),
                        kind: derived_kind(&mount.spec)?,
                        variant: Some(ocr_variant(&mount.spec)),
                        body: Body::Text { text },
                        media_type: Some("text/plain".into()),
                        lineage: lineage_of(op, &mount.spec),
                    });
                }
                // 引擎不可用或未识别到文字：成功且无产出，记入备忘。
                Ok(None) => {
                    memo.successes
                        .insert(Memo::key(meta.id, meta.version, &unit, &mount.spec));
                }
                // 失败不记备忘：同一单元被再次挂载时可重试。
                Err(error) => return Err(error),
            }
        }
    }
    Ok((derived, processed))
}

/// OCR 处理器元信息（组件 id 固定 `content-processor/ocr`）。
fn ocr_meta() -> &'static ProcessorMeta {
    static META: ProcessorMeta = ProcessorMeta {
        id: "content-processor/ocr",
        version: "1",
    };
    &META
}

/// OCR 派生片段 variant：spec 里 `variant`（默认 `ocr`）。
fn ocr_variant(spec: &Value) -> String {
    spec.get("variant")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("ocr")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use strata_model::{LineageInput, Target};

    fn text_facet(text: &str, target: &str) -> Op {
        Op::PutFacet {
            target: Target::Alias(target.to_string()),
            kind: FacetKind::Text,
            variant: None,
            body: Body::Text { text: text.into() },
            media_type: Some("text/plain".into()),
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "h0".into(),
            },
        }
    }

    fn spec(min_length: u64, keywords: &[&str]) -> Value {
        json!({"minLength": min_length, "dropKeywords": keywords})
    }

    #[test]
    fn text_filter_drops_keyword_and_short_lines() {
        let out = TextFilter
            .process("广告推销忽略\n正文第一行\n短\n正文第二行", &spec(3, &["广告"]))
            .unwrap();
        assert_eq!(out.as_deref(), Some("正文第一行\n正文第二行"));
    }

    #[test]
    fn text_filter_empty_output_is_none() {
        let out = TextFilter.process("广告\n推销", &spec(0, &["广告", "推销"])).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn text_filter_rejects_bad_spec() {
        let error = TextFilter.process("abc", &json!("nope")).unwrap_err();
        assert!(error.to_string().contains("必须是对象"), "{error}");
        let error = TextFilter.process("abc", &json!({"minLength": -1})).unwrap_err();
        assert!(error.to_string().contains("minLength"), "{error}");
    }

    #[test]
    fn unknown_processor_id_is_config_error() {
        let mut memo = Memo::new();
        let mounts = vec![Mount {
            id: "content-processor/nope".into(),
            spec: Value::Null,
        }];
        let ops = vec![text_facet("正文", "p0")];
        let error = apply(&ops, &mounts, &mut memo).unwrap_err();
        assert!(error.to_string().contains("未知内容处理器"), "{error}");
    }

    #[test]
    fn appends_derived_facet_and_keeps_original() {
        let mut memo = Memo::new();
        let mounts = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: spec(3, &["广告"]),
        }];
        let ops = vec![
            text_facet("广告推销\n正文第一行\n短", "p0"),
            Op::Ensure {
                identity: "url:https://example.com".into(),
                ns: None,
                labels: Default::default(),
                r#as: "p0".into(),
            },
        ];
        let (derived, processed) = apply(&ops, &mounts, &mut memo).unwrap();
        assert_eq!(processed, 1);
        assert_eq!(derived.len(), 1);
        // 原始 op 原样保留，派生片段只追加。
        assert_eq!(ops.len(), 2);
        let Op::PutFacet { variant, body, .. } = &derived[0] else {
            panic!("派生片段必须是 PutFacet");
        };
        assert_eq!(variant.as_deref(), Some("text-filter"));
        let Body::Text { text } = body else {
            panic!("派生片段必须是文本");
        };
        assert_eq!(text, "正文第一行");
        // 血缘参数哈希取有效参数。
        let Op::PutFacet { lineage, .. } = &derived[0] else {
            unreachable!();
        };
        assert_eq!(lineage.params_hash, params_hash(&spec(3, &["广告"])));
    }

    #[test]
    fn duplicate_mount_skips_by_unit_hash_and_counts_as_success() {
        let mut memo = Memo::new();
        let mounts = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: spec(3, &["广告"]),
        }];
        let ops = vec![text_facet("广告推销\n正文第一行", "p0")];
        let (first, processed_first) = apply(&ops, &mounts, &mut memo).unwrap();
        assert_eq!(processed_first, 1);
        assert_eq!(first.len(), 1);
        // 同一输入单元 + 同参数：重复挂载按哈希跳过，视为成功。
        let (second, processed_second) = apply(&ops, &mounts, &mut memo).unwrap();
        assert_eq!(processed_second, 0);
        assert!(second.is_empty());
        // 不同参数是不同挂载，重新处理。
        let other = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: spec(10, &[]),
        }];
        let (_, processed_other) = apply(&ops, &other, &mut memo).unwrap();
        assert_eq!(processed_other, 1);
    }

    #[test]
    fn failed_unit_is_not_memoized_so_remount_retries() {
        let mut memo = Memo::new();
        let bad = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: json!("坏 spec"),
        }];
        let good = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: spec(0, &[]),
        }];
        let ops = vec![text_facet("正文", "p0")];
        assert!(apply(&ops, &bad, &mut memo).is_err());
        // 失败不得导致跳过：随后的有效挂载仍处理该单元。
        let (derived, processed) = apply(&ops, &good, &mut memo).unwrap();
        assert_eq!(processed, 1);
        assert_eq!(derived.len(), 1);
    }

    #[test]
    fn empty_output_is_success_and_skips_remount() {
        let mut memo = Memo::new();
        let mounts = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: spec(0, &["全部"]),
        }];
        let ops = vec![text_facet("全部过滤", "p0")];
        let (derived, processed) = apply(&ops, &mounts, &mut memo).unwrap();
        assert_eq!(processed, 1);
        assert!(derived.is_empty(), "产出为空不追加片段");
        let (_, processed_again) = apply(&ops, &mounts, &mut memo).unwrap();
        assert_eq!(processed_again, 0, "空产出已记成功，重复挂载跳过");
    }

    #[test]
    fn non_text_facets_are_ignored() {
        let mut memo = Memo::new();
        let mounts = vec![Mount {
            id: "content-processor/text-filter".into(),
            spec: Value::Null,
        }];
        let ops = vec![Op::PutFacet {
            target: Target::Alias("p0".into()),
            kind: FacetKind::Json,
            variant: None,
            body: Body::Struct {
                value: json!({"k": 1}),
            },
            media_type: None,
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "h0".into(),
            },
        }];
        let (derived, processed) = apply(&ops, &mounts, &mut memo).unwrap();
        assert_eq!(processed, 0);
        assert!(derived.is_empty());
    }

    // ---- OCR（content-processor/ocr，图片单元）----

    use std::path::PathBuf;
    use strata_model::PartRef;

    /// 可控的 OCR 引擎桩：按模式决定产出/失败/不可用。
    struct FakeOcr {
        mode: FakeMode,
    }

    #[derive(Clone, Copy)]
    enum FakeMode {
        Text(&'static str),
        Nothing,
        Fail,
    }

    /// 引擎工厂助手：`apply_ocr` 按挂载构造引擎，桩测试用同一模式填充。
    fn fake_factory(
        mode: FakeMode,
    ) -> impl Fn(&Mount) -> Box<dyn OcrEngine> {
        move |_mount| Box::new(FakeOcr { mode })
    }

    impl OcrEngine for FakeOcr {
        fn recognize(
            &self,
            _image_path: &std::path::Path,
            _language: Option<&str>,
        ) -> Result<Option<String>, AdapterError> {
            match self.mode {
                FakeMode::Text(text) => Ok(Some(text.to_string())),
                FakeMode::Nothing => Ok(None),
                FakeMode::Fail => Err(AdapterError::permanent("引擎崩溃")),
            }
        }
    }

    fn staged_image(sha: &str, media: Option<&str>) -> Op {
        Op::PutFacet {
            target: Target::Alias("p0".into()),
            kind: FacetKind::Image,
            variant: Some("image".into()),
            body: Body::Staged {
                part: PartRef::new(sha, 7).unwrap(),
            },
            media_type: media.map(str::to_string),
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "h0".into(),
            },
        }
    }

    fn ocr_mount(language: Option<&str>) -> Mount {
        Mount {
            id: "content-processor/ocr".into(),
            spec: match language {
                Some(lang) => json!({"language": lang}),
                None => json!({}),
            },
        }
    }

    fn resolver(found: bool) -> impl Fn(&str, u64) -> Option<PathBuf> {
        move |_sha, _size| found.then(|| PathBuf::from("C:/staging/img.png"))
    }

    #[test]
    fn ocr_appends_derived_text_facet_and_keeps_original() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("a".repeat(64).as_str(), Some("image/png"))];
        let (derived, processed) = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &fake_factory(FakeMode::Text("识别出的文字")),
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed, 1);
        assert_eq!(derived.len(), 1);
        // 原始 op 原样保留，派生只追加。
        assert_eq!(ops.len(), 1);
        let Op::PutFacet {
            variant, body, ..
        } = &derived[0]
        else {
            panic!("派生必须是 PutFacet");
        };
        assert_eq!(variant.as_deref(), Some("ocr"));
        assert_eq!(*body, Body::Text { text: "识别出的文字".into() });
        let (Op::PutFacet { target: source, .. }, Op::PutFacet { target: out, .. }) =
            (&ops[0], &derived[0])
        else {
            unreachable!();
        };
        assert_eq!(source, out, "派生片段与输入片段落到同一资源");
    }

    #[test]
    fn ocr_ignores_non_image_facets() {
        let mut memo = Memo::new();
        let ops = vec![
            // 非 Staged / 非图片声明都不进 OCR。
            text_facet("正文", "p0"),
            {
                let mut op = staged_image("b".repeat(64).as_str(), None);
                if let Op::PutFacet { kind, .. } = &mut op {
                    *kind = FacetKind::Text;
                }
                op
            },
        ];
        let (derived, processed) = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &fake_factory(FakeMode::Text("不应出现")),
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed, 0);
        assert!(derived.is_empty());
    }

    #[test]
    fn ocr_treats_image_kind_without_media_type_as_image() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("c".repeat(64).as_str(), None)];
        let (derived, _) = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &fake_factory(FakeMode::Text("图中文字")),
            &resolver(true),
        )
        .unwrap();
        assert_eq!(derived.len(), 1);
    }

    #[test]
    fn ocr_duplicate_mount_skips_by_unit_hash_and_params() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("d".repeat(64).as_str(), Some("image/jpeg"))];
        let engine = fake_factory(FakeMode::Text("一次就够"));
        let (_, processed_first) = apply_ocr(
            &ops,
            &[ocr_mount(Some("zh-Hans-CN"))],
            &mut memo,
            &engine,
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed_first, 1);
        // 同输入单元 + 同参数：重复挂载按哈希跳过并视为成功（引擎桩不该被再调）。
        let (_, processed_again) = apply_ocr(
            &ops,
            &[ocr_mount(Some("zh-Hans-CN"))],
            &mut memo,
            &engine,
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed_again, 0);
        // 不同参数是不同挂载，重新处理。
        let (_, processed_other) = apply_ocr(
            &ops,
            &[ocr_mount(Some("en-US"))],
            &mut memo,
            &engine,
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed_other, 1);
    }

    #[test]
    fn ocr_engine_unavailable_is_success_with_no_output() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("e".repeat(64).as_str(), Some("image/png"))];
        let engine = fake_factory(FakeMode::Nothing); // 引擎无匹配识别语言 → None
        let (derived, processed) = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &engine,
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed, 1);
        assert!(derived.is_empty(), "不可用不产生派生片段");
        // 成功无产出也记入备忘：重复挂载按成功跳过。
        let (_, processed_again) = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &engine,
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed_again, 0);
    }

    #[test]
    fn ocr_engine_failure_is_not_memoized_so_remount_retries() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("f".repeat(64).as_str(), Some("image/png"))];
        let broken = fake_factory(FakeMode::Fail);
        assert!(apply_ocr(&ops, &[ocr_mount(None)], &mut memo, &broken, &resolver(true)).is_err());
        // 失败不得导致跳过：随后的有效引擎仍处理该单元。
        let good = fake_factory(FakeMode::Text("重试成功"));
        let (derived, processed) = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &good,
            &resolver(true),
        )
        .unwrap();
        assert_eq!(processed, 1);
        assert_eq!(derived.len(), 1);
    }

    #[test]
    fn ocr_missing_staged_part_is_permanent_error() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("9".repeat(64).as_str(), Some("image/png"))];
        let error = apply_ocr(
            &ops,
            &[ocr_mount(None)],
            &mut memo,
            &fake_factory(FakeMode::Text("不应到达")),
            &resolver(false),
        )
        .unwrap_err();
        assert!(error.to_string().contains("暂存区"), "{error}");
    }

    #[test]
    fn ocr_rejects_bad_language_param() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("7".repeat(64).as_str(), Some("image/png"))];
        let mounts = vec![Mount {
            id: "content-processor/ocr".into(),
            spec: json!({"language": 42}),
        }];
        let error = apply_ocr(
            &ops,
            &mounts,
            &mut memo,
            &fake_factory(FakeMode::Text("不应到达")),
            &resolver(true),
        )
        .unwrap_err();
        assert!(error.to_string().contains("language"), "{error}");
    }

    #[test]
    fn ocr_derived_kind_and_variant_follow_spec() {
        let mut memo = Memo::new();
        let ops = vec![staged_image("8".repeat(64).as_str(), Some("image/png"))];
        let mounts = vec![Mount {
            id: "content-processor/ocr".into(),
            spec: json!({"kind": "md", "variant": "screenshot-ocr"}),
        }];
        let (derived, _) = apply_ocr(
            &ops,
            &mounts,
            &mut memo,
            &fake_factory(FakeMode::Text("文字")),
            &resolver(true),
        )
        .unwrap();
        let Op::PutFacet { kind, variant, .. } = &derived[0] else {
            panic!();
        };
        assert_eq!(*kind, FacetKind::Md);
        assert_eq!(variant.as_deref(), Some("screenshot-ocr"));
    }

    // ---- Tesseract 引擎（跨平台调用契约）----

    #[test]
    fn tesseract_args_follow_the_cli_contract() {
        // `tesseract <image> stdout [-l lang] [--psm n]`：三大平台同一命令形。
        let path = std::path::Path::new("/tmp/img.png");
        assert_eq!(
            tesseract_args(path, None, None),
            vec!["/tmp/img.png".to_string(), "stdout".to_string()]
        );
        assert_eq!(
            tesseract_args(path, Some("eng"), None),
            vec![
                "/tmp/img.png".to_string(),
                "stdout".to_string(),
                "-l".to_string(),
                "eng".to_string()
            ]
        );
        assert_eq!(
            tesseract_args(path, Some("chi_sim"), Some(6)),
            vec![
                "/tmp/img.png".to_string(),
                "stdout".to_string(),
                "-l".to_string(),
                "chi_sim".to_string(),
                "--psm".to_string(),
                "6".to_string()
            ]
        );
        // 空语言等价于不指定。
        assert_eq!(
            tesseract_args(path, Some(""), None),
            vec!["/tmp/img.png".to_string(), "stdout".to_string()]
        );
    }

    #[test]
    fn tesseract_engine_is_built_from_spec() {
        let engine = TesseractOcr::from_spec(&json!({"program": "/usr/local/bin/tesseract", "psm": 6}))
            .unwrap();
        assert_eq!(
            tesseract_args(std::path::Path::new("i.png"), None, engine.psm),
            vec![
                "i.png".to_string(),
                "stdout".to_string(),
                "--psm".to_string(),
                "6".to_string()
            ]
        );
        // 缺省：PATH 上的 tesseract，不指定 psm。
        let default_engine = TesseractOcr::from_spec(&json!({})).unwrap();
        assert_eq!(default_engine.psm, None);
        // 坏 spec 明确报配置错误。
        assert!(TesseractOcr::from_spec(&json!({"program": 42})).is_err());
        assert!(TesseractOcr::from_spec(&json!({"program": ""})).is_err());
        assert!(TesseractOcr::from_spec(&json!({"psm": "6"})).is_err());
    }

    #[test]
    fn missing_tesseract_engine_fails_loudly_not_silently() {
        // 找不到引擎必须明确失败（配置错误），不得静默当作"无文字"或平台降级。
        let error = TesseractOcr::new("modalprism-no-such-ocr-engine-xyz")
            .recognize(std::path::Path::new("i.png"), Some("eng"))
            .unwrap_err();
        assert!(error.to_string().contains("OCR 引擎不可用"), "{error}");
        assert_eq!(error.class(), strata_model::ErrorClass::Config);
    }

    #[test]
    fn tesseract_stub_runs_the_same_contract_on_every_desktop_os() {
        // 用当前平台的标配解释器做假 tesseract：验证真实子进程路径——
        // stdout 文本 → Some(trim 后文本)；退出码非 0 → 明确错误；空输出 → None。
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("img.png");
        std::fs::write(&image, b"fake").unwrap();

        #[cfg(windows)]
        fn stub(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
            let path = dir.join("fake-tesseract.cmd");
            std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
            path
        }
        #[cfg(not(windows))]
        fn stub(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("fake-tesseract.sh");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        // 有输出：stdout 文本 trim 后返回。
        let engine = TesseractOcr::new(
            stub(dir.path(), "echo  图中文字 ").to_string_lossy().into_owned(),
        );
        assert_eq!(
            engine.recognize(&image, None).unwrap(),
            Some("图中文字".to_string())
        );

        // 空输出：成功且无产出。
        let empty = TesseractOcr::new(stub(dir.path(), "exit /b 0").to_string_lossy().into_owned());
        #[cfg(not(windows))]
        let empty = TesseractOcr::new(stub(dir.path(), "").to_string_lossy().into_owned());
        assert_eq!(empty.recognize(&image, None).unwrap(), None);

        // 退出码非 0：明确失败，带 stderr。
        #[cfg(windows)]
        let failing = TesseractOcr::new(
            stub(dir.path(), "echo read error 1>&2\r\nexit /b 1").to_string_lossy().into_owned(),
        );
        #[cfg(not(windows))]
        let failing = TesseractOcr::new(
            stub(dir.path(), "echo read error >&2\nexit 1").to_string_lossy().into_owned(),
        );
        let error = failing.recognize(&image, None).unwrap_err();
        assert!(error.to_string().contains("OCR 识别失败"), "{error}");
        assert!(error.to_string().contains("read error"), "{error}");
    }
}
