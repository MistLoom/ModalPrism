//! 把适配器输出收成变更集。适配器本身不调用写入。

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value};
use strata_model::{
    Body, ChangeSet, FacetKind, LineageInput, Locator, Op, Producer, Rid, Target, Timestamp,
};

use crate::error::AdapterError;
use crate::model::{Discovery, DraftBody, FacetDraft, Follow, LinkDraft, LinkTarget, RenderMode};
use crate::textutil::locator_text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityHit {
    Absent,
    Live(Rid),
    Tombstone(Rid),
}

pub trait IdentityView: Send + Sync {
    fn lookup(&self, key: &str) -> IdentityHit;
}

#[derive(Debug, Default)]
pub struct MapIdentities {
    pub hits: BTreeMap<String, IdentityHit>,
}

impl IdentityView for MapIdentities {
    fn lookup(&self, key: &str) -> IdentityHit {
        self.hits.get(key).cloned().unwrap_or(IdentityHit::Absent)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Admission {
    pub allow_prefixes: Vec<String>,
    pub deny_prefixes: Vec<String>,
    pub published_after: Option<Duration>,
    pub backfill_limit: Option<u64>,
    pub backfill_max_age: Option<Duration>,
    pub noise: Vec<String>,
}

impl Admission {
    pub fn from_value(value: &Value) -> Result<Self, AdapterError> {
        let allow_prefixes = string_array(value.get("allowUrlPrefixes"))?;
        let deny_prefixes = string_array(value.get("denyUrlPrefixes"))?;
        let noise = string_array(value.get("noise"))?;
        let published_after = optional_duration(value.get("publishedAfter"))?;
        let backfill = value.get("initialBackfill");
        let (backfill_limit, backfill_max_age) = match backfill {
            None => (None, None),
            Some(object) => (
                object
                    .get("limit")
                    .map(|item| {
                        item.as_u64()
                            .ok_or_else(|| AdapterError::config("limit 必须是整数"))
                    })
                    .transpose()?,
                optional_duration(object.get("maxAge"))?,
            ),
        };
        Ok(Self {
            allow_prefixes,
            deny_prefixes,
            published_after,
            backfill_limit,
            backfill_max_age,
            noise,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Emitted {
    pub params_hash: String,
    pub draft: FacetDraft,
}

#[derive(Clone, Debug)]
pub struct EmittedDiscovery {
    pub params_hash: String,
    pub item: Discovery,
}

#[derive(Clone, Debug)]
pub struct Batch {
    pub discoveries: Vec<EmittedDiscovery>,
    pub facets: Vec<Emitted>,
    pub links: Vec<(String, LinkDraft)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FollowRequest {
    pub locator: Locator,
    pub accessor: String,
    pub render: RenderMode,
    pub priority: i32,
    pub identity: String,
}

pub struct Compiled {
    pub changeset: ChangeSet,
    pub follows: Vec<FollowRequest>,
}

pub struct CompileInput<'a> {
    pub producer: Producer,
    pub batch: Batch,
    pub admission: &'a Admission,
    pub now: Timestamp,
    pub initial: bool,
    pub resource: Option<&'a Rid>,
    pub origin: Option<&'a Rid>,
    pub identities: &'a dyn IdentityView,
}

pub fn compile(input: CompileInput<'_>) -> Result<Compiled, AdapterError> {
    let CompileInput {
        producer,
        batch,
        admission,
        now,
        initial,
        resource,
        origin,
        identities,
    } = input;
    let admitted = admit(&batch.discoveries, admission, now, initial);
    let mut ops = Vec::new();
    let mut follows = Vec::new();
    let mut kept = 0u64;
    let mut new_index = 0u64;
    let limit = if initial {
        admission.backfill_limit
    } else {
        None
    };
    for emitted in admitted {
        if limit.is_some_and(|limit| kept >= limit) {
            break;
        }
        let discovery = &emitted.item;
        match classify(&discovery.identity, identities) {
            Class::Tombstone => continue,
            Class::Existing(rid) => {
                kept += 1;
                ops.push(put_facet(
                    Target::Rid(rid.clone()),
                    &struct_draft(discovery),
                    emitted.params_hash.clone(),
                )?);
                if let Some(dst) = origin.or(resource) {
                    ops.push(Op::Link {
                        src: Target::Rid(rid),
                        rel: "discovered_by".into(),
                        dst: Target::Rid(dst.clone()),
                        attrs: None,
                    });
                }
            }
            Class::New => {
                kept += 1;
                let alias = format!("d{new_index}");
                new_index += 1;
                let identity = discovery
                    .identity
                    .first()
                    .cloned()
                    .ok_or_else(|| AdapterError::config("发现缺少身份键"))?;
                ops.push(Op::Ensure {
                    identity,
                    ns: None,
                    labels: BTreeMap::new(),
                    r#as: alias.clone(),
                });
                ops.push(put_facet(
                    Target::Alias(alias.clone()),
                    &struct_draft(discovery),
                    emitted.params_hash.clone(),
                )?);
                for draft in &discovery.materialize {
                    ops.push(put_facet(
                        Target::Alias(alias.clone()),
                        draft,
                        emitted.params_hash.clone(),
                    )?);
                }
                if let Some(dst) = origin.or(resource) {
                    ops.push(Op::Link {
                        src: Target::Alias(alias),
                        rel: "discovered_by".into(),
                        dst: Target::Rid(dst.clone()),
                        attrs: None,
                    });
                }
                if let Follow::Yes {
                    accessor,
                    render,
                    priority,
                } = &discovery.follow
                {
                    follows.push(FollowRequest {
                        locator: discovery.locator.clone(),
                        accessor: accessor.clone(),
                        render: *render,
                        priority: *priority,
                        identity: discovery.identity[0].clone(),
                    });
                }
            }
        }
    }
    if (!batch.facets.is_empty() || !batch.links.is_empty()) && resource.is_none() {
        return Err(AdapterError::config("内容适配器缺少目标资源"));
    }
    if let Some(resource) = resource {
        for emitted in &batch.facets {
            ops.push(put_facet(
                Target::Rid(resource.clone()),
                &emitted.draft,
                emitted.params_hash.clone(),
            )?);
        }
        let mut url_links: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (hash, link) in &batch.links {
            match &link.target {
                LinkTarget::Url(url) => {
                    url_links
                        .entry(hash.clone())
                        .or_default()
                        .push(serde_json::json!({
                            "rel": link.rel,
                            "target": url,
                            "attrs": link.attrs,
                        }));
                }
                LinkTarget::Rid(dst) => {
                    ops.push(Op::Link {
                        src: Target::Rid(resource.clone()),
                        rel: link.rel.clone(),
                        dst: Target::Rid(dst.clone()),
                        attrs: link.attrs.clone(),
                    });
                }
            }
        }
        for (hash, links) in url_links {
            ops.push(put_facet(
                Target::Rid(resource.clone()),
                &FacetDraft {
                    kind: FacetKind::Links,
                    variant: None,
                    body: DraftBody::Struct(Value::Object(Map::from_iter([(
                        "links".into(),
                        Value::Array(links),
                    )]))),
                    media_type: Some("application/json".into()),
                    derived_from: Vec::new(),
                },
                hash,
            )?);
        }
    }
    let changeset = ChangeSet {
        producer,
        idempotency_key: None,
        ack: None,
        ops,
    };
    changeset.validate()?;
    Ok(Compiled { changeset, follows })
}

enum Class {
    New,
    Existing(Rid),
    Tombstone,
}

fn classify(keys: &[String], identities: &dyn IdentityView) -> Class {
    let mut live = None;
    for key in keys {
        match identities.lookup(key) {
            IdentityHit::Tombstone(_) => return Class::Tombstone,
            IdentityHit::Live(rid) if live.is_none() => live = Some(rid),
            IdentityHit::Live(_) | IdentityHit::Absent => {}
        }
    }
    match live {
        Some(rid) => Class::Existing(rid),
        None => Class::New,
    }
}

fn admit(
    items: &[EmittedDiscovery],
    admission: &Admission,
    now: Timestamp,
    initial: bool,
) -> Vec<EmittedDiscovery> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for emitted in items {
        let item = &emitted.item;
        let text = locator_text(&item.locator);
        if !admission.allow_prefixes.is_empty()
            && !admission
                .allow_prefixes
                .iter()
                .any(|prefix| text.starts_with(prefix))
        {
            continue;
        }
        if admission
            .deny_prefixes
            .iter()
            .any(|prefix| text.starts_with(prefix))
        {
            continue;
        }
        if admission.noise.iter().any(|word| noisy(item, word)) {
            continue;
        }
        if let Some(window) = admission.published_after {
            if let Some(published) = item.hints.published_at {
                if older_than(published, now, window) {
                    continue;
                }
            }
        }
        if initial {
            if let Some(window) = admission.backfill_max_age {
                if let Some(published) = item.hints.published_at {
                    if older_than(published, now, window) {
                        continue;
                    }
                }
            }
        }
        let Some(key) = item.identity.first() else {
            continue;
        };
        if seen.contains(key) {
            continue;
        }
        seen.push(key.clone());
        out.push(emitted.clone());
    }
    out
}

fn noisy(item: &Discovery, word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    let title = item
        .hints
        .title
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let summary = item
        .hints
        .summary
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    title.contains(&word) || summary.contains(&word)
}

fn older_than(published: Timestamp, now: Timestamp, window: Duration) -> bool {
    match published.checked_add(window) {
        Ok(limit) => limit < now,
        Err(_) => false,
    }
}

fn struct_draft(discovery: &Discovery) -> FacetDraft {
    let mut hints = Map::new();
    if let Some(title) = &discovery.hints.title {
        hints.insert("title".into(), Value::String(title.clone()));
    }
    if let Some(published) = discovery.hints.published_at {
        if let Ok(value) = serde_json::to_value(published) {
            hints.insert("publishedAt".into(), value);
        }
    }
    if let Some(author) = &discovery.hints.author {
        hints.insert("author".into(), Value::String(author.clone()));
    }
    if let Some(summary) = &discovery.hints.summary {
        hints.insert("summary".into(), Value::String(summary.clone()));
    }
    if let Some(rank) = discovery.hints.rank {
        hints.insert("rank".into(), Value::from(rank));
    }
    if !discovery.hints.extra.is_empty() {
        hints.insert("extra".into(), Value::Object(discovery.hints.extra.clone()));
    }
    FacetDraft {
        kind: FacetKind::Struct,
        variant: Some("discovery".into()),
        body: DraftBody::Struct(serde_json::json!({
            "locator": discovery.locator,
            "identity": discovery.identity,
            "hints": hints,
        })),
        media_type: Some("application/json".into()),
        derived_from: discovery.span.clone().into_iter().collect(),
    }
}

fn put_facet(target: Target, draft: &FacetDraft, params_hash: String) -> Result<Op, AdapterError> {
    let body = match &draft.body {
        DraftBody::Text(text) => Body::Text { text: text.clone() },
        DraftBody::Struct(value) => Body::Struct {
            value: value.clone(),
        },
    };
    Ok(Op::PutFacet {
        target,
        kind: draft.kind.clone(),
        variant: draft.variant.clone(),
        body,
        media_type: draft.media_type.clone(),
        lineage: LineageInput {
            inputs: draft.derived_from.clone(),
            params_hash,
        },
    })
}

fn string_array(value: Option<&Value>) -> Result<Vec<String>, AdapterError> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| AdapterError::config("列表项必须是字符串"))
            })
            .collect(),
        Some(_) => Err(AdapterError::config("字段必须是字符串列表")),
    }
}

fn optional_duration(value: Option<&Value>) -> Result<Option<Duration>, AdapterError> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => crate::textutil::parse_duration(text)
            .map(Some)
            .ok_or_else(|| AdapterError::config(format!("时长无效: {text}"))),
        Some(_) => Err(AdapterError::config("时长必须是字符串")),
    }
}
