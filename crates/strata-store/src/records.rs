//! 快照读出的领域行。不含 Lance 类型。

use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub seq: u64,
    pub versions: BTreeMap<String, u64>,
    pub commit_ids: Vec<String>,
    pub kind: String,
    pub config_generation: u64,
    pub published_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRecord {
    pub rid: String,
    pub ns: String,
    pub source_id: Option<String>,
    pub created_at: String,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkRecord {
    pub rid: String,
    pub labels: Vec<String>,
    pub tombstone: bool,
    pub reason: Option<String>,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetRecord {
    pub rid: String,
    pub kind: String,
    pub variant: Option<String>,
    pub version: u64,
    pub content_hash: String,
    pub media_type: Option<String>,
    pub size: u64,
    pub body_text: Option<String>,
    pub body_struct: Option<String>,
    pub blob_ref: Option<String>,
    pub inputs: Vec<String>,
    pub params_hash: String,
    pub producer_module: String,
    pub producer_version: String,
    pub config_generation: u64,
    pub commit_id: String,
    pub created_at: String,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationRecord {
    pub src_rid: String,
    pub rel: String,
    pub dst_rid: String,
    pub attrs: Option<Value>,
    pub commit_id: String,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityRecord {
    pub identity_key: String,
    pub rid: String,
    pub scheme: String,
    pub commit_id: String,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateRecord {
    pub scope: String,
    pub key: String,
    pub value: Value,
    pub commit_id: String,
    pub seq: u64,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub snapshot: Option<SnapshotInfo>,
    pub resources: Vec<ResourceRecord>,
    pub marks: Vec<MarkRecord>,
    pub facets: Vec<FacetRecord>,
    pub relations: Vec<RelationRecord>,
    pub identities: Vec<IdentityRecord>,
    pub states: Vec<StateRecord>,
}
