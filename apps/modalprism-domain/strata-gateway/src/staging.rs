//! 内容寻址暂存区。入库可见之前，大对象先落在这里。
//!
//! 文件名就是内容的 sha256；引用方通过 `PartRef` 校验大小与哈希。可见之后
//! 由提交层删除，未被引用的文件按 TTL 清理。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use strata_model::{PartRef, content_hash};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};

pub struct Staging {
    dir: PathBuf,
}

impl Staging {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub async fn open(root: &Path) -> Result<Self> {
        let dir = root.join("staging");
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self::new(dir))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn write(&self, bytes: &[u8]) -> Result<PartRef> {
        let sha = content_hash(bytes);
        let path = self.dir.join(&sha);
        if tokio::fs::try_exists(&path).await? {
            self.check_file(&sha, bytes.len() as u64).await?;
            return PartRef::new(sha, bytes.len() as u64).map_err(Error::from);
        }
        let tmp = self.dir.join(format!(".tmp-{sha}-{}", ulid::Ulid::new()));
        {
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(bytes).await?;
            file.sync_all().await?;
        }
        if let Err(error) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            // 同一哈希可能被两个提交同时写入：先到者已经改名，后到者核对长度即可。
            if tokio::fs::try_exists(&path).await? {
                self.check_file(&sha, bytes.len() as u64).await?;
                return PartRef::new(sha, bytes.len() as u64).map_err(Error::from);
            }
            return Err(error.into());
        }
        // 目录 fsync 需要按平台分别处理：Windows 用普通 open 打开目录会报
        // PermissionDenied（需要 FILE_FLAG_BACKUP_SEMANTICS）。
        #[cfg(unix)]
        {
            tokio::fs::File::open(&self.dir).await?.sync_all().await?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            // FlushFileBuffers（sync_all）要求句柄具备写权限，否则报 PermissionDenied。
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&self.dir)?
                .sync_all()?;
        }
        PartRef::new(sha, bytes.len() as u64).map_err(Error::from)
    }

    pub async fn check(&self, part: &PartRef) -> Result<()> {
        self.check_file(part.sha256(), part.size()).await
    }

    pub async fn read(&self, sha: &str) -> Result<Vec<u8>> {
        let bytes = self.read_file(sha).await?;
        if content_hash(&bytes) != sha {
            return Err(Error::StagingMismatch(sha.to_string()));
        }
        Ok(bytes)
    }

    pub async fn remove(&self, sha: &str) -> Result<()> {
        match tokio::fs::remove_file(self.dir.join(sha)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// `keep` 中的哈希仍被已接收变更集引用。
    pub async fn sweep(&self, keep: &HashSet<String>, ttl: Duration) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        let now = SystemTime::now();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if keep.contains(name) {
                continue;
            }
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                // 另一个清理者已经删掉这个文件。
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let modified = metadata.modified()?;
            let aged = now.duration_since(modified).unwrap_or(Duration::ZERO) >= ttl;
            if !aged {
                continue;
            }
            if is_sha_name(name) || name.starts_with(".tmp-") {
                match tokio::fs::remove_file(entry.path()).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    async fn check_file(&self, sha: &str, size: u64) -> Result<()> {
        let bytes = self.read_file(sha).await?;
        if bytes.len() as u64 != size || content_hash(&bytes) != sha {
            return Err(Error::StagingMismatch(sha.to_string()));
        }
        Ok(())
    }

    async fn read_file(&self, sha: &str) -> Result<Vec<u8>> {
        let path = self.dir.join(sha);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::MissingStaging(sha.to_string()))
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn is_sha_name(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}
