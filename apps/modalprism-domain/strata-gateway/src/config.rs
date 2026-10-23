//! 数据层配置。声明式派生声明也在这里。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strata_model::{FacetKind, FacetSelector, Producer, QueueName};

use crate::error::Error;

pub const DEFAULT_INLINE_THRESHOLD: usize = 64 * 1024;
pub const DEFAULT_DEDICATED_THRESHOLD: usize = 16 * 1024 * 1024;
pub const DEFAULT_INLINE_TEXT_MAX: u64 = 4 * 1024 * 1024;
pub const DEFAULT_STAGING_TTL: Duration = Duration::from_secs(24 * 60 * 60);

fn default_inline() -> usize {
    DEFAULT_INLINE_THRESHOLD
}

fn default_dedicated() -> usize {
    DEFAULT_DEDICATED_THRESHOLD
}

fn default_inline_text() -> u64 {
    DEFAULT_INLINE_TEXT_MAX
}

fn default_staging_ttl() -> Duration {
    DEFAULT_STAGING_TTL
}

fn default_queue() -> QueueName {
    QueueName::Render
}

fn default_priority() -> i32 {
    100
}

/// `strata.gateway` 段的配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// 元数据后端。
    #[serde(default)]
    pub backend: Backend,
    /// Blob 后端。
    #[serde(default)]
    pub blobs: BlobBackend,
    /// 小于等于该值的 blob 内联进元数据行。
    #[serde(default = "default_inline")]
    pub inline_threshold: usize,
    /// 大于等于该值的 blob 按专属对象存放。
    #[serde(default = "default_dedicated")]
    pub dedicated_threshold: usize,
    /// 超过该字节数的文本正文改存 blob，行上只留引用。
    #[serde(default = "default_inline_text")]
    pub inline_text_max: u64,
    /// 暂存区未被引用的文件保留时长。
    #[serde(default = "default_staging_ttl", with = "humantime_serde")]
    pub staging_ttl: Duration,
    /// 全文索引配置。
    #[serde(default)]
    pub text: TextOptions,
    /// 融合排序配置。
    #[serde(default)]
    pub fusion: FusionOptions,
    /// 派生声明。入库可见后按此触发派生任务。
    #[serde(default)]
    pub derivations: Vec<DerivationSpec>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            backend: Backend::default(),
            blobs: BlobBackend::default(),
            inline_threshold: DEFAULT_INLINE_THRESHOLD,
            dedicated_threshold: DEFAULT_DEDICATED_THRESHOLD,
            inline_text_max: DEFAULT_INLINE_TEXT_MAX,
            staging_ttl: DEFAULT_STAGING_TTL,
            text: TextOptions::default(),
            fusion: FusionOptions::default(),
            derivations: Vec::new(),
        }
    }
}

impl Options {
    pub fn validate(&self) -> Result<(), Error> {
        if self.inline_threshold == 0 || self.inline_threshold > self.dedicated_threshold {
            return Err(Error::BlobThreshold {
                inline: self.inline_threshold,
                dedicated: self.dedicated_threshold,
            });
        }
        if self.inline_text_max == 0 {
            return Err(Error::InvalidConfig("内联文本上限不能为 0".into()));
        }
        let mut names = std::collections::BTreeSet::new();
        for spec in &self.derivations {
            spec.validate()?;
            if !names.insert(spec.name.clone()) {
                return Err(Error::InvalidDerivation(format!(
                    "派生声明重名: {}",
                    spec.name
                )));
            }
        }
        for weight in [
            self.fusion.filter_weight,
            self.fusion.vector_weight,
            self.fusion.text_weight,
        ] {
            if !weight.is_finite() || weight < 0.0 {
                return Err(Error::InvalidConfig("融合权重必须是非负有限数".into()));
            }
        }
        Ok(())
    }
}

/// 元数据后端。SQLite 是默认；PostgreSQL 需要打开 `postgres` feature。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Backend {
    /// 单文件 SQLite，可附加 `sqlite-vec` 等扩展。
    Sqlite {
        /// 缺省是 `<root>/gateway.db`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        /// 打开连接时装载的 SQLite 扩展名（例如 `sqlite_vec`）。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extensions: Vec<String>,
    },
    /// PostgreSQL；pgvector 可选。
    Postgres { url: String },
}

impl Default for Backend {
    fn default() -> Self {
        Self::Sqlite {
            path: None,
            extensions: Vec::new(),
        }
    }
}

impl Backend {
    /// 后端名字，供启动日志与诊断。
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sqlite { .. } => "sqlite",
            Self::Postgres { .. } => "postgres",
        }
    }
}

/// Blob 后端。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlobBackend {
    /// 本地目录，内容寻址。
    Dir {
        /// 缺省是 `<root>/blobs`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
    },
    /// 进程内存储，供测试使用。
    Memory,
    /// 进程内 RustFS：本地一键启动用的 S3 后端，服务随数据层启停。
    /// 需要打开 `embedded-s3` feature。
    Embedded {
        /// 桶名，缺省 `strata`。启动时自动创建。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bucket: Option<String>,
        /// 对象前缀，缺省 `strata`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        /// RustFS 数据卷，缺省 `<root>/rustfs`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
    },
    /// 外部 RustFS 二进制：下载官方发行包并按子进程托管。
    /// 需要打开 `rustfs-bin` feature（默认开启）。
    Rustfs {
        /// 显式指定的可执行文件。未写时依次尝试缓存、`PATH`、下载。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exe: Option<PathBuf>,
        /// 二进制缓存根，缺省 `<root>/tools/rustfs`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<PathBuf>,
        /// 只在本机找二进制：不联网下载，缺了直接报错。缺省 `false`。
        #[serde(default, skip_serializing_if = "is_false")]
        offline: bool,
        /// 桶名，缺省 `strata`。启动时自动创建。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bucket: Option<String>,
        /// 对象前缀，缺省 `strata`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        /// RustFS 数据卷，缺省 `<root>/rustfs`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
    },
    /// S3 兼容对象存储（AWS S3 / RustFS）。需要打开 `s3` feature。
    S3 {
        bucket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        /// 访问凭据从这两个环境变量读取，不落配置文件。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret_key_env: Option<String>,
    },
}

impl Default for BlobBackend {
    fn default() -> Self {
        Self::Dir { path: None }
    }
}

impl BlobBackend {
    /// 后端名字，供启动日志与诊断。
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dir { .. } => "dir",
            Self::Memory => "memory",
            Self::Embedded { .. } => "embedded",
            Self::Rustfs { .. } => "rustfs",
            Self::S3 { .. } => "s3",
        }
    }
}

/// `serde` 用：布尔字段为假时不写出来。
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tokenizer {
    /// FTS5 `unicode61`，按空白与标点切词。
    #[default]
    Unicode,
    /// FTS5 `trigram`，适合无空格分词的中文。
    Trigram,
}

impl Tokenizer {
    pub fn fts5_name(self) -> &'static str {
        match self {
            Self::Unicode => "unicode61",
            Self::Trigram => "trigram",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextOptions {
    #[serde(default)]
    pub tokenizer: Tokenizer,
}

/// 融合排序参数。三路候选按倒数排名融合（RRF）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionOptions {
    #[serde(default = "default_rrf_k")]
    pub rrf_k: u32,
    #[serde(default = "default_weight")]
    pub filter_weight: f32,
    #[serde(default = "default_weight")]
    pub vector_weight: f32,
    #[serde(default = "default_weight")]
    pub text_weight: f32,
}

fn default_rrf_k() -> u32 {
    60
}

fn default_weight() -> f32 {
    1.0
}

impl Default for FusionOptions {
    fn default() -> Self {
        Self {
            rrf_k: default_rrf_k(),
            filter_weight: default_weight(),
            vector_weight: default_weight(),
            text_weight: default_weight(),
        }
    }
}

/// 一条派生声明：媒体 facet → 派生 facet。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivationSpec {
    /// 注册表内唯一，同时用作任务的 source。
    pub name: String,
    /// 输入 facet 选择器。
    pub from: FacetSelector,
    /// 派生 facet 的种类。
    pub produces: FacetKind,
    /// 派生 facet 的 variant；`produces` 要求 variant 时必填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// 产出方。版本或配置代际变化会触发重算。
    pub producer: Producer,
    /// 派生参数。参与 `params_hash`。
    #[serde(default)]
    pub params: Value,
    /// 执行队列，默认 render。
    #[serde(default = "default_queue")]
    pub queue: QueueName,
    /// 执行任务时登记的 accessor 名，默认用声明名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessor: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: i32,
}

impl DerivationSpec {
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() || !self.name.chars().all(|c| c.is_ascii_graphic()) {
            return Err(Error::InvalidDerivation(format!(
                "声明名必须是可见 ASCII 字符: {:?}",
                self.name
            )));
        }
        if self.producer.module.is_empty() || self.producer.version.is_empty() {
            return Err(Error::InvalidDerivation(format!(
                "{} 的产出方不完整",
                self.name
            )));
        }
        if self.variant.is_none() && self.produces.requires_variant() {
            return Err(Error::InvalidDerivation(format!(
                "{} 的派生种类要求 variant",
                self.name
            )));
        }
        if !(self.params.is_null() || self.params.is_object()) {
            return Err(Error::InvalidDerivation(format!(
                "{} 的 params 必须是对象",
                self.name
            )));
        }
        if self
            .accessor
            .as_ref()
            .is_some_and(|accessor| accessor.is_empty())
        {
            return Err(Error::InvalidDerivation(format!(
                "{} 的 accessor 为空",
                self.name
            )));
        }
        Ok(())
    }

    pub fn accessor_name(&self) -> &str {
        self.accessor.as_deref().unwrap_or(&self.name)
    }

    /// 参数指纹。模型换代或参数调整都会改变它。
    pub fn params_hash(&self) -> String {
        let canonical = if self.params.is_null() {
            "{}".to_string()
        } else {
            let mut text = serde_json::to_string(&self.params).unwrap_or_default();
            text.retain(|c| !c.is_whitespace());
            text
        };
        strata_model::content_hash(
            format!(
                "{}:{}:{}:{}",
                self.name, self.producer.module, self.producer.version, canonical
            )
            .as_bytes(),
        )
    }

    /// 派生 facet 的 variant；无 variant 时用空串占位。
    pub fn variant_key(&self) -> String {
        self.variant.clone().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> DerivationSpec {
        DerivationSpec {
            name: "ocr".into(),
            from: FacetSelector {
                kind: FacetKind::Image,
                variant: Some("image".into()),
                fields: Vec::new(),
            },
            produces: FacetKind::Struct,
            variant: Some("ocr".into()),
            producer: Producer {
                module: "derive-ocr".into(),
                version: "1.0.0".into(),
                config_generation: 1,
            },
            params: json!({"language": "zh"}),
            queue: QueueName::Render,
            accessor: None,
            priority: 100,
        }
    }

    #[test]
    fn params_hash_changes_with_version() {
        let spec = spec();
        let hash = spec.params_hash();
        let mut other = spec.clone();
        other.producer.version = "1.0.1".into();
        assert_ne!(hash, other.params_hash());
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn rejects_bad_thresholds() {
        let options = Options {
            inline_threshold: 0,
            ..Options::default()
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn serde_defaults() {
        let options: Options = serde_json::from_value(json!({})).unwrap();
        assert_eq!(options.inline_threshold, DEFAULT_INLINE_THRESHOLD);
        assert_eq!(options.fusion.rrf_k, 60);
        assert!(matches!(options.backend, Backend::Sqlite { .. }));
    }
}
