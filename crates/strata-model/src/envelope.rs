//! 访问器输出的原始信封。部件字节只出现为暂存引用。

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::ModelError;
use crate::facet::PartRef;
use crate::id::{EnvelopeId, PartId, ReceiptId, Timestamp};

/// 部件角色。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PartRole {
    Document,
    Subresource,
    NetworkResponse,
    NetworkTrace,
    Screenshot,
    Print,
    Rendition,
    Attachment,
    Media,
    Metadata,
    Other(String),
}

impl PartRole {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        Ok(match raw {
            "document" => Self::Document,
            "subresource" => Self::Subresource,
            "network-response" => Self::NetworkResponse,
            "network-trace" => Self::NetworkTrace,
            "screenshot" => Self::Screenshot,
            "print" => Self::Print,
            "rendition" => Self::Rendition,
            "attachment" => Self::Attachment,
            "media" => Self::Media,
            "metadata" => Self::Metadata,
            other => {
                if !crate::id::is_name(other) {
                    return Err(ModelError::InvalidEnvelope);
                }
                Self::Other(other.to_string())
            }
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Document => "document",
            Self::Subresource => "subresource",
            Self::NetworkResponse => "network-response",
            Self::NetworkTrace => "network-trace",
            Self::Screenshot => "screenshot",
            Self::Print => "print",
            Self::Rendition => "rendition",
            Self::Attachment => "attachment",
            Self::Media => "media",
            Self::Metadata => "metadata",
            Self::Other(name) => name,
        }
    }
}

impl Serialize for PartRole {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PartRole {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        PartRole::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// 渲染级别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderLevel {
    #[serde(rename = "L0")]
    L0,
    #[serde(rename = "L1")]
    L1,
    #[serde(rename = "L2")]
    L2,
    #[serde(rename = "L3")]
    L3,
    #[serde(rename = "L4")]
    L4,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Signals {
    #[serde(default, with = "humantime_serde::option")]
    pub retry_after: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub ttl: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub cache_max_age: Option<Duration>,
}

/// 访问目标。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Locator {
    Url { url: String },
    Query { query: String },
    ApiEndpoint { endpoint: String },
    PushChannel { channel: String },
}

impl Locator {
    pub fn validate(&self) -> Result<(), ModelError> {
        let text = match self {
            Self::Url { url } => url,
            Self::Query { query } => query,
            Self::ApiEndpoint { endpoint } => endpoint,
            Self::PushChannel { channel } => channel,
        };
        if text.is_empty() {
            Err(ModelError::InvalidEnvelope)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transport {
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub started_at: Timestamp,
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    pub bytes_in: u64,
    pub not_modified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_level: Option<RenderLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_id: Option<ReceiptId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawPart {
    pub part_id: PartId,
    pub role: PartRole,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
    pub bytes: PartRef,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawEnvelope {
    pub envelope_id: EnvelopeId,
    pub locator: Locator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_locator: Option<Locator>,
    pub transport: Transport,
    pub parts: Vec<RawPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_candidate: Option<Value>,
    #[serde(default)]
    pub signals: Signals,
}

impl RawEnvelope {
    pub fn validate(&self) -> Result<(), ModelError> {
        self.locator.validate()?;
        if let Some(locator) = &self.final_locator {
            locator.validate()?;
        }
        if self.transport.protocol.is_empty() {
            return Err(ModelError::InvalidEnvelope);
        }
        let mut seen = Vec::new();
        for part in &self.parts {
            if part.media_type.is_empty() || seen.contains(&part.part_id) {
                return Err(ModelError::InvalidEnvelope);
            }
            if let Some(locator) = &part.locator {
                locator.validate()?;
            }
            seen.push(part.part_id.clone());
        }
        Ok(())
    }
}
