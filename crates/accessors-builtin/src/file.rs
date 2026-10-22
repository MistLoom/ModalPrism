//! `file` 访问器。读取本地文件。

use std::path::{Path, PathBuf};
use std::time::Instant;

use accessors::{
    AccessContext, AccessError, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::Value;
use strata_model::Locator;

use crate::envelope::{Page, from_page};
use crate::ready::{config, permanent, ready, spec_error};

pub struct FileAccessor;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Spec {
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

fn parse_spec(spec: &Value) -> Result<Spec, AccessError> {
    if spec.is_null() {
        return Ok(Spec {
            root: None,
            media_type: None,
            path: None,
        });
    }
    serde_json::from_value(spec.clone()).map_err(spec_error)
}

fn raw_path(spec: &Spec, locator: &Locator) -> Result<String, AccessError> {
    if let Some(path) = &spec.path {
        return Ok(path.clone());
    }
    match locator {
        Locator::Url { url } => Ok(url.strip_prefix("file://").unwrap_or(url).to_string()),
        _ => Err(config("locator")),
    }
}

fn locate(root: Option<&Path>, raw: &str) -> Result<PathBuf, AccessError> {
    let path = Path::new(raw);
    if let Some(root) = root {
        let root = std::fs::canonicalize(root).map_err(|_| permanent("not-found"))?;
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let full = std::fs::canonicalize(&joined).map_err(|_| permanent("not-found"))?;
        if !full.starts_with(&root) {
            return Err(permanent("not-found"));
        }
        Ok(full)
    } else if path.is_absolute() {
        std::fs::canonicalize(path).map_err(|_| permanent("not-found"))
    } else {
        Err(config("locator"))
    }
}

async fn read_opened(root: Option<&Path>, path: &Path) -> Result<Vec<u8>, AccessError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| permanent("not-found"))?;
    if let Some(root) = root {
        let root = std::fs::canonicalize(root).map_err(|_| permanent("not-found"))?;
        let opened = opened_path(&file, path).await?;
        if !opened.starts_with(&root) {
            return Err(permanent("not-found"));
        }
    }
    let mut bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut file, &mut bytes)
        .await
        .map_err(|_| permanent("not-found"))?;
    Ok(bytes)
}

#[cfg(unix)]
async fn opened_path(file: &tokio::fs::File, _path: &Path) -> Result<PathBuf, AccessError> {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    std::fs::canonicalize(format!("/proc/self/fd/{fd}")).map_err(|_| permanent("not-found"))
}

#[cfg(windows)]
async fn opened_path(file: &tokio::fs::File, path: &Path) -> Result<PathBuf, AccessError> {
    // Windows 没有 /proc：以打开句柄的文件身份与规范化路径比对，拒绝 locate 之后被替换的链接。
    let cloned = file
        .try_clone()
        .await
        .map_err(|_| permanent("not-found"))?
        .into_std()
        .await;
    let opened = same_file::Handle::from_file(cloned).map_err(|_| permanent("not-found"))?;
    let canon = std::fs::canonicalize(path).map_err(|_| permanent("not-found"))?;
    let expected = same_file::Handle::from_path(&canon).map_err(|_| permanent("not-found"))?;
    if opened != expected {
        return Err(permanent("not-found"));
    }
    Ok(canon)
}

#[async_trait]
impl Accessor for FileAccessor {
    fn id(&self) -> &str {
        "file"
    }

    fn version(&self) -> &str {
        "1"
    }

    fn capabilities(&self) -> &AccessorCapabilities {
        &CAPS
    }

    fn validate(&self, spec: &Value) -> Result<(), AccessError> {
        parse_spec(spec).map(|_| ())
    }

    async fn access(
        &self,
        request: AccessRequest,
        ctx: &AccessContext,
    ) -> Result<Vec<strata_model::RawEnvelope>, AccessError> {
        ready(ctx, request.deadline)?;
        let spec = parse_spec(&request.spec)?;
        let raw = raw_path(&spec, &request.locator)?;
        let root = spec.root.as_deref().map(Path::new);
        let path = locate(root, &raw)?;
        let bytes = read_opened(root, &path).await?;
        let media = spec
            .media_type
            .unwrap_or_else(|| "application/octet-stream".into());
        let started = Instant::now();
        Ok(vec![from_page(
            ctx,
            &request.policy,
            started,
            Page {
                locator: request.locator.clone(),
                final_url: format!("file://{}", path.display()),
                status: 200,
                headers: vec![("Content-Type".into(), media)],
                body: Bytes::from(bytes),
                cursor: None,
                receipt_id: None,
                render_level: None,
                engine: None,
                extra: Vec::new(),
                document: true,
            },
        )?])
    }
}

fn caps() -> AccessorCapabilities {
    AccessorCapabilities {
        protocols: vec!["file".into()],
        conditional: false,
        render: false,
        layout: false,
        capture_network: CaptureNetwork::None,
        interact: false,
        paid: None,
        batchable: false,
        inbound: false,
    }
}

static CAPS: std::sync::LazyLock<AccessorCapabilities> = std::sync::LazyLock::new(caps);

#[cfg(all(test, unix))]
mod tests {
    use super::{locate, read_opened};

    #[tokio::test]
    async fn swapped_symlink_after_locate_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside = root.path().join("note.txt");
        std::fs::write(&inside, b"safe").unwrap();
        std::fs::write(outside.path().join("secret"), b"secret").unwrap();
        let path = locate(Some(root.path()), "note.txt").unwrap();
        std::fs::remove_file(&inside).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), &inside).unwrap();
        let error = read_opened(Some(root.path()), &path).await.unwrap_err();
        assert_eq!(error.message, "not-found");
    }
}
