//! facet 种类、版本引用与内容寻址。

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::ModelError;
use crate::id::{Rid, is_name};

/// 内置 facet 种类。未知种类进入 `Other`。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FacetKind {
    Raw,
    Html,
    Xml,
    Md,
    Text,
    Json,
    Yaml,
    Struct,
    Links,
    Image,
    Audio,
    Video,
    File,
    Embedding,
    Regions,
    Other(String),
}

impl FacetKind {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let kind = match raw {
            "raw" => Self::Raw,
            "html" => Self::Html,
            "xml" => Self::Xml,
            "md" => Self::Md,
            "text" => Self::Text,
            "json" => Self::Json,
            "yaml" => Self::Yaml,
            "struct" => Self::Struct,
            "links" => Self::Links,
            "image" => Self::Image,
            "audio" => Self::Audio,
            "video" => Self::Video,
            "file" => Self::File,
            "embedding" => Self::Embedding,
            "regions" => Self::Regions,
            other => {
                if !is_name(other) {
                    return Err(ModelError::InvalidVariant);
                }
                Self::Other(other.to_string())
            }
        };
        Ok(kind)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Raw => "raw",
            Self::Html => "html",
            Self::Xml => "xml",
            Self::Md => "md",
            Self::Text => "text",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Struct => "struct",
            Self::Links => "links",
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::File => "file",
            Self::Embedding => "embedding",
            Self::Regions => "regions",
            Self::Other(name) => name,
        }
    }

    /// `struct`、`embedding`、`regions` 必须带 variant。
    pub fn requires_variant(&self) -> bool {
        matches!(self, Self::Struct | Self::Embedding | Self::Regions)
    }
}

impl Serialize for FacetKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FacetKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        FacetKind::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// 版本选择。文本形式里精确版本写作 `#v<n>`，最新版本写作 `#latest`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionSel {
    Latest,
    Exact(u64),
}

/// `r_…@<kind>[:<variant>]#v<n>` 或 `#latest`。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FacetRef {
    pub rid: Rid,
    pub kind: FacetKind,
    pub variant: Option<String>,
    pub version: VersionSel,
}

impl FacetRef {
    pub fn new(
        rid: Rid,
        kind: FacetKind,
        variant: Option<String>,
        version: VersionSel,
    ) -> Result<Self, ModelError> {
        if let Some(variant) = &variant {
            check_variant(variant)?;
        } else if kind.requires_variant() {
            return Err(ModelError::MissingVariant);
        }
        Ok(Self {
            rid,
            kind,
            variant,
            version,
        })
    }

    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        if raw.contains('[') {
            return Err(ModelError::InvalidRef);
        }
        parse_facet_body(raw)
    }
}

impl fmt::Display for FacetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.rid, self.kind.as_str())?;
        if let Some(variant) = &self.variant {
            write!(f, ":{variant}")?;
        }
        match self.version {
            VersionSel::Latest => write!(f, "#latest"),
            VersionSel::Exact(n) => write!(f, "#v{n}"),
        }
    }
}

impl From<FacetRef> for String {
    fn from(value: FacetRef) -> String {
        value.to_string()
    }
}

impl TryFrom<String> for FacetRef {
    type Error = ModelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// 半开字节区间 `[start, end)`。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SpanRef {
    pub facet: FacetRef,
    pub start: u64,
    pub end: u64,
}

impl SpanRef {
    pub fn new(facet: FacetRef, start: u64, end: u64) -> Result<Self, ModelError> {
        if end < start {
            return Err(ModelError::InvalidSpan);
        }
        Ok(Self { facet, start, end })
    }

    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let (head, tail) = raw.split_once('[').ok_or(ModelError::InvalidRef)?;
        let inner = tail.strip_suffix(']').ok_or(ModelError::InvalidRef)?;
        if inner.contains('[') || inner.contains(']') {
            return Err(ModelError::InvalidRef);
        }
        let (start_raw, end_raw) = inner.split_once("..").ok_or(ModelError::InvalidSpan)?;
        let start: u64 = start_raw.parse().map_err(|_| ModelError::InvalidSpan)?;
        let end: u64 = end_raw.parse().map_err(|_| ModelError::InvalidSpan)?;
        Self::new(parse_facet_body(head)?, start, end)
    }
}

impl fmt::Display for SpanRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}..{}]", self.facet, self.start, self.end)
    }
}

impl From<SpanRef> for String {
    fn from(value: SpanRef) -> String {
        value.to_string()
    }
}

impl TryFrom<String> for SpanRef {
    type Error = ModelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

fn parse_facet_body(raw: &str) -> Result<FacetRef, ModelError> {
    let (rid_raw, rest) = raw.split_once('@').ok_or(ModelError::InvalidRef)?;
    let rid = Rid::parse(rid_raw)?;
    let (body, ver) = rest.split_once('#').ok_or(ModelError::InvalidRef)?;
    let version = if ver == "latest" {
        VersionSel::Latest
    } else if let Some(n) = ver.strip_prefix('v') {
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ModelError::InvalidRef);
        }
        VersionSel::Exact(n.parse().map_err(|_| ModelError::InvalidRef)?)
    } else {
        return Err(ModelError::InvalidRef);
    };
    let (kind_raw, variant) = match body.split_once(':') {
        Some((kind, variant)) => {
            check_variant(variant)?;
            (kind, Some(variant.to_string()))
        }
        None => (body, None),
    };
    FacetRef::new(rid, FacetKind::parse(kind_raw)?, variant, version)
}

pub(crate) fn check_variant(raw: &str) -> Result<(), ModelError> {
    if is_name(raw) {
        Ok(())
    } else {
        Err(ModelError::InvalidVariant)
    }
}

/// 暂存区或 blob 中的内容寻址引用。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "PartRefSerde", into = "PartRefSerde")]
pub struct PartRef {
    sha256: String,
    size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartRefSerde {
    sha256: String,
    size: u64,
}

impl PartRef {
    pub fn new(sha256: impl Into<String>, size: u64) -> Result<Self, ModelError> {
        let sha256 = canonicalize_sha256(&sha256.into())?;
        Ok(Self { sha256, size })
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: content_hash(bytes),
            size: bytes.len() as u64,
        }
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

impl From<PartRef> for PartRefSerde {
    fn from(value: PartRef) -> Self {
        Self {
            sha256: value.sha256,
            size: value.size,
        }
    }
}

impl TryFrom<PartRefSerde> for PartRef {
    type Error = ModelError;

    fn try_from(value: PartRefSerde) -> Result<Self, Self::Error> {
        PartRef::new(value.sha256, value.size)
    }
}

/// 小写十六进制 sha256。
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn canonicalize_sha256(raw: &str) -> Result<String, ModelError> {
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ModelError::InvalidDigest);
    }
    Ok(raw.to_ascii_lowercase())
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// 某个资源下一种 facet 的摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FacetSummary {
    pub kind: FacetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub latest: u64,
    pub versions: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub updated_at: crate::Timestamp,
}
