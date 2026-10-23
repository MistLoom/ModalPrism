//! 同步写入暂存区。文件名是内容哈希，与存储打开时的暂存目录相同。

use std::path::PathBuf;

use accessors::{AccessError, Staging};
use strata_model::{ErrorClass, PartRef, content_hash};

pub struct DiskStaging {
    pub dir: PathBuf,
}

impl Staging for DiskStaging {
    fn put(&self, bytes: &[u8]) -> Result<PartRef, AccessError> {
        let sha = content_hash(bytes);
        let path = self.dir.join(&sha);
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if path.is_file() {
            let meta = std::fs::metadata(&path).map_err(|source| {
                AccessError::with_source(ErrorClass::Transient, "staging", source)
            })?;
            if meta.len() != size {
                return Err(AccessError::new(ErrorClass::Permanent, "staging"));
            }
            return PartRef::new(sha, size).map_err(|source| {
                AccessError::with_source(ErrorClass::Permanent, "staging", source)
            });
        }
        let tmp = self.dir.join(format!(".tmp-{sha}"));
        std::fs::write(&tmp, bytes)
            .map_err(|source| AccessError::with_source(ErrorClass::Transient, "staging", source))?;
        std::fs::rename(&tmp, &path)
            .map_err(|source| AccessError::with_source(ErrorClass::Transient, "staging", source))?;
        PartRef::new(sha, size)
            .map_err(|source| AccessError::with_source(ErrorClass::Permanent, "staging", source))
    }
}

pub fn read_part(dir: &std::path::Path, part: &PartRef) -> Result<Vec<u8>, AccessError> {
    let bytes = std::fs::read(dir.join(part.sha256()))
        .map_err(|source| AccessError::with_source(ErrorClass::Permanent, "staging", source))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != part.size() {
        return Err(AccessError::new(ErrorClass::Permanent, "staging"));
    }
    Ok(bytes)
}
