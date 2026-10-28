//! 门面 DTO。调用方与后端都只认这些字段。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Space {
    Persist,
    Temp,
}

impl Space {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.unwrap_or("persist") {
            "" | "persist" => Ok(Self::Persist),
            "temp" => Ok(Self::Temp),
            other => Err(format!("非法 space: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Json,
    Yaml,
    Xml,
    Md,
    Bin,
    Sqlite,
}

impl Codec {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Xml => "xml",
            Self::Md => "md",
            Self::Bin => "bin",
            Self::Sqlite => "sqlite",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "json" => Ok(Self::Json),
            "yaml" | "yml" => Ok(Self::Yaml),
            "xml" => Ok(Self::Xml),
            "md" | "markdown" => Ok(Self::Md),
            "bin" | "bytes" => Ok(Self::Bin),
            "sqlite" | "db" => Ok(Self::Sqlite),
            other => Err(format!("非法 codec: {other}")),
        }
    }

    pub fn from_key(key: &str) -> Self {
        let ext = key.rsplit('.').next().unwrap_or("");
        match ext {
            "json" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "xml" => Self::Xml,
            "md" | "markdown" => Self::Md,
            "bin" | "dat" => Self::Bin,
            "sqlite" | "db" => Self::Sqlite,
            _ => Self::Json,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GetArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    #[serde(default)]
    pub codec: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetOut {
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub codec: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PutArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    pub value: Value,
    #[serde(default)]
    pub codec: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    pub patch: Value,
    #[serde(default)]
    pub codec: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    #[serde(default)]
    pub codec: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    #[serde(default)]
    pub prefix: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListOut {
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobGetArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub len: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobOut {
    pub data: String,
    pub eof: bool,
    pub len: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobPutArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    pub data: String,
    #[serde(default)]
    pub append: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobWriteArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
    pub offset: u64,
    pub data: String,
    #[serde(default)]
    pub last: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcquireArgs {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub space: Option<String>,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcquireOut {
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseArgs {
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatOut {
    pub exists: bool,
    pub len: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OkOut {
    pub ok: bool,
}
