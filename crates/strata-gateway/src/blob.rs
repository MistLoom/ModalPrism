//! Blob 层。内容寻址、去重、区间读；小对象内联，大对象独立存放。
//!
//! 三级策略：
//! - `inline`：不超过内联阈值，字节随元数据行保存，读取不落盘；
//! - `packed`：内容寻址对象，普通写入；
//! - `dedicated`：达到独立阈值，单独对象并保留同步写语义（便于单独做保留策略）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use strata_model::content_hash;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::config::BlobBackend;
use crate::error::{Error, Result};

/// Blob 的存放层级，随元数据一起保存以便做保留/迁移策略。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlobTier {
    Inline,
    #[default]
    Packed,
    Dedicated,
}

impl BlobTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Packed => "packed",
            Self::Dedicated => "dedicated",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "inline" => Ok(Self::Inline),
            "packed" => Ok(Self::Packed),
            "dedicated" => Ok(Self::Dedicated),
            other => Err(Error::ColumnType(format!("未知 blob 层级: {other}"))),
        }
    }
}

/// 对象存储接口。`sha` 是内容的 sha256 十六进制串。
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, sha: &str, bytes: &[u8]) -> Result<()>;
    async fn get(&self, sha: &str) -> Result<Vec<u8>>;
    async fn range(&self, sha: &str, start: u64, len: u64) -> Result<Vec<u8>>;
    async fn contains(&self, sha: &str) -> Result<bool>;
    async fn remove(&self, sha: &str) -> Result<()>;
}

fn check_sha(sha: &str) -> Result<()> {
    if sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Error::BlobMismatch(sha.to_string()))
    }
}

/// 本地目录后端：`<root>/ab/<sha>`，按前缀分桶。
pub struct DirBlobStore {
    root: PathBuf,
}

impl DirBlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, sha: &str) -> PathBuf {
        self.root.join(&sha[..2]).join(sha)
    }
}

#[async_trait::async_trait]
impl BlobStore for DirBlobStore {
    async fn put(&self, sha: &str, bytes: &[u8]) -> Result<()> {
        check_sha(sha)?;
        let path = self.path(sha);
        if tokio::fs::try_exists(&path).await? {
            let size = tokio::fs::metadata(&path).await?.len();
            if size != bytes.len() as u64 {
                return Err(Error::BlobMismatch(sha.to_string()));
            }
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_file_name(format!(".tmp-{sha}-{}", ulid::Ulid::new()));
        {
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(bytes).await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        check_sha(sha)?;
        match tokio::fs::read(self.path(sha)).await {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::MissingBlob(sha.to_string()))
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn range(&self, sha: &str, start: u64, len: u64) -> Result<Vec<u8>> {
        check_sha(sha)?;
        if len == 0 {
            return Ok(Vec::new());
        }
        let path = self.path(sha);
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::MissingBlob(sha.to_string()));
            }
            Err(error) => return Err(error.into()),
        };
        let size = file.metadata().await?.len();
        let end = start.saturating_add(len);
        if end > size {
            return Err(Error::RangeOutOfBounds {
                sha: sha.to_string(),
                start,
                end,
                len: size,
            });
        }
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut buffer = vec![0u8; len as usize];
        file.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    async fn contains(&self, sha: &str) -> Result<bool> {
        check_sha(sha)?;
        Ok(tokio::fs::try_exists(self.path(sha)).await?)
    }

    async fn remove(&self, sha: &str) -> Result<()> {
        check_sha(sha)?;
        match tokio::fs::remove_file(self.path(sha)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// 进程内后端，供测试与小规模部署使用。
#[derive(Default)]
pub struct MemoryBlobStore {
    entries: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, sha: &str, bytes: &[u8]) -> Result<()> {
        check_sha(sha)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| Error::BlobMismatch("内存 blob 锁中毒".into()))?;
        match entries.get(sha) {
            Some(existing) if existing.len() != bytes.len() => {
                Err(Error::BlobMismatch(sha.to_string()))
            }
            Some(_) => Ok(()),
            None => {
                entries.insert(sha.to_string(), bytes.to_vec());
                Ok(())
            }
        }
    }

    async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        check_sha(sha)?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| Error::BlobMismatch("内存 blob 锁中毒".into()))?;
        entries
            .get(sha)
            .cloned()
            .ok_or_else(|| Error::MissingBlob(sha.to_string()))
    }

    async fn range(&self, sha: &str, start: u64, len: u64) -> Result<Vec<u8>> {
        let bytes = self.get(sha).await?;
        slice(&bytes, sha, start, len)
    }

    async fn contains(&self, sha: &str) -> Result<bool> {
        check_sha(sha)?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| Error::BlobMismatch("内存 blob 锁中毒".into()))?;
        Ok(entries.contains_key(sha))
    }

    async fn remove(&self, sha: &str) -> Result<()> {
        check_sha(sha)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| Error::BlobMismatch("内存 blob 锁中毒".into()))?;
        entries.remove(sha);
        Ok(())
    }
}

fn slice(bytes: &[u8], sha: &str, start: u64, len: u64) -> Result<Vec<u8>> {
    let end = start.saturating_add(len);
    if end > bytes.len() as u64 {
        return Err(Error::RangeOutOfBounds {
            sha: sha.to_string(),
            start,
            end,
            len: bytes.len() as u64,
        });
    }
    Ok(bytes[start as usize..end as usize].to_vec())
}

/// S3 / MinIO 后端。
#[cfg(feature = "s3")]
pub struct S3BlobStore {
    store: Arc<dyn object_store::ObjectStore>,
    prefix: object_store::path::Path,
}

#[cfg(feature = "s3")]
impl S3BlobStore {
    pub fn new(
        store: Arc<dyn object_store::ObjectStore>,
        prefix: impl AsRef<str>,
    ) -> Self {
        Self {
            store,
            prefix: object_store::path::Path::from(prefix.as_ref()),
        }
    }

    fn path(&self, sha: &str) -> object_store::path::Path {
        self.prefix.child(sha)
    }
}

#[cfg(feature = "s3")]
#[async_trait::async_trait]
impl BlobStore for S3BlobStore {
    async fn put(&self, sha: &str, bytes: &[u8]) -> Result<()> {
        check_sha(sha)?;
        use object_store::ObjectStore;
        if self.contains(sha).await? {
            return Ok(());
        }
        self.store
            .put(&self.path(sha), bytes.to_vec().into())
            .await
            .map_err(|error| Error::ObjectStore(error.to_string()))?;
        Ok(())
    }

    async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        check_sha(sha)?;
        use object_store::ObjectStore;
        let result = self
            .store
            .get(&self.path(sha))
            .await
            .map_err(|error| Error::ObjectStore(error.to_string()))?;
        let bytes = result
            .bytes()
            .await
            .map_err(|error| Error::ObjectStore(error.to_string()))?;
        Ok(bytes.to_vec())
    }

    async fn range(&self, sha: &str, start: u64, len: u64) -> Result<Vec<u8>> {
        check_sha(sha)?;
        if len == 0 {
            return Ok(Vec::new());
        }
        use object_store::ObjectStore;
        let end = start.saturating_add(len);
        let bytes = self
            .store
            .get_range(&self.path(sha), start as usize..end as usize)
            .await
            .map_err(|error| Error::ObjectStore(error.to_string()))?;
        Ok(bytes.to_vec())
    }

    async fn contains(&self, sha: &str) -> Result<bool> {
        check_sha(sha)?;
        use object_store::ObjectStore;
        match self.store.head(&self.path(sha)).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(error) => Err(Error::ObjectStore(error.to_string())),
        }
    }

    async fn remove(&self, sha: &str) -> Result<()> {
        check_sha(sha)?;
        use object_store::ObjectStore;
        match self.store.delete(&self.path(sha)).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(Error::ObjectStore(error.to_string())),
        }
    }
}

/// 按配置打开 Blob 后端。
pub async fn open_store(backend: &BlobBackend, root: &Path) -> Result<Arc<dyn BlobStore>> {
    match backend {
        BlobBackend::Dir { path } => {
            let path = path.clone().unwrap_or_else(|| root.join("blobs"));
            tokio::fs::create_dir_all(&path).await?;
            Ok(Arc::new(DirBlobStore::new(path)))
        }
        BlobBackend::Memory => Ok(Arc::new(MemoryBlobStore::new())),
        BlobBackend::S3 {
            bucket,
            region,
            endpoint,
            prefix,
            access_key_env,
            secret_key_env,
        } => {
            #[cfg(not(feature = "s3"))]
            {
                let _ = (bucket, region, endpoint, prefix, access_key_env, secret_key_env);
                Err(Error::BackendDisabled("s3"))
            }
            #[cfg(feature = "s3")]
            {
                use object_store::aws::AmazonS3Builder;
                let mut builder = AmazonS3Builder::from_env()
                    .with_bucket_name(bucket)
                    .with_allow_http(true);
                if let Some(region) = region {
                    builder = builder.with_region(region);
                }
                if let Some(endpoint) = endpoint {
                    builder = builder.with_endpoint(endpoint);
                }
                if let Some(env) = access_key_env {
                    let value = std::env::var(env).map_err(|_| {
                        Error::InvalidConfig(format!("环境变量未设置: {env}"))
                    })?;
                    builder = builder.with_access_key_id(value);
                }
                if let Some(env) = secret_key_env {
                    let value = std::env::var(env).map_err(|_| {
                        Error::InvalidConfig(format!("环境变量未设置: {env}"))
                    })?;
                    builder = builder.with_secret_access_key(value);
                }
                let store = builder
                    .build()
                    .map_err(|error| Error::ObjectStore(error.to_string()))?;
                let prefix = prefix.clone().unwrap_or_else(|| "strata".to_string());
                Ok(Arc::new(S3BlobStore::new(Arc::new(store), prefix)))
            }
        }
    }
}

/// 决定一个字节串落在哪一层，并把对象放进去。
pub struct BlobService {
    store: Arc<dyn BlobStore>,
    inline_threshold: usize,
    dedicated_threshold: usize,
}

/// 入库前的 blob 描述。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedBlob {
    pub sha256: String,
    pub size: u64,
    pub tier: BlobTier,
    pub inline: Option<Vec<u8>>,
}

impl BlobService {
    pub fn new(
        store: Arc<dyn BlobStore>,
        inline_threshold: usize,
        dedicated_threshold: usize,
    ) -> Result<Self> {
        if inline_threshold == 0 || inline_threshold > dedicated_threshold {
            return Err(Error::BlobThreshold {
                inline: inline_threshold,
                dedicated: dedicated_threshold,
            });
        }
        Ok(Self {
            store,
            inline_threshold,
            dedicated_threshold,
        })
    }

    pub async fn prepare(&self, bytes: &[u8]) -> Result<PreparedBlob> {
        let sha256 = content_hash(bytes);
        let size = bytes.len() as u64;
        if bytes.len() <= self.inline_threshold {
            return Ok(PreparedBlob {
                sha256,
                size,
                tier: BlobTier::Inline,
                inline: Some(bytes.to_vec()),
            });
        }
        self.store.put(&sha256, bytes).await?;
        let tier = if bytes.len() >= self.dedicated_threshold {
            BlobTier::Dedicated
        } else {
            BlobTier::Packed
        };
        Ok(PreparedBlob {
            sha256,
            size,
            tier,
            inline: None,
        })
    }

    /// 内联字节优先，否则回落到对象存储。
    pub async fn read(&self, sha: &str, inline: Option<&[u8]>) -> Result<Vec<u8>> {
        match inline {
            Some(bytes) => Ok(bytes.to_vec()),
            None => self.store.get(sha).await,
        }
    }

    pub async fn range(
        &self,
        sha: &str,
        inline: Option<&[u8]>,
        start: u64,
        len: u64,
    ) -> Result<Vec<u8>> {
        match inline {
            Some(bytes) => slice(bytes, sha, start, len),
            None => self.store.range(sha, start, len).await,
        }
    }

    pub async fn contains(&self, sha: &str, inline: Option<&[u8]>) -> Result<bool> {
        match inline {
            Some(_) => Ok(true),
            None => self.store.contains(sha).await,
        }
    }

    pub async fn remove(&self, sha: &str) -> Result<()> {
        self.store.remove(sha).await
    }

    pub fn store(&self) -> &dyn BlobStore {
        self.store.as_ref()
    }
}
