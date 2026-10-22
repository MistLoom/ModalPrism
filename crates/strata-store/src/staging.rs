//! 内容寻址暂存区。可见之后才删除被引用的文件。

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use strata_model::{PartRef, content_hash};
use tokio::io::AsyncWriteExt;

use crate::error::Error;

pub(crate) struct Staging {
    dir: PathBuf,
}

impl Staging {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(crate) async fn write(&self, bytes: &[u8]) -> Result<PartRef, Error> {
        let sha = content_hash(bytes);
        let path = self.dir.join(&sha);
        if tokio::fs::try_exists(&path).await? {
            self.check_file(&sha, bytes.len() as u64).await?;
            return PartRef::new(sha, bytes.len() as u64).map_err(Error::from);
        }
        let tmp = self.dir.join(format!(".tmp-{sha}"));
        {
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(bytes).await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&tmp, &path).await?;
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

    pub(crate) async fn check(&self, part: &PartRef) -> Result<(), Error> {
        self.check_file(part.sha256(), part.size()).await
    }

    pub(crate) async fn read(&self, sha: &str) -> Result<Vec<u8>, Error> {
        let bytes = self.read_file(sha).await?;
        if content_hash(&bytes) != sha {
            return Err(Error::StagingMismatch(sha.to_string()));
        }
        Ok(bytes)
    }

    pub(crate) async fn remove(&self, sha: &str) -> Result<(), Error> {
        match tokio::fs::remove_file(self.dir.join(sha)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// `keep` 中的哈希仍被已接收变更集引用。
    pub(crate) async fn sweep(
        &self,
        keep: &std::collections::HashSet<String>,
        ttl: Duration,
    ) -> Result<(), Error> {
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
            let modified = entry.metadata().await?.modified()?;
            let aged = now.duration_since(modified).unwrap_or(Duration::ZERO) >= ttl;
            if !aged {
                continue;
            }
            if is_sha_name(name) || name.starts_with(".tmp-") {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
        Ok(())
    }

    async fn check_file(&self, sha: &str, size: u64) -> Result<(), Error> {
        let bytes = self.read_file(sha).await?;
        if bytes.len() as u64 != size || content_hash(&bytes) != sha {
            return Err(Error::StagingMismatch(sha.to_string()));
        }
        Ok(())
    }

    async fn read_file(&self, sha: &str) -> Result<Vec<u8>, Error> {
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
