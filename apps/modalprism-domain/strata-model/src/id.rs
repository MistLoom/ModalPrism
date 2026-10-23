//! 资源 ID、不透明标识、命名空间与身份键。

use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ulid::Ulid;

use crate::error::ModelError;

/// `r_` 加 26 位 Crockford Base32 ULID。
///
/// `generate` 使用标准 ULID。它不提供多实例之间的单调顺序。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rid(String);

impl Rid {
    pub fn generate() -> Self {
        Self(format!("r_{}", Ulid::new()))
    }

    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let body = raw.strip_prefix("r_").ok_or(ModelError::InvalidRid)?;
        let ulid = Ulid::from_string(body).map_err(|_| ModelError::InvalidRid)?;
        Ok(Self(format!("r_{ulid}")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Rid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Rid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Rid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Rid::parse(&raw).map_err(serde::de::Error::custom)
    }
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(raw: impl Into<String>) -> Result<Self, ModelError> {
                let raw = raw.into();
                if !is_token(&raw) {
                    return Err(ModelError::EmptyId);
                }
                Ok(Self(raw))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ModelError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

opaque_id!(TaskId);
opaque_id!(CommitId);
opaque_id!(EnvelopeId);
opaque_id!(PartId);
opaque_id!(ReceiptId);
opaque_id!(SourceId);

fn is_token(raw: &str) -> bool {
    !raw.is_empty() && raw.chars().all(|c| c.is_ascii_graphic())
}

/// RFC 3339 时间戳。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(OffsetDateTime);

impl Timestamp {
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        OffsetDateTime::parse(raw, &Rfc3339)
            .map(Self)
            .map_err(|_| ModelError::InvalidTimestamp)
    }

    pub fn unix_millis(self) -> Result<i64, ModelError> {
        let millis = self.0.unix_timestamp_nanos().div_euclid(1_000_000);
        i64::try_from(millis).map_err(|_| ModelError::InvalidTimestamp)
    }

    pub fn from_unix_millis(millis: i64) -> Result<Self, ModelError> {
        let seconds = millis.div_euclid(1000);
        let nanos = u32::try_from(millis.rem_euclid(1000) * 1_000_000)
            .map_err(|_| ModelError::InvalidTimestamp)?;
        let time = OffsetDateTime::from_unix_timestamp(seconds)
            .map_err(|_| ModelError::InvalidTimestamp)?;
        time.replace_nanosecond(nanos)
            .map(Self)
            .map_err(|_| ModelError::InvalidTimestamp)
    }

    pub fn checked_add(self, duration: std::time::Duration) -> Result<Self, ModelError> {
        let duration =
            time::Duration::try_from(duration).map_err(|_| ModelError::InvalidTimestamp)?;
        self.0
            .checked_add(duration)
            .map(Self)
            .ok_or(ModelError::InvalidTimestamp)
    }

    pub fn to_rfc3339(self) -> Result<String, ModelError> {
        self.0
            .format(&Rfc3339)
            .map_err(|_| ModelError::InvalidTimestamp)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_rfc3339() {
            Ok(text) => f.write_str(&text),
            Err(_) => Err(fmt::Error),
        }
    }
}

impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.to_rfc3339().map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Timestamp::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// 资源命名空间：`data`、`sys`、`type:<name>`。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Namespace {
    Data,
    Sys,
    Type(String),
}

impl Namespace {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        match raw {
            "data" => Ok(Self::Data),
            "sys" => Ok(Self::Sys),
            _ => {
                let name = raw
                    .strip_prefix("type:")
                    .ok_or(ModelError::InvalidNamespace)?;
                if !is_name(name) {
                    return Err(ModelError::InvalidNamespace);
                }
                Ok(Self::Type(name.to_string()))
            }
        }
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data => f.write_str("data"),
            Self::Sys => f.write_str("sys"),
            Self::Type(name) => write!(f, "type:{name}"),
        }
    }
}

impl From<Namespace> for String {
    fn from(value: Namespace) -> String {
        value.to_string()
    }
}

impl TryFrom<String> for Namespace {
    type Error = ModelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// 身份键。URL 的规范化不在本类型中进行，`url:` 后面的文本原样保存。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum IdentityKey {
    External { scheme: String, id: String },
    Url { canonical: String },
    Source { source_id: String, hash: String },
}

impl IdentityKey {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        if let Some(rest) = raw.strip_prefix("ext:") {
            let (scheme, id) = rest.split_once(':').ok_or(ModelError::InvalidIdentity)?;
            if !is_name(scheme) || id.is_empty() || id.chars().any(char::is_whitespace) {
                return Err(ModelError::InvalidIdentity);
            }
            return Ok(Self::External {
                scheme: scheme.to_string(),
                id: id.to_string(),
            });
        }
        if let Some(rest) = raw.strip_prefix("url:") {
            if rest.is_empty() || rest.chars().any(char::is_whitespace) {
                return Err(ModelError::InvalidIdentity);
            }
            return Ok(Self::Url {
                canonical: rest.to_string(),
            });
        }
        if let Some(rest) = raw.strip_prefix("src:") {
            let (source_id, hash) = rest.split_once(':').ok_or(ModelError::InvalidIdentity)?;
            if !is_token(source_id) || !is_token(hash) {
                return Err(ModelError::InvalidIdentity);
            }
            return Ok(Self::Source {
                source_id: source_id.to_string(),
                hash: hash.to_string(),
            });
        }
        Err(ModelError::InvalidIdentity)
    }
}

impl fmt::Display for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::External { scheme, id } => write!(f, "ext:{scheme}:{id}"),
            Self::Url { canonical } => write!(f, "url:{canonical}"),
            Self::Source { source_id, hash } => write!(f, "src:{source_id}:{hash}"),
        }
    }
}

impl From<IdentityKey> for String {
    fn from(value: IdentityKey) -> String {
        value.to_string()
    }
}

impl TryFrom<String> for IdentityKey {
    type Error = ModelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

pub(crate) fn is_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}
