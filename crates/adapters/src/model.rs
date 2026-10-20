//! 适配器的输入、发现和内容草稿。

use bytes::Bytes;
use serde_json::{Map, Value};
use strata_model::{
    FacetKind, FacetRef, Locator, PartId, PartRole, RenderLevel, Rid, SourceId, SpanRef, Timestamp,
};

/// 只读字节视图。
#[derive(Clone, Debug)]
pub struct ReadonlyView {
    pub bytes: Bytes,
    pub media_type: String,
    pub charset: Option<String>,
    pub role: Option<PartRole>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub media_type: String,
    pub charset: Option<String>,
    pub confidence: f64,
    pub traits: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionHit {
    pub r#type: String,
    pub part_id: String,
    pub start: u64,
    pub end: u64,
    pub confidence: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    Auto,
    Never,
    Always,
}

impl RenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Never => "never",
            Self::Always => "always",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceView {
    pub id: SourceId,
    pub accessor: String,
    pub render: RenderMode,
}

#[derive(Clone, Debug)]
pub struct AdapterInput {
    pub part_id: Option<PartId>,
    pub view: ReadonlyView,
    pub detection: Option<Detection>,
    pub source: SourceView,
    pub resource: Option<Rid>,
    pub locator: Locator,
    pub facet: Option<FacetRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryHints {
    pub title: Option<String>,
    pub published_at: Option<Timestamp>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub rank: Option<u32>,
    pub extra: Map<String, Value>,
}

impl Default for DiscoveryHints {
    fn default() -> Self {
        Self {
            title: None,
            published_at: None,
            author: None,
            summary: None,
            rank: None,
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Follow {
    No,
    Yes {
        accessor: String,
        render: RenderMode,
        priority: i32,
        /// 详情页要补的字段。空列表表示这条跟进不入队。
        needs: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DraftBody {
    Text(String),
    Struct(Value),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FacetDraft {
    pub kind: FacetKind,
    pub variant: Option<String>,
    pub body: DraftBody,
    pub media_type: Option<String>,
    pub derived_from: Vec<SpanRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Discovery {
    pub locator: Locator,
    pub identity: Vec<String>,
    pub hints: DiscoveryHints,
    pub span: Option<SpanRef>,
    pub follow: Follow,
    pub materialize: Vec<FacetDraft>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Ok,
    Thin,
    Blocked,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feedback {
    pub needs_render: Option<RenderLevel>,
    pub quality: Quality,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoverOutput {
    pub items: Vec<Discovery>,
    pub feedback: Option<Feedback>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Url(String),
    Rid(Rid),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkDraft {
    pub rel: String,
    pub target: LinkTarget,
    pub attrs: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentOutput {
    pub facets: Vec<FacetDraft>,
    pub links: Vec<LinkDraft>,
    pub feedback: Option<Feedback>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    pub accept: bool,
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterRole {
    Search,
    Content,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSelector {
    pub kind: Option<String>,
    pub variant: Option<String>,
    pub part_role: Option<String>,
    pub media_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputDecl {
    Discovery,
    Facet {
        kind: String,
        variant: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterMeta {
    pub id: String,
    pub version: String,
    pub role: AdapterRole,
    pub consumes: Vec<InputSelector>,
    pub produces: Vec<OutputDecl>,
    pub deterministic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    pub media_type: String,
    pub charset: Option<String>,
    pub role: PartRole,
}
