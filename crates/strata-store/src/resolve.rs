//! 把变更集解析成可重放的提交。别名在这里替换成 RID。

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strata_model::{Body, ChangeSet, FacetKind, IdentityKey, Namespace, Op, Target, content_hash};

use crate::error::Error;

#[derive(Clone, Debug)]
pub(crate) struct IdentityHead {
    pub rid: String,
    pub tombstone: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ResourceHead {
    pub tombstone: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HeadSet {
    pub identities: HashMap<String, IdentityHead>,
    pub resources: HashMap<String, ResourceHead>,
    pub states: HashMap<(String, String), u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StoredCommit {
    pub commit_id: String,
    pub producer_module: String,
    pub producer_version: String,
    pub config_generation: u64,
    pub ops: Vec<StoredOp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum StoredOp {
    Ensure {
        identity: String,
        ns: String,
        labels: BTreeMap<String, String>,
        rid: String,
        source_id: Option<String>,
        created_at: String,
        created: bool,
    },
    PutFacet {
        rid: String,
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        body: StoredBody,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        inputs: Vec<String>,
        params_hash: String,
        content_hash: String,
        size: u64,
    },
    Link {
        src: String,
        rel: String,
        dst: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attrs: Option<Value>,
    },
    Label {
        rid: String,
        set: BTreeMap<String, String>,
        unset: Vec<String>,
    },
    SetState {
        scope: String,
        key: String,
        value: Value,
        version: u64,
    },
    Tombstone {
        rid: String,
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum StoredBody {
    Text { text: String },
    Struct { value: Value },
    Staged { sha256: String, size: u64 },
}

#[derive(Clone, Debug)]
pub(crate) struct IdentityInsert {
    pub key: String,
    pub rid: String,
    pub scheme: String,
    pub source_id: Option<String>,
    pub tombstone: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ResourceInsert {
    pub rid: String,
    pub ns: String,
    pub source_id: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct StateInsert {
    pub scope: String,
    pub key: String,
    pub version: u64,
    pub value: String,
}

#[derive(Debug)]
pub(crate) struct Resolved {
    pub commit: StoredCommit,
    pub identities: Vec<IdentityInsert>,
    pub resources: Vec<ResourceInsert>,
    pub tombstones: Vec<String>,
    pub states: Vec<StateInsert>,
    pub staging: Vec<String>,
    pub ack: Option<String>,
}

pub(crate) fn resolve(
    cs: &ChangeSet,
    heads: &mut HeadSet,
    commit_id: &str,
    now: &str,
) -> Result<Resolved, Error> {
    let mut aliases = HashMap::<String, String>::new();
    let mut ops = Vec::with_capacity(cs.ops.len());
    let mut identities = Vec::new();
    let mut resources = Vec::new();
    let mut tombstones = Vec::new();
    let mut states = Vec::new();
    let mut staging = Vec::new();

    for op in &cs.ops {
        match op {
            Op::Ensure {
                identity,
                ns,
                labels,
                r#as,
            } => {
                check_labels(labels)?;
                if aliases.contains_key(r#as) {
                    return Err(Error::DuplicateAlias(r#as.clone()));
                }
                let ns = match ns {
                    Some(ns) => Namespace::parse(ns)?.to_string(),
                    None => Namespace::Data.to_string(),
                };
                let key = IdentityKey::parse(identity)?;
                let source_id = match &key {
                    IdentityKey::Source { source_id, .. } => Some(source_id.clone()),
                    _ => cs.producer.source_id.clone(),
                };
                let scheme = scheme_of(identity);
                if let Some(existing) = heads.identities.get(identity) {
                    aliases.insert(r#as.clone(), existing.rid.clone());
                    ops.push(StoredOp::Ensure {
                        identity: identity.clone(),
                        ns,
                        labels: labels.clone(),
                        rid: existing.rid.clone(),
                        source_id,
                        created_at: now.to_string(),
                        created: false,
                    });
                } else {
                    let rid = strata_model::Rid::generate().to_string();
                    aliases.insert(r#as.clone(), rid.clone());
                    heads.identities.insert(
                        identity.clone(),
                        IdentityHead {
                            rid: rid.clone(),
                            tombstone: false,
                        },
                    );
                    heads
                        .resources
                        .insert(rid.clone(), ResourceHead { tombstone: false });
                    identities.push(IdentityInsert {
                        key: identity.clone(),
                        rid: rid.clone(),
                        scheme: scheme.to_string(),
                        source_id: source_id.clone(),
                        tombstone: false,
                    });
                    resources.push(ResourceInsert {
                        rid: rid.clone(),
                        ns: ns.clone(),
                        source_id: source_id.clone(),
                        created_at: now.to_string(),
                    });
                    ops.push(StoredOp::Ensure {
                        identity: identity.clone(),
                        ns,
                        labels: labels.clone(),
                        rid,
                        source_id,
                        created_at: now.to_string(),
                        created: true,
                    });
                }
            }
            Op::PutFacet {
                target,
                kind,
                variant,
                body,
                media_type,
                lineage,
            } => {
                if matches!(kind, FacetKind::Embedding) {
                    return Err(Error::EmbeddingUnspecified);
                }
                let rid = resolve_target(target, &aliases, heads)?;
                let stored = store_body(kind, body)?;
                if let StoredBody::Staged { sha256, .. } = &stored {
                    staging.push(sha256.clone());
                }
                let (content_hash, size) = body_hash(&stored)?;
                ops.push(StoredOp::PutFacet {
                    rid,
                    kind: kind.as_str().to_string(),
                    variant: variant.clone(),
                    body: stored,
                    media_type: media_type.clone(),
                    inputs: lineage.inputs.iter().map(|span| span.to_string()).collect(),
                    params_hash: lineage.params_hash.clone(),
                    content_hash,
                    size,
                });
            }
            Op::Link {
                src,
                rel,
                dst,
                attrs,
            } => {
                let src = resolve_target(src, &aliases, heads)?;
                let dst = resolve_target(dst, &aliases, heads)?;
                ops.push(StoredOp::Link {
                    src,
                    rel: rel.clone(),
                    dst,
                    attrs: attrs.clone(),
                });
            }
            Op::Label { target, set, unset } => {
                check_labels(set)?;
                if unset.iter().any(|key| bad_label(key)) {
                    return Err(Error::InvalidLabel);
                }
                let rid = resolve_target(target, &aliases, heads)?;
                ops.push(StoredOp::Label {
                    rid,
                    set: set.clone(),
                    unset: unset.clone(),
                });
            }
            Op::SetState {
                scope,
                key,
                value,
                expect_version,
            } => {
                let actual = heads
                    .states
                    .get(&(scope.clone(), key.clone()))
                    .copied()
                    .unwrap_or(0);
                if let Some(expected) = expect_version {
                    if *expected != actual {
                        return Err(Error::StateVersion {
                            scope: scope.clone(),
                            key: key.clone(),
                            expected: *expected,
                            actual,
                        });
                    }
                }
                let version = actual + 1;
                heads.states.insert((scope.clone(), key.clone()), version);
                states.push(StateInsert {
                    scope: scope.clone(),
                    key: key.clone(),
                    version,
                    value: serde_json::to_string(value)?,
                });
                ops.push(StoredOp::SetState {
                    scope: scope.clone(),
                    key: key.clone(),
                    value: value.clone(),
                    version,
                });
            }
            Op::Tombstone { target, reason } => {
                let rid = resolve_target(target, &aliases, heads)?;
                if let Some(resource) = heads.resources.get_mut(&rid) {
                    resource.tombstone = true;
                }
                for identity in heads.identities.values_mut() {
                    if identity.rid == rid {
                        identity.tombstone = true;
                    }
                }
                tombstones.push(rid.clone());
                ops.push(StoredOp::Tombstone {
                    rid,
                    reason: reason.clone(),
                });
            }
        }
    }

    Ok(Resolved {
        commit: StoredCommit {
            commit_id: commit_id.to_string(),
            producer_module: cs.producer.module.clone(),
            producer_version: cs.producer.version.clone(),
            config_generation: cs.producer.config_generation,
            ops,
        },
        identities,
        resources,
        tombstones,
        states,
        staging,
        ack: cs.ack.as_ref().map(|id| id.as_str().to_string()),
    })
}

fn resolve_target(
    target: &Target,
    aliases: &HashMap<String, String>,
    heads: &HeadSet,
) -> Result<String, Error> {
    match target {
        Target::Alias(alias) => aliases
            .get(alias)
            .cloned()
            .ok_or_else(|| Error::UnknownAlias(alias.clone())),
        Target::Rid(rid) => {
            let text = rid.as_str();
            if heads.resources.contains_key(text) {
                Ok(text.to_string())
            } else {
                Err(Error::UnknownResource(text.to_string()))
            }
        }
    }
}

fn store_body(kind: &FacetKind, body: &Body) -> Result<StoredBody, Error> {
    match body {
        Body::External { .. } => Err(Error::ExternalUnspecified),
        Body::Text { text } if text_kind(kind) => Ok(StoredBody::Text { text: text.clone() }),
        Body::Struct { value } if struct_kind(kind) => Ok(StoredBody::Struct {
            value: value.clone(),
        }),
        Body::Staged { part } if binary_kind(kind) => Ok(StoredBody::Staged {
            sha256: part.sha256().to_string(),
            size: part.size(),
        }),
        _ => Err(Error::BodyMismatch),
    }
}

fn body_hash(body: &StoredBody) -> Result<(String, u64), Error> {
    match body {
        StoredBody::Text { text } => Ok((content_hash(text.as_bytes()), text.len() as u64)),
        StoredBody::Struct { value } => {
            let bytes = serde_json::to_vec(value)?;
            let size = bytes.len() as u64;
            Ok((content_hash(&bytes), size))
        }
        StoredBody::Staged { sha256, size } => Ok((sha256.clone(), *size)),
    }
}

fn text_kind(kind: &FacetKind) -> bool {
    matches!(
        kind,
        FacetKind::Raw
            | FacetKind::Html
            | FacetKind::Xml
            | FacetKind::Md
            | FacetKind::Text
            | FacetKind::Json
            | FacetKind::Yaml
            | FacetKind::Other(_)
    )
}

fn struct_kind(kind: &FacetKind) -> bool {
    matches!(
        kind,
        FacetKind::Struct
            | FacetKind::Links
            | FacetKind::Regions
            | FacetKind::Json
            | FacetKind::Yaml
            | FacetKind::Other(_)
    )
}

fn binary_kind(kind: &FacetKind) -> bool {
    matches!(
        kind,
        FacetKind::Raw
            | FacetKind::Html
            | FacetKind::Xml
            | FacetKind::Md
            | FacetKind::Text
            | FacetKind::Json
            | FacetKind::Yaml
            | FacetKind::Image
            | FacetKind::Audio
            | FacetKind::Video
            | FacetKind::File
            | FacetKind::Other(_)
    )
}

fn scheme_of(identity: &str) -> &str {
    identity.split(':').next().unwrap_or("")
}

fn bad_label(text: &str) -> bool {
    text.contains('=') || text.contains('\n')
}

fn check_labels(labels: &BTreeMap<String, String>) -> Result<(), Error> {
    if labels.keys().any(|key| bad_label(key)) || labels.values().any(|value| bad_label(value)) {
        Err(Error::InvalidLabel)
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
#[cfg(test)]
mod tests {
    use super::*;
    use strata_model::{FacetKind, LineageInput, Producer, Target};

    fn producer() -> Producer {
        Producer {
            module: "test".to_string(),
            version: "0".to_string(),
            config_generation: 1,
            source_id: None,
        }
    }

    #[test]
    fn producer_source_id_on_url_identity() {
        let cs = ChangeSet {
            producer: Producer {
                module: "runner".to_string(),
                version: "1".to_string(),
                config_generation: 1,
                source_id: Some("source/demo".to_string()),
            },
            idempotency_key: None,
            ack: None,
            ops: vec![ensure("url:https://example.test/post", "a")],
        };
        let mut heads = HeadSet::default();
        let resolved = resolve(&cs, &mut heads, "c1", "2026-10-19T00:00:00Z").unwrap();
        let StoredOp::Ensure { source_id, .. } = &resolved.commit.ops[0] else {
            panic!("ensure");
        };
        assert_eq!(source_id.as_deref(), Some("source/demo"));
        assert_eq!(
            resolved.resources[0].source_id.as_deref(),
            Some("source/demo")
        );
    }

    fn ensure(identity: &str, alias: &str) -> Op {
        Op::Ensure {
            identity: identity.to_string(),
            ns: None,
            labels: BTreeMap::new(),
            r#as: alias.to_string(),
        }
    }

    #[test]
    fn second_ensure_reuses_rid() {
        let cs = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![ensure("ext:doi:10.1", "a"), ensure("ext:doi:10.1", "b")],
        };
        cs.validate().unwrap();
        let mut heads = HeadSet::default();
        let resolved = resolve(&cs, &mut heads, "c1", "2026-10-19T00:00:00Z").unwrap();
        let StoredOp::Ensure {
            rid: first,
            created,
            ..
        } = &resolved.commit.ops[0]
        else {
            panic!("ensure");
        };
        assert!(created);
        let StoredOp::Ensure {
            rid: second,
            created: created_again,
            ..
        } = &resolved.commit.ops[1]
        else {
            panic!("ensure");
        };
        assert!(!created_again);
        assert_eq!(first, second);
        assert_eq!(resolved.resources.len(), 1);
    }

    #[test]
    fn tombstone_identity_returns_same_rid() {
        let first = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![
                ensure("url:https://example.test/a", "a"),
                Op::Tombstone {
                    target: Target::Alias("a".to_string()),
                    reason: "gone".to_string(),
                },
            ],
        };
        let mut heads = HeadSet::default();
        let resolved = resolve(&first, &mut heads, "c1", "2026-10-19T00:00:00Z").unwrap();
        let StoredOp::Ensure { rid, .. } = &resolved.commit.ops[0] else {
            panic!("ensure");
        };
        let rid = rid.clone();
        let second = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![ensure("url:https://example.test/a", "a")],
        };
        let resolved = resolve(&second, &mut heads, "c2", "2026-10-19T00:00:00Z").unwrap();
        let StoredOp::Ensure {
            rid: again,
            created,
            ..
        } = &resolved.commit.ops[0]
        else {
            panic!("ensure");
        };
        assert_eq!(again, &rid);
        assert!(!created);
        assert!(heads.identities.values().next().unwrap().tombstone);
    }

    #[test]
    fn state_version_conflict_leaves_heads() {
        let mut heads = HeadSet::default();
        let ok = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![Op::SetState {
                scope: "cursor".to_string(),
                key: "src".to_string(),
                value: Value::String("1".to_string()),
                expect_version: Some(0),
            }],
        };
        resolve(&ok, &mut heads, "c1", "2026-10-19T00:00:00Z").unwrap();
        let bad = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![Op::SetState {
                scope: "cursor".to_string(),
                key: "src".to_string(),
                value: Value::String("2".to_string()),
                expect_version: Some(0),
            }],
        };
        let error = resolve(&bad, &mut heads, "c2", "2026-10-19T00:00:00Z").unwrap_err();
        assert!(matches!(error, Error::StateVersion { actual: 1, .. }));
        assert_eq!(heads.states[&("cursor".to_string(), "src".to_string())], 1);
    }

    #[test]
    fn rejects_embedding_external_and_mismatched_body() {
        let mut heads = HeadSet::default();
        let cs = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![ensure("ext:doi:1", "a")],
        };
        resolve(&cs, &mut heads, "c1", "2026-10-19T00:00:00Z").unwrap();
        let embed = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![
                ensure("ext:doi:1", "a"),
                Op::PutFacet {
                    target: Target::Alias("a".to_string()),
                    kind: FacetKind::Embedding,
                    variant: Some("m".to_string()),
                    body: Body::Text {
                        text: "x".to_string(),
                    },
                    media_type: None,
                    lineage: LineageInput {
                        inputs: vec![],
                        params_hash: "p".to_string(),
                    },
                },
            ],
        };
        assert!(matches!(
            resolve(&embed, &mut heads, "c2", "2026-10-19T00:00:00Z"),
            Err(Error::EmbeddingUnspecified)
        ));
        let external = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![
                ensure("ext:doi:1", "a"),
                Op::PutFacet {
                    target: Target::Alias("a".to_string()),
                    kind: FacetKind::Image,
                    variant: None,
                    body: Body::External {
                        uri: "s3://bucket/a".to_string(),
                    },
                    media_type: None,
                    lineage: LineageInput {
                        inputs: vec![],
                        params_hash: "p".to_string(),
                    },
                },
            ],
        };
        assert!(matches!(
            resolve(&external, &mut heads, "c3", "2026-10-19T00:00:00Z"),
            Err(Error::ExternalUnspecified)
        ));
        let mismatch = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![
                ensure("ext:doi:1", "a"),
                Op::PutFacet {
                    target: Target::Alias("a".to_string()),
                    kind: FacetKind::Image,
                    variant: None,
                    body: Body::Text {
                        text: "x".to_string(),
                    },
                    media_type: None,
                    lineage: LineageInput {
                        inputs: vec![],
                        params_hash: "p".to_string(),
                    },
                },
            ],
        };
        assert!(matches!(
            resolve(&mismatch, &mut heads, "c4", "2026-10-19T00:00:00Z"),
            Err(Error::BodyMismatch)
        ));
    }

    #[test]
    fn struct_hash_uses_json_bytes() {
        let mut heads = HeadSet::default();
        let value = serde_json::json!({"b": 1, "a": 2});
        let cs = ChangeSet {
            producer: producer(),
            idempotency_key: None,
            ack: None,
            ops: vec![
                ensure("ext:doi:1", "a"),
                Op::PutFacet {
                    target: Target::Alias("a".to_string()),
                    kind: FacetKind::Struct,
                    variant: Some("article".to_string()),
                    body: Body::Struct {
                        value: value.clone(),
                    },
                    media_type: None,
                    lineage: LineageInput {
                        inputs: vec![],
                        params_hash: "p".to_string(),
                    },
                },
            ],
        };
        let resolved = resolve(&cs, &mut heads, "c1", "2026-10-19T00:00:00Z").unwrap();
        let StoredOp::PutFacet {
            content_hash: hash,
            body,
            ..
        } = &resolved.commit.ops[1]
        else {
            panic!("facet");
        };
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(hash, &content_hash(&bytes));
        let StoredBody::Struct { value: stored } = body else {
            panic!("struct");
        };
        assert_eq!(stored, &value);
    }
}
