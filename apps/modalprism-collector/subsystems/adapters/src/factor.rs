//! 因子（测度）层。
//!
//! 因子是测度，不是内容处理器：不做 OCR/转写/情感一类理解工作，只对规范化片段
//! 产出确定性的、可比的测度记录。与处理器同走管道挂载（`factor/<name>`），在
//! 数据面产出之后运行；只追加派生片段，不改写原始信封、规范单元或他人片段。
//!
//! 测度记录（派生片段正文，`kind: struct`）必含：
//! - `factor` / `version`：因子组件 id 与版本
//! - `name`：因子名称
//! - `asOf`：截至时点（RFC3339 带偏移），取自信封取回时间（`transport.started_at`），
//!   不得读运行墙钟
//! - `value` / `label` / `unit`：可比数值与受控标签
//! - `inputHash`：输入哈希（组件 id、版本、输入单元、有效参数的规范化哈希）
//! - `evidence`：依据引用（指向规范单元，经血缘回溯原始信封）
//!
//! 依据不足或缺必填字段明确报错，不得用空值冒充已测度。去重语义与处理器一致：
//! 同一输入哈希的成功产出按成功跳过；失败不记备忘，可重试。

use serde_json::{Value, json};

use crate::error::AdapterError;
use crate::textutil::params_hash;
use strata_model::{Body, FacetKind, Op};

/// 因子元信息。id 约定 `factor/<name>`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactorMeta {
    pub id: &'static str,
    pub version: &'static str,
    pub name: &'static str,
}

/// 确定性内置因子契约：输入文本与有效参数，产出数值与受控标签。
pub trait Measure: Send + Sync {
    fn meta(&self) -> &'static FactorMeta;
    fn measure(&self, text: &str, spec: &Value) -> Result<(f64, &'static str), AdapterError>;
}

/// 内置因子 `factor/text-density`：文本密度——非空白字符占全部字符的比例。
/// 受控标签按比例分带：`< 0.2` → `sparse`，`< 0.6` → `normal`，否则 `dense`。
/// 输入必须是 kind=text 的片段；依据不足（空文本）明确报错。
pub struct TextDensity;

impl Measure for TextDensity {
    fn meta(&self) -> &'static FactorMeta {
        static META: FactorMeta = FactorMeta {
            id: "factor/text-density",
            version: "1",
            name: "text-density",
        };
        &META
    }

    fn measure(&self, text: &str, spec: &Value) -> Result<(f64, &'static str), AdapterError> {
        match spec {
            Value::Null | Value::Object(_) => {}
            _ => return Err(AdapterError::config("text-density 的 spec 必须是对象")),
        }
        let total = text.chars().count();
        if total == 0 || text.chars().all(char::is_whitespace) {
            // 空文本与纯空白没有可测对象：不得用 0 冒充已测度。
            return Err(AdapterError::permanent("依据不足：文本片段为空，无法测度"));
        }
        let solid = text.chars().filter(|c| !c.is_whitespace()).count();
        let density = solid as f64 / total as f64;
        let label = if density < 0.2 {
            "sparse"
        } else if density < 0.6 {
            "normal"
        } else {
            "dense"
        };
        Ok((density, label))
    }
}

/// 内置因子注册表。只做这一个最小因子，落实测度合约；不做其它示例。
pub fn measure(id: &str) -> Option<&'static dyn Measure> {
    static TEXT_DENSITY: TextDensity = TextDensity;
    match id {
        "factor/text-density" => Some(&TEXT_DENSITY),
        _ => None,
    }
}

/// 因子挂载（复用处理器同款项形：`run` + `spec` 按组件 id 取有效参数）。
#[derive(Clone, Debug)]
pub struct FactorMount {
    pub id: String,
    pub spec: Value,
}

/// 去重备忘：只记成功。失败/取消/未完成不得跳过。与处理器共用同款键规则。
#[derive(Default)]
pub struct FactorMemo {
    successes: std::collections::HashSet<String>,
}

impl FactorMemo {
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

/// 因子通过：对数据面里每个 `kind=text` 的片段应用每个挂载，追加测度记录片段。
/// `as_of` 是信封取回时间（RFC3339 带偏移）；缺省（信封无声明时间）明确报错，
/// 不得读运行墙钟。原始 op 原样保留。
pub fn apply(
    ops: &[Op],
    mounts: &[FactorMount],
    memo: &mut FactorMemo,
    as_of: Option<&str>,
) -> Result<(Vec<Op>, usize), AdapterError> {
    let Some(as_of) = as_of.filter(|value| !value.is_empty()) else {
        if mounts.is_empty() {
            return Ok((Vec::new(), 0));
        }
        return Err(AdapterError::permanent(
            "依据不足：信封没有声明取回时间，因子缺 as-of，不得测度",
        ));
    };
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
        // 按 kind 分发：text-density 消费 kind=text；标签不参与调度。
        if *kind != FacetKind::Text {
            continue;
        }
        let unit = json!({
            "target": target,
            "kind": kind,
            "variant": variant,
            "text": text,
        });
        for mount in mounts {
            let factor = measure(&mount.id).ok_or_else(|| {
                AdapterError::config(format!("未知因子: {}", mount.id))
            })?;
            let meta = factor.meta();
            let key = FactorMemo::key(meta.id, meta.version, &unit, &mount.spec);
            if memo.successes.contains(&key) {
                // 同一输入哈希已有成功产出：按成功跳过。
                continue;
            }
            processed += 1;
            let (value, label) = factor.measure(text, &mount.spec)?;
            // 成功才记备忘：失败/取消/未完成不得跳过。
            memo.successes.insert(key);
            derived.push(Op::PutFacet {
                target: target.clone(),
                kind: FacetKind::Struct,
                variant: Some(meta.name.to_string()),
                body: Body::Struct {
                    value: json!({
                        "factor": meta.id,
                        "version": meta.version,
                        "name": meta.name,
                        "asOf": as_of,
                        "value": value,
                        "label": label,
                        "unit": "ratio",
                        "inputHash": FactorMemo::key(meta.id, meta.version, &unit, &mount.spec),
                        "evidence": {
                            "target": target,
                            "kind": kind,
                            "variant": variant,
                        },
                    }),
                },
                media_type: Some("application/json".into()),
                lineage: lineage_of(op, &mount.spec),
            });
        }
    }
    Ok((derived, processed))
}

/// 测度血缘：沿用输入片段的血缘输入（可回溯原始信封），参数哈希取有效参数。
fn lineage_of(op: &Op, spec: &Value) -> strata_model::LineageInput {
    let Op::PutFacet { lineage, .. } = op else {
        unreachable!("lineage_of 只对 PutFacet 调用");
    };
    strata_model::LineageInput {
        inputs: lineage.inputs.clone(),
        params_hash: params_hash(spec),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use strata_model::{LineageInput, Target};

    const AS_OF: &str = "2026-11-14T08:00:00+08:00";

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

    fn mount(spec: Value) -> FactorMount {
        FactorMount {
            id: "factor/text-density".into(),
            spec,
        }
    }

    #[test]
    fn density_is_deterministic_with_controlled_label() {
        let (value, label) = TextDensity.measure("正文内容没有空白", &json!({})).unwrap();
        assert!((value - 1.0).abs() < f64::EPSILON);
        assert_eq!(label, "dense");
        // 确定性：同样输入两次，结果一致。
        let again = TextDensity.measure("正文内容没有空白", &json!({})).unwrap();
        assert_eq!((value, label), again);
        // 6 字符 3 非空白 → 0.5，落在 normal 带。
        let (half, label) = TextDensity.measure("ab   e", &json!({})).unwrap();
        assert!((half - 0.5).abs() < 1e-9);
        assert_eq!(label, "normal");
        // 1 非空白 + 9 空白 → 0.1，落在 sparse 带。
        let (thin, label) = TextDensity.measure("a         ", &json!({})).unwrap();
        assert!((thin - 0.1).abs() < 1e-9);
        assert_eq!(label, "sparse");
    }

    #[test]
    fn empty_input_is_an_explicit_error_not_a_zero_value() {
        let error = TextDensity.measure("", &json!({})).unwrap_err();
        assert!(error.to_string().contains("依据不足"), "{error}");
        let error = TextDensity.measure("   \n\t", &json!({})).unwrap_err();
        assert!(error.to_string().contains("依据不足"), "{error}");
    }

    #[test]
    fn record_carries_full_contract() {
        let mut memo = FactorMemo::new();
        let ops = vec![text_facet("正文内容", "p0")];
        let (derived, processed) = apply(&ops, &[mount(json!({}))], &mut memo, Some(AS_OF)).unwrap();
        assert_eq!(processed, 1);
        let Op::PutFacet {
            target,
            kind,
            variant,
            body: Body::Struct { value },
            media_type,
            lineage,
        } = &derived[0]
        else {
            panic!("测度记录必须是 Struct 片段");
        };
        // 落到同一资源；原始 op 原样保留。
        assert_eq!(*target, Target::Alias("p0".into()));
        assert_eq!(ops.len(), 1);
        assert_eq!(*kind, FacetKind::Struct);
        assert_eq!(variant.as_deref(), Some("text-density"));
        assert_eq!(media_type.as_deref(), Some("application/json"));
        // 合约字段：组件 id、版本、名称、as-of、可比数值、受控标签、输入哈希、依据。
        assert_eq!(value["factor"], "factor/text-density");
        assert_eq!(value["version"], "1");
        assert_eq!(value["name"], "text-density");
        assert_eq!(value["asOf"], AS_OF);
        assert!(value["value"].is_number());
        assert!(matches!(
            value["label"].as_str(),
            Some("sparse") | Some("normal") | Some("dense")
        ));
        assert_eq!(value["unit"], "ratio");
        assert!(value["inputHash"].is_string());
        assert_eq!(value["evidence"]["target"]["alias"], "p0");
        assert_eq!(value["evidence"]["kind"], "text");
        // 血缘沿用输入片段的 span 引用，参数哈希取有效参数。
        assert_eq!(lineage.params_hash, params_hash(&json!({})));
        assert!(lineage.inputs.is_empty());
    }

    #[test]
    fn as_of_must_be_declared_not_a_wall_clock() {
        let mut memo = FactorMemo::new();
        let ops = vec![text_facet("正文", "p0")];
        // 挂载了因子但信封没有声明时间：明确报错，不得拿运行墙钟冒充 as-of。
        let error = apply(&ops, &[mount(json!({}))], &mut memo, None).unwrap_err();
        assert!(error.to_string().contains("as-of"), "{error}");
        let error = apply(&ops, &[mount(json!({}))], &mut memo, Some("")).unwrap_err();
        assert!(error.to_string().contains("as-of"), "{error}");
        // 未挂载时不跑：无时间也不报错。
        let (derived, processed) = apply(&ops, &[], &mut memo, None).unwrap();
        assert_eq!((processed, derived.len()), (0, 0));
    }

    #[test]
    fn duplicate_mount_skips_by_input_hash_and_counts_as_success() {
        let mut memo = FactorMemo::new();
        let ops = vec![text_facet("同一份正文", "p0")];
        let (first, processed_first) = apply(&ops, &[mount(json!({}))], &mut memo, Some(AS_OF)).unwrap();
        assert_eq!((processed_first, first.len()), (1, 1));
        // 同一输入哈希：成功产出复用为 skipped，视为成功。
        let (second, processed_again) = apply(&ops, &[mount(json!({}))], &mut memo, Some(AS_OF)).unwrap();
        assert_eq!((processed_again, second.len()), (0, 0));
        // 不同参数是不同输入哈希，重新测度。
        let (_, processed_other) =
            apply(&ops, &[mount(json!({"band": "自定义不影响密度"}))], &mut memo, Some(AS_OF)).unwrap();
        assert_eq!(processed_other, 1);
    }

    #[test]
    fn failure_is_not_memoized_so_remount_retries() {
        let mut memo = FactorMemo::new();
        let ops = vec![text_facet("", "p0")]; // 空文本 → 依据不足
        assert!(apply(&ops, &[mount(json!({}))], &mut memo, Some(AS_OF)).is_err());
        // 失败不得导致跳过：同一单元换个可测输入仍会被测（此处验证备忘为空）。
        assert!(memo.successes.is_empty());
    }

    #[test]
    fn unknown_factor_id_is_config_error() {
        let mut memo = FactorMemo::new();
        let ops = vec![text_facet("正文", "p0")];
        let error = apply(
            &ops,
            &[FactorMount {
                id: "factor/nope".into(),
                spec: json!({}),
            }],
            &mut memo,
            Some(AS_OF),
        )
        .unwrap_err();
        assert!(error.to_string().contains("未知因子"), "{error}");
    }

    #[test]
    fn dispatch_is_by_kind_not_by_labels() {
        let mut memo = FactorMemo::new();
        // 非 text 片段（struct）不进测度：kind 分发，不看标签。
        let ops = vec![Op::PutFacet {
            target: Target::Alias("p0".into()),
            kind: FacetKind::Json,
            variant: Some("discovery".into()),
            body: Body::Struct { value: json!({"labels": ["dense"]}) },
            media_type: Some("application/json".into()),
            lineage: LineageInput {
                inputs: Vec::new(),
                params_hash: "h0".into(),
            },
        }];
        let (derived, processed) = apply(&ops, &[mount(json!({}))], &mut memo, Some(AS_OF)).unwrap();
        assert_eq!((processed, derived.len()), (0, 0));
    }
}
