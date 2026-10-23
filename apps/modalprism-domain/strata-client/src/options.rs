//! 门面选项。`system.yaml` 的 `strata` 段在这里翻译成数据层选项。

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use strata_gateway::{Backend, BlobBackend, DerivationSpec, Options as GatewayOptions, Tokenizer};

use crate::Error;

pub const INLINE_TEXT_MAX: u64 = 4 * 1024 * 1024;
pub const BLOB_INLINE_THRESHOLD: usize = 64 * 1024;
pub const BLOB_DEDICATED_THRESHOLD: usize = 16 * 1024 * 1024;
pub const STAGING_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 打开数据层的选项。缺省是 SQLite + 本地目录 Blob，全部落在数据目录内。
#[derive(Clone, Debug)]
pub struct Options {
    pub inline_text_max: u64,
    pub blob_inline_threshold: usize,
    pub blob_dedicated_threshold: usize,
    pub staging_ttl: Duration,
    /// 元数据后端，默认 SQLite。
    pub backend: Backend,
    /// Blob 后端，默认本地目录。
    pub blobs: BlobBackend,
    /// 全文索引分词方式，默认按空白与标点。
    pub tokenizer: Tokenizer,
    /// 声明式派生：媒体 facet → 派生 facet。
    pub derivations: Vec<DerivationSpec>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            inline_text_max: INLINE_TEXT_MAX,
            blob_inline_threshold: BLOB_INLINE_THRESHOLD,
            blob_dedicated_threshold: BLOB_DEDICATED_THRESHOLD,
            staging_ttl: STAGING_TTL,
            backend: Backend::default(),
            blobs: BlobBackend::default(),
            tokenizer: Tokenizer::default(),
            derivations: Vec::new(),
        }
    }
}

impl Options {
    /// 读取 `strata` 段。未写的键沿用默认值；取值合法性由配置 schema 保证。
    pub fn from_system(system: &Value) -> Result<Self, Error> {
        let mut options = Self::default();
        // 本地一键启动的缺省后端按编译能力选择：开了内置 S3 就用进程内 RustFS，
        // 否则用外部二进制（下载官方发行包）；两者都没有才回落到本地目录。
        #[cfg(feature = "embedded-s3")]
        {
            options.blobs = BlobBackend::Embedded {
                bucket: None,
                prefix: None,
                path: None,
            };
        }
        #[cfg(all(not(feature = "embedded-s3"), feature = "rustfs-bin"))]
        {
            options.blobs = rustfs_backend(system);
        }
        if let Some(size) = text(system, "/strata/blob/inline_size_threshold").and_then(parse_size)
        {
            options.blob_inline_threshold = size;
        }
        if let Some(size) =
            text(system, "/strata/blob/dedicated_size_threshold").and_then(parse_size)
        {
            options.blob_dedicated_threshold = size;
        }
        if let Some(ttl) = text(system, "/strata/staging/ttl").and_then(parse_duration) {
            options.staging_ttl = ttl;
        }
        if let Some(backend) = text(system, "/strata/gateway/backend") {
            options.backend = match backend {
                "postgres" => Backend::Postgres {
                    url: own(system, "/strata/gateway/url").unwrap_or_default(),
                },
                _ => Backend::Sqlite {
                    path: own(system, "/strata/gateway/path").map(PathBuf::from),
                    extensions: list(system, "/strata/gateway/extensions"),
                },
            };
        }
        if let Some(backend) = text(system, "/strata/gateway/blob_backend") {
            options.blobs = match backend {
                "memory" => BlobBackend::Memory,
                "embedded" => BlobBackend::Embedded {
                    bucket: own(system, "/strata/gateway/embedded/bucket"),
                    prefix: own(system, "/strata/gateway/embedded/prefix"),
                    path: own(system, "/strata/gateway/embedded/path").map(PathBuf::from),
                },
                "rustfs" => rustfs_backend(system),
                "s3" => BlobBackend::S3 {
                    bucket: own(system, "/strata/gateway/s3/bucket").unwrap_or_default(),
                    region: own(system, "/strata/gateway/s3/region"),
                    endpoint: own(system, "/strata/gateway/s3/endpoint"),
                    prefix: own(system, "/strata/gateway/s3/prefix"),
                    access_key_env: own(system, "/strata/gateway/s3/access_key_env"),
                    secret_key_env: own(system, "/strata/gateway/s3/secret_key_env"),
                },
                _ => BlobBackend::Dir {
                    path: own(system, "/strata/gateway/blob_path").map(PathBuf::from),
                },
            };
        }
        if let Some(tokenizer) = text(system, "/strata/fts/tokenizer") {
            options.tokenizer = match tokenizer {
                "trigram" => Tokenizer::Trigram,
                _ => Tokenizer::Unicode,
            };
        }
        if let Some(value) = system.pointer("/strata/gateway/derivations") {
            options.derivations = serde_json::from_value(value.clone())
                .map_err(|source| Error::InvalidDerivation(source.to_string()))?;
        }
        Ok(options)
    }

    pub(crate) fn gateway(&self) -> GatewayOptions {
        GatewayOptions {
            backend: self.backend.clone(),
            blobs: self.blobs.clone(),
            inline_threshold: self.blob_inline_threshold,
            dedicated_threshold: self.blob_dedicated_threshold,
            inline_text_max: self.inline_text_max,
            staging_ttl: self.staging_ttl,
            text: strata_gateway::TextOptions {
                tokenizer: self.tokenizer,
            },
            derivations: self.derivations.clone(),
            ..GatewayOptions::default()
        }
    }
}

/// 校验 `strata` 段与实际数据目录的关系。
///
/// 本地元数据与暂存区仍固定在数据目录内；PostgreSQL 需要连接串，
/// S3 需要桶名。不满足时统一返回“存储位置未规定”，调用方在创建目录前拒绝。
pub fn check_storage(system: &Value, data: &Path) -> Result<(), String> {
    if text(system, "/strata/gateway/backend") == Some("postgres") {
        if text(system, "/strata/gateway/url").is_none_or(|url| url.trim().is_empty()) {
            return Err("存储位置未规定".into());
        }
    } else if let Some(path) = text(system, "/strata/gateway/path") {
        if normalize(Path::new(path)) != normalize(&data.join("gateway.db")) {
            return Err("存储位置未规定".into());
        }
    }
    let default_blob = if cfg!(feature = "embedded-s3") {
        "embedded"
    } else if cfg!(feature = "rustfs-bin") {
        "rustfs"
    } else {
        "dir"
    };
    match text(system, "/strata/gateway/blob_backend").unwrap_or(default_blob) {
        "memory" => {}
        "embedded" => {
            if let Some(path) = text(system, "/strata/gateway/embedded/path") {
                if normalize(Path::new(path)) != normalize(&data.join("rustfs")) {
                    return Err("存储位置未规定".into());
                }
            }
        }
        "rustfs" => {
            if let Some(path) = text(system, "/strata/gateway/rustfs/path") {
                if normalize(Path::new(path)) != normalize(&data.join("rustfs")) {
                    return Err("存储位置未规定".into());
                }
            }
            if let Some(path) = text(system, "/strata/gateway/rustfs/cache") {
                if normalize(Path::new(path)) != normalize(&data.join("tools").join("rustfs")) {
                    return Err("存储位置未规定".into());
                }
            }
        }
        "s3" => {
            if text(system, "/strata/gateway/s3/bucket")
                .is_none_or(|bucket| bucket.trim().is_empty())
            {
                return Err("存储位置未规定".into());
            }
        }
        "dir" => {
            if let Some(path) = text(system, "/strata/gateway/blob_path") {
                if normalize(Path::new(path)) != normalize(&data.join("blobs")) {
                    return Err("存储位置未规定".into());
                }
            }
        }
        _ => return Err("存储位置未规定".into()),
    }
    if let Some(path) = text(system, "/strata/staging/dir") {
        if normalize(Path::new(path)) != normalize(&data.join("staging")) {
            return Err("存储位置未规定".into());
        }
    }
    Ok(())
}

fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn text<'a>(value: &'a Value, path: &str) -> Option<&'a str> {
    value.pointer(path).and_then(Value::as_str)
}

fn own(value: &Value, path: &str) -> Option<String> {
    text(value, path).map(str::to_string)
}

/// 外部 RustFS 后端的参数。未写 `blob_backend` 时也走这里，
/// 否则 `rustfs.offline` 这类开关在缺省后端下会被忽略。
fn rustfs_backend(system: &Value) -> BlobBackend {
    BlobBackend::Rustfs {
        exe: own(system, "/strata/gateway/rustfs/exe").map(PathBuf::from),
        cache: own(system, "/strata/gateway/rustfs/cache").map(PathBuf::from),
        offline: flag(system, "/strata/gateway/rustfs/offline"),
        bucket: own(system, "/strata/gateway/rustfs/bucket"),
        prefix: own(system, "/strata/gateway/rustfs/prefix"),
        path: own(system, "/strata/gateway/rustfs/path").map(PathBuf::from),
    }
}

fn flag(value: &Value, path: &str) -> bool {
    value
        .pointer(path)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn list(value: &Value, path: &str) -> Vec<String> {
    value
        .pointer(path)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_size(text: &str) -> Option<usize> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    usize::try_from(count.saturating_mul(scale)).ok()
}

fn parse_duration(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(Duration::from_millis(count.saturating_mul(scale)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_keep_storage_local() {
        let options = Options::from_system(&json!({})).unwrap();
        assert!(matches!(options.backend, Backend::Sqlite { .. }));
        // 开了内置 S3 的构建，缺省 Blob 后端就是进程内 RustFS；
        // 默认构建优先用外部二进制，两者都没有才落到本地目录。
        #[cfg(feature = "embedded-s3")]
        assert!(matches!(options.blobs, BlobBackend::Embedded { .. }));
        #[cfg(all(not(feature = "embedded-s3"), feature = "rustfs-bin"))]
        assert!(matches!(options.blobs, BlobBackend::Rustfs { .. }));
        #[cfg(all(not(feature = "embedded-s3"), not(feature = "rustfs-bin")))]
        assert!(matches!(options.blobs, BlobBackend::Dir { .. }));
        assert_eq!(options.blob_inline_threshold, BLOB_INLINE_THRESHOLD);
        assert_eq!(options.tokenizer, Tokenizer::Unicode);
        assert!(options.derivations.is_empty());
    }

    #[test]
    fn maps_gateway_section() {
        let options = Options::from_system(&json!({
            "strata": {
                "gateway": {
                    "backend": "sqlite",
                    "path": "var/gateway.db",
                    "extensions": ["sqlite_vec"],
                    "blob_backend": "s3",
                    "s3": {"bucket": "media"}
                },
                "blob": {"inline_size_threshold": "1KiB"},
                "staging": {"ttl": "2h"},
                "fts": {"tokenizer": "trigram"}
            }
        }))
        .unwrap();
        assert!(matches!(
            options.backend,
            Backend::Sqlite { ref path, ref extensions }
                if path.as_deref() == Some(std::path::Path::new("var/gateway.db"))
                    && extensions == &["sqlite_vec".to_string()]
        ));
        assert!(matches!(options.blobs, BlobBackend::S3 { ref bucket, .. } if bucket == "media"));
        assert_eq!(options.blob_inline_threshold, 1024);
        assert_eq!(options.staging_ttl, Duration::from_secs(7200));
        assert_eq!(options.tokenizer, Tokenizer::Trigram);
    }

    #[test]
    fn maps_derivations_into_gateway_options() {
        let system = json!({
            "strata": {
                "gateway": {
                    "derivations": [{
                        "name": "ocr",
                        "from": {"kind": "image", "variant": "image"},
                        "produces": "struct",
                        "variant": "ocr",
                        "producer": {
                            "module": "derive-ocr",
                            "version": "1.0.0",
                            "configGeneration": 2
                        },
                        "params": {"language": "zh"},
                        "queue": "render",
                        "accessor": "derive.ocr",
                        "priority": 10
                    }]
                }
            }
        });
        let options = Options::from_system(&system).unwrap();
        assert_eq!(options.derivations.len(), 1);
        assert_eq!(options.derivations[0].name, "ocr");
        assert_eq!(options.derivations[0].producer.config_generation, 2);
        assert_eq!(options.gateway().derivations.len(), 1);
    }

    #[test]
    fn rejects_bad_derivation() {
        let error = Options::from_system(&json!({
            "strata": {"gateway": {"derivations": [{"name": "ocr"}]}}
        }))
        .unwrap_err();
        assert!(error.to_string().starts_with("派生声明无效"), "{error}");
    }

    #[test]
    fn maps_embedded_blob_backend() {
        let data = tempfile::tempdir().unwrap();
        let rustfs = data.path().join("rustfs");
        let options = Options::from_system(&json!({
            "strata": {"gateway": {
                "blob_backend": "embedded",
                "embedded": {"bucket": "media", "prefix": "objects"}
            }}
        }))
        .unwrap();
        assert!(matches!(
            options.blobs,
            BlobBackend::Embedded { ref bucket, ref prefix, path: None }
                if bucket.as_deref() == Some("media") && prefix.as_deref() == Some("objects")
        ));
        assert!(
            check_storage(
                &json!({"strata": {"gateway": {"blob_backend": "embedded", "embedded": {
                    "path": rustfs.to_str().unwrap()
                }}}}),
                data.path()
            )
            .is_ok()
        );
    }

    #[test]
    fn maps_rustfs_binary_options() {
        let options = Options::from_system(&json!({
            "strata": {"gateway": {
                "blob_backend": "rustfs",
                "rustfs": {
                    "exe": "tools/rustfs.exe",
                    "cache": "tools/cache",
                    "offline": true,
                    "bucket": "media"
                }
            }}
        }))
        .unwrap();
        assert!(matches!(
            options.blobs,
            BlobBackend::Rustfs {
                ref exe,
                ref cache,
                offline: true,
                ref bucket,
                prefix: None,
                path: None,
            } if exe.as_deref() == Some(std::path::Path::new("tools/rustfs.exe"))
                && cache.as_deref() == Some(std::path::Path::new("tools/cache"))
                && bucket.as_deref() == Some("media")
        ));
    }

    /// 未写 `blob_backend` 时，缺省后端也要读 `rustfs` 段的开关。
    #[cfg(all(not(feature = "embedded-s3"), feature = "rustfs-bin"))]
    #[test]
    fn default_rustfs_backend_honors_offline() {
        let options = Options::from_system(&json!({
            "strata": {"gateway": {"rustfs": {"offline": true, "cache": "tools/cache"}}}
        }))
        .unwrap();
        assert!(matches!(
            options.blobs,
            BlobBackend::Rustfs {
                offline: true,
                ref cache,
                ..
            } if cache.as_deref() == Some(std::path::Path::new("tools/cache"))
        ));
    }

    #[test]
    fn storage_check_follows_the_active_backend() {
        let data = tempfile::tempdir().unwrap();
        let ok = |system: Value| check_storage(&system, data.path());
        assert!(
            ok(json!({"strata": {"gateway": {"backend": "postgres", "url": "postgres://x"}}}))
                .is_ok()
        );
        assert!(ok(json!({"strata": {"gateway": {"backend": "postgres"}}})).is_err());
        assert!(
            ok(json!({"strata": {"gateway": {"blob_backend": "s3", "s3": {"bucket": "media"}}}}))
                .is_ok()
        );
        assert!(ok(json!({"strata": {"gateway": {"blob_backend": "s3"}}})).is_err());
        assert!(ok(json!({"strata": {"gateway": {"blob_backend": "memory"}}})).is_ok());
        assert!(ok(json!({"strata": {"gateway": {"blob_backend": "embedded"}}})).is_ok());
        assert!(
            ok(json!({"strata": {"gateway": {"blob_backend": "embedded", "embedded": {"path": "elsewhere"}}}}))
                .is_err()
        );
        let gateway = data.path().join("gateway.db");
        assert!(ok(json!({"strata": {"gateway": {"path": gateway.to_str().unwrap()}}})).is_ok());
        assert!(ok(json!({"strata": {"gateway": {"path": "elsewhere.db"}}})).is_err());
        let staging = data.path().join("staging");
        assert!(ok(json!({"strata": {"staging": {"dir": staging.to_str().unwrap()}}})).is_ok());
        assert!(ok(json!({"strata": {"staging": {"dir": "elsewhere"}}})).is_err());
    }
}
