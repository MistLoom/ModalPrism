//! 按路由执行适配器，再收成变更集。

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::{Map, Value};
use strata_model::{
    FacetKind, FacetRef, PartId, PartRole, Producer, Rid, SourceId, SpanRef, TaskReason, Timestamp,
};

use crate::builtin::{PartDetection, detection_body};
use crate::compile::{
    Admission, Batch, Compiled, Emitted, EmittedDiscovery, IdentityView, MapIdentities,
};
use crate::error::AdapterError;
use crate::model::{
    AdapterInput, Detection, DraftBody, FacetDraft, Feedback, InputSelector, ReadonlyView,
    RenderMode, SourceView,
};
use crate::registry::Registry;
use crate::prism::Prism;
use crate::route::{PartMatch, Step, candidates, needs_detection, schedule};
use crate::textutil::{essence, params_hash};

#[derive(Clone, Debug)]
pub struct PartBytes {
    pub part_id: PartId,
    pub role: PartRole,
    pub declared: String,
    pub charset: Option<String>,
    pub bytes: Bytes,
    pub facet: Option<FacetRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectorChoice {
    Basic,
    /// RawPrism 识别器；引擎未注入时自动退回 `Basic`。
    Prism,
    Off,
    Plugin(String),
}

pub struct RunRequest {
    pub parts: Vec<PartBytes>,
    pub pipeline: Vec<crate::route::Rule>,
    pub reason: TaskReason,
    pub source: SourceView,
    pub resource: Option<Rid>,
    /// 文档资源在变更集里的别名（与 `Op::Ensure.as` 对应）。
    ///
    /// 临时采集与首次采集还没有 rid：调用方先 `Ensure` 一个别名，再把内容面落到该别名，
    /// 避免为此提前生成 rid 或空写一份资源。
    pub resource_alias: Option<String>,
    pub origin: Option<Rid>,
    pub locator: strata_model::Locator,
    pub detector: DetectorChoice,
    pub admission: Admission,
    pub now: Timestamp,
    pub initial: bool,
    pub producer: Producer,
    pub specs: Map<String, Value>,
    pub identities: Arc<dyn IdentityView>,
    /// 结构化识别引擎（RawPrism）。None 表示未装载，全链路按基础检测降级。
    pub prism: Option<Arc<dyn Prism>>,
}

#[derive(Debug)]
pub struct RunOutput {
    pub changeset: strata_model::ChangeSet,
    pub follows: Vec<crate::compile::FollowRequest>,
    pub feedback: Vec<AdapterFeedback>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterFeedback {
    pub adapter_id: String,
    pub part_id: Option<String>,
    pub feedback: Feedback,
}

struct Executed {
    view: Option<ReadonlyView>,
    kind: Option<String>,
    variant: Option<String>,
}

pub async fn run(registry: &Registry, request: RunRequest) -> Result<RunOutput, AdapterError> {
    let pass = detect_once(registry, &request).await?;
    let detections = pass
        .as_ref()
        .map(|item| &item.by_part)
        .cloned()
        .unwrap_or_default();
    let matches = part_matches(&request.parts, &detections);
    let mut nodes = candidates(&matches, &request.pipeline, request.reason, &request.specs)?;
    nodes.retain(|node| consumes_ok(registry, node, &request, &detections));
    for node in &nodes {
        ensure_deterministic(registry, &node.adapter_id)?;
        validate_spec(registry, &node.adapter_id, &node.spec)?;
    }
    let probed = probe_nodes(registry, nodes, &request, &detections).await?;
    let steps = schedule(probed)?;
    execute(registry, request, pass, steps).await
}

struct DetectionPass {
    by_part: BTreeMap<String, Detection>,
    regions: Vec<crate::model::RegionHit>,
    detector_id: String,
}

async fn detect_once(
    registry: &Registry,
    request: &RunRequest,
) -> Result<Option<DetectionPass>, AdapterError> {
    // 识别器就绪时总是检测：即使声明里没有 traits 规则，也要留下 detect/regions 面，
    // 内容库据此展示“这份内容由哪些类型构成”。
    let prism_ready = request.prism.is_some() && matches!(request.detector, DetectorChoice::Prism);
    let empty = BTreeMap::new();
    let matches = part_matches(&request.parts, &empty);
    if !prism_ready && !needs_detection(&matches, &request.pipeline, request.reason) {
        return Ok(None);
    }
    let detector_id = match &request.detector {
        DetectorChoice::Off => return Ok(None),
        DetectorChoice::Basic => "detector/basic".to_string(),
        DetectorChoice::Prism if prism_ready => "detector/prism".to_string(),
        DetectorChoice::Prism => "detector/basic".to_string(),
        DetectorChoice::Plugin(id) => id.clone(),
    };
    let Some(detector) = registry.detector(&detector_id) else {
        return Err(AdapterError::config(format!("检测器不存在: {detector_id}")));
    };
    // 基础检测器只认 json-ld 区域；识别器返回全量区域。
    let wanted: Vec<String> = if detector_id == "detector/prism" {
        Vec::new()
    } else {
        vec!["json-ld".into()]
    };
    let mut by_part = BTreeMap::new();
    let mut regions = Vec::new();
    for part in &request.parts {
        let view = raw_view(part);
        let declared = crate::model::Declared {
            media_type: part.declared.clone(),
            charset: part.charset.clone(),
            role: part.role.clone(),
        };
        let detection = detector.detect(&view, &declared).await?;
        let mut hits = detector.regions(&view, &wanted).await;
        for hit in &mut hits {
            hit.part_id = part.part_id.as_str().to_string();
        }
        regions.extend(hits);
        by_part.insert(part.part_id.as_str().to_string(), detection);
    }
    Ok(Some(DetectionPass {
        by_part,
        regions,
        detector_id,
    }))
}

fn part_matches<'a>(
    parts: &'a [PartBytes],
    detections: &'a BTreeMap<String, Detection>,
) -> Vec<PartMatch<'a>> {
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| PartMatch {
            index,
            role: &part.role,
            declared: &part.declared,
            detection: detections.get(part.part_id.as_str()),
        })
        .collect()
}

fn consumes_ok(
    registry: &Registry,
    node: &crate::route::Candidate,
    request: &RunRequest,
    detections: &BTreeMap<String, Detection>,
) -> bool {
    if node.after.is_some() {
        return true;
    }
    let Some(part) = request.parts.get(node.part_index) else {
        return false;
    };
    let selectors = selectors_of(registry, &node.adapter_id);
    let detected = detections
        .get(part.part_id.as_str())
        .map(|item| item.media_type.as_str());
    selector_match(
        &selectors,
        "raw",
        None,
        Some(part.role.as_str()),
        &part.declared,
        detected,
    )
}

fn selectors_of(registry: &Registry, id: &str) -> Vec<InputSelector> {
    if let Some(adapter) = registry.search(id) {
        return adapter.meta().consumes.clone();
    }
    if let Some(adapter) = registry.content(id) {
        return adapter.meta().consumes.clone();
    }
    Vec::new()
}

fn selector_match(
    selectors: &[InputSelector],
    kind: &str,
    variant: Option<&str>,
    role: Option<&str>,
    declared: &str,
    detected: Option<&str>,
) -> bool {
    if selectors.is_empty() {
        return true;
    }
    selectors.iter().any(|selector| {
        selector.kind.as_ref().is_none_or(|item| item == kind)
            && variant_ok(selector, variant)
            && selector
                .part_role
                .as_ref()
                .is_none_or(|item| Some(item.as_str()) == role)
            && media_ok(selector, declared, detected)
    })
}

fn variant_ok(selector: &InputSelector, variant: Option<&str>) -> bool {
    match &selector.variant {
        None => true,
        Some(expected) => variant == Some(expected.as_str()),
    }
}

fn media_ok(selector: &InputSelector, declared: &str, detected: Option<&str>) -> bool {
    let Some(expected) = &selector.media_type else {
        return true;
    };
    let expected = essence(expected);
    essence(declared) == expected || detected.is_some_and(|item| essence(item) == expected)
}

fn ensure_deterministic(registry: &Registry, id: &str) -> Result<(), AdapterError> {
    let deterministic = registry
        .search(id)
        .map(|adapter| adapter.meta().deterministic)
        .or_else(|| {
            registry
                .content(id)
                .map(|adapter| adapter.meta().deterministic)
        });
    match deterministic {
        None => Err(AdapterError::config(format!("适配器不存在: {id}"))),
        Some(false) => Err(AdapterError::config(format!("{id} 不是确定性适配器"))),
        Some(true) => Ok(()),
    }
}

fn validate_spec(registry: &Registry, id: &str, spec: &Value) -> Result<(), AdapterError> {
    if let Some(adapter) = registry.search(id) {
        return adapter.validate(spec);
    }
    if let Some(adapter) = registry.content(id) {
        return adapter.validate(spec);
    }
    Err(AdapterError::config(format!("适配器不存在: {id}")))
}

async fn probe_nodes(
    registry: &Registry,
    nodes: Vec<crate::route::Candidate>,
    request: &RunRequest,
    detections: &BTreeMap<String, Detection>,
) -> Result<Vec<crate::route::Candidate>, AdapterError> {
    let mut kept = Vec::new();
    for node in nodes {
        if !node.probe {
            kept.push(node);
            continue;
        }
        let part = &request.parts[node.part_index];
        let input = part_input(part, request, detections.get(part.part_id.as_str()));
        let accept = if let Some(adapter) = registry.search(&node.adapter_id) {
            adapter.probe(&input, &node.spec).await
        } else if let Some(adapter) = registry.content(&node.adapter_id) {
            adapter.probe(&input, &node.spec).await
        } else {
            None
        };
        if accept.is_some_and(|probe| probe.accept) {
            kept.push(node);
        }
    }
    Ok(kept)
}

async fn execute(
    registry: &Registry,
    request: RunRequest,
    pass: Option<DetectionPass>,
    steps: Vec<Step>,
) -> Result<RunOutput, AdapterError> {
    let detections = pass
        .as_ref()
        .map(|item| item.by_part.clone())
        .unwrap_or_default();
    let mut produced: BTreeMap<(usize, String), Executed> = BTreeMap::new();
    let mut discoveries = Vec::new();
    let mut facets = Vec::new();
    let mut links = Vec::new();
    let mut feedback = Vec::new();
    if let Some(pass) = pass.as_ref() {
        push_detection(&request, pass, &mut facets)?;
    }
    for step in steps {
        let input = step_input(registry, &step, &request, &detections, &produced)?;
        let Some(input) = input else {
            continue;
        };
        let hash = params_hash(&step.spec);
        if let Some(adapter) = registry.search(&step.adapter_id) {
            let output = adapter.discover(&input, &step.spec).await?;
            record_feedback(&mut feedback, &step, output.feedback);
            for mut item in output.items {
                attach_span(&mut item.materialize, &input);
                discoveries.push(EmittedDiscovery {
                    params_hash: hash.clone(),
                    item,
                });
            }
            produced.insert(
                (step.part_index, step.adapter_id.clone()),
                Executed {
                    view: None,
                    kind: None,
                    variant: None,
                },
            );
        } else if let Some(adapter) = registry.content(&step.adapter_id) {
            let output = adapter.adapt(&input, &step.spec).await?;
            record_feedback(&mut feedback, &step, output.feedback);
            let view = output.facets.iter().find_map(as_view);
            let kind = output
                .facets
                .first()
                .map(|draft| draft.kind.as_str().to_string());
            let variant = output
                .facets
                .first()
                .and_then(|draft| draft.variant.clone());
            for mut draft in output.facets {
                if draft.derived_from.is_empty() {
                    attach_one(&mut draft, &input);
                }
                facets.push(Emitted {
                    params_hash: hash.clone(),
                    draft,
                });
            }
            for link in output.links {
                links.push((hash.clone(), link));
            }
            produced.insert(
                (step.part_index, step.adapter_id),
                Executed {
                    view,
                    kind,
                    variant,
                },
            );
        }
    }
    let compiled: Compiled = crate::compile::compile(crate::compile::CompileInput {
        producer: request.producer,
        batch: Batch {
            discoveries,
            facets,
            links,
        },
        admission: &request.admission,
        now: request.now,
        initial: request.initial,
        resource: request.resource.as_ref(),
        resource_alias: request.resource_alias.as_deref(),
        origin: request.origin.as_ref(),
        identities: request.identities.as_ref(),
    })?;
    Ok(RunOutput {
        changeset: compiled.changeset,
        follows: compiled.follows,
        feedback,
    })
}

fn push_detection(
    request: &RunRequest,
    pass: &DetectionPass,
    facets: &mut Vec<Emitted>,
) -> Result<(), AdapterError> {
    let mut parts = Vec::new();
    for part in &request.parts {
        let Some(detection) = pass.by_part.get(part.part_id.as_str()) else {
            continue;
        };
        parts.push(PartDetection {
            part_id: part.part_id.clone(),
            role: part.role.clone(),
            detection: detection.clone(),
        });
    }
    let hash = strata_model::content_hash(pass.detector_id.as_bytes());
    let mut detect_facet = FacetDraft {
        kind: FacetKind::Struct,
        variant: Some("detect".into()),
        body: DraftBody::Struct(detection_body(&parts)),
        media_type: Some("application/json".into()),
        derived_from: Vec::new(),
    };
    if let Some(part) = request.parts.first() {
        attach_one(
            &mut detect_facet,
            &part_input(part, request, pass.by_part.get(part.part_id.as_str())),
        );
    }
    facets.push(Emitted {
        params_hash: hash.clone(),
        draft: detect_facet,
    });
    if !pass.regions.is_empty() {
        let hits = pass
            .regions
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "type": hit.r#type,
                    "partId": hit.part_id,
                    "start": hit.start,
                    "end": hit.end,
                    "confidence": hit.confidence,
                })
            })
            .collect::<Vec<_>>();
        facets.push(Emitted {
            params_hash: hash,
            draft: FacetDraft {
                kind: FacetKind::Regions,
                variant: Some("detect".into()),
                body: DraftBody::Struct(serde_json::json!({ "hits": hits })),
                media_type: Some("application/json".into()),
                derived_from: Vec::new(),
            },
        });
    }
    Ok(())
}

fn step_input(
    registry: &Registry,
    step: &Step,
    request: &RunRequest,
    detections: &BTreeMap<String, Detection>,
    produced: &BTreeMap<(usize, String), Executed>,
) -> Result<Option<AdapterInput>, AdapterError> {
    if let Some(after) = &step.after {
        let Some(upstream) = produced.get(&(step.part_index, after.clone())) else {
            return Ok(None);
        };
        let Some(view) = upstream.view.clone() else {
            return Ok(None);
        };
        let selectors = selectors_of(registry, &step.adapter_id);
        if !selector_match(
            &selectors,
            upstream.kind.as_deref().unwrap_or(""),
            upstream.variant.as_deref(),
            None,
            &view.media_type,
            None,
        ) {
            return Ok(None);
        }
        let part = &request.parts[step.part_index];
        return Ok(Some(AdapterInput {
            part_id: Some(part.part_id.clone()),
            detection: detections.get(part.part_id.as_str()).cloned(),
            source: request.source.clone(),
            resource: request.resource.clone(),
            locator: request.locator.clone(),
            facet: None,
            view,
        }));
    }
    let part = &request.parts[step.part_index];
    Ok(Some(part_input(
        part,
        request,
        detections.get(part.part_id.as_str()),
    )))
}

fn part_input(
    part: &PartBytes,
    request: &RunRequest,
    detection: Option<&Detection>,
) -> AdapterInput {
    AdapterInput {
        part_id: Some(part.part_id.clone()),
        view: raw_view(part),
        detection: detection.cloned(),
        source: request.source.clone(),
        resource: request.resource.clone(),
        locator: request.locator.clone(),
        facet: part.facet.clone(),
    }
}

fn raw_view(part: &PartBytes) -> ReadonlyView {
    ReadonlyView {
        bytes: part.bytes.clone(),
        media_type: part.declared.clone(),
        charset: part.charset.clone(),
        role: Some(part.role.clone()),
    }
}

fn as_view(draft: &FacetDraft) -> Option<ReadonlyView> {
    let (bytes, media_type) = match &draft.body {
        DraftBody::Text(text) => (
            Bytes::from(text.clone().into_bytes()),
            draft
                .media_type
                .clone()
                .unwrap_or_else(|| "text/plain".into()),
        ),
        DraftBody::Struct(value) => (
            Bytes::from(serde_json::to_vec(value).unwrap_or_default()),
            "application/json".into(),
        ),
    };
    Some(ReadonlyView {
        bytes,
        media_type,
        charset: Some("utf-8".into()),
        role: None,
    })
}

fn attach_span(drafts: &mut [FacetDraft], input: &AdapterInput) {
    for draft in drafts {
        attach_one(draft, input);
    }
}

fn attach_one(draft: &mut FacetDraft, input: &AdapterInput) {
    if !draft.derived_from.is_empty() {
        return;
    }
    let Some(facet) = &input.facet else {
        return;
    };
    if let Ok(span) = SpanRef::new(facet.clone(), 0, input.view.bytes.len() as u64) {
        draft.derived_from.push(span);
    }
}

fn record_feedback(out: &mut Vec<AdapterFeedback>, step: &Step, feedback: Option<Feedback>) {
    if let Some(feedback) = feedback {
        out.push(AdapterFeedback {
            adapter_id: step.adapter_id.clone(),
            part_id: None,
            feedback,
        });
    }
}

pub fn source_view(id: &str, accessor: &str) -> Result<SourceView, AdapterError> {
    Ok(SourceView {
        id: SourceId::new(id)?,
        accessor: accessor.to_string(),
        render: RenderMode::Auto,
    })
}

pub fn empty_identities() -> Arc<dyn IdentityView> {
    Arc::new(MapIdentities::default())
}
