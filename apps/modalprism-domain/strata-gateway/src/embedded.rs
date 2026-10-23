//! 进程内 RustFS：本地一键启动用的 S3 后端。
//!
//! 不依赖外部进程：`RustFSServerBuilder` 在当前进程里起一个 S3 兼容服务，只监听
//! 回环地址的随机端口，数据卷落在数据目录内。桶在启动后由这里显式创建，
//! `RustFSServer` 随 [`EmbeddedBlobStore`] 一起析构。
//!
//! # 为什么另起一个线程
//!
//! 两点原因：
//! - 启动路径的栈用量超过默认主线程的 1 MiB。`#[tokio::main]` 的 `block_on`
//!   就发生在主线程上，直接 await 会栈溢出；这里给专用线程固定 16 MiB。
//! - S3 服务要有自己的 runtime 长期存活，不能依附调用方的 runtime（它可能在
//!   数据层关闭前先结束）。线程里跑一个独立 runtime，关闭时 `server.shutdown()`。
//!
//! 之所以固定凭据：服务只绑定 `127.0.0.1`，凭据不落配置文件也不出进程；每次
//! 启动换随机密钥反而会与 RustFS 持久化在数据卷里的 IAM 状态冲突。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustfs::embedded::{RustFSServer, RustFSServerBuilder, find_available_port};

use crate::blob::{BlobStore, S3BlobStore};
use crate::error::{Error, Result};

/// 默认桶名。
pub const DEFAULT_BUCKET: &str = "strata";
/// 默认对象前缀。
pub const DEFAULT_PREFIX: &str = "strata";

const ACCESS_KEY: &str = "strata";
const SECRET_KEY: &str = "strata-embedded-local";
const REGION: &str = "us-east-1";
/// 启动路径的栈用量超过默认主线程的 1 MiB，这里给足余量。
const SERVER_STACK: usize = 16 * 1024 * 1024;
/// 端口是"探测后使用"，存在被抢占的窗口；失败就换一个端口重来。
const START_ATTEMPTS: usize = 3;
/// 服务启动的等待上限，避免线程异常时永久挂起。
const START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// RustFS 的启动不是并发安全的：第一个实例会接管进程级 bootstrap 上下文，
/// 并发的第二个实例同样判定自己是“第一个”，随后写 region 时 panic。
/// 启动是毫秒级操作，串行化对使用没有影响。
static START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 进程内 RustFS 加一层 S3 客户端。
pub struct EmbeddedBlobStore {
    /// 先析构：先放掉客户端，再停服务。
    inner: S3BlobStore,
    /// 后析构：发关闭信号并等待托管线程收尾。
    host: ServerHost,
}

impl EmbeddedBlobStore {
    /// 启动服务、建桶，返回可用的 Blob 后端。
    ///
    /// `volume` 是 RustFS 的数据卷，`bucket`/`prefix` 缺省为 `strata`。
    pub async fn open(volume: PathBuf, bucket: Option<&str>, prefix: Option<&str>) -> Result<Self> {
        tokio::fs::create_dir_all(&volume).await?;
        let host = ServerHost::start(volume).await?;
        let bucket = bucket.unwrap_or(DEFAULT_BUCKET).to_string();
        ensure_bucket(&host.endpoint, &bucket).await?;

        let store = object_store::aws::AmazonS3Builder::new()
            .with_bucket_name(&bucket)
            .with_region(REGION)
            .with_endpoint(&host.endpoint)
            .with_access_key_id(ACCESS_KEY)
            .with_secret_access_key(SECRET_KEY)
            // 嵌入式服务只讲明文 HTTP。
            .with_allow_http(true)
            .build()
            .map_err(|error| Error::ObjectStore(error.to_string()))?;
        let prefix = prefix.unwrap_or(DEFAULT_PREFIX);
        Ok(Self {
            inner: S3BlobStore::new(Arc::new(store), prefix),
            host,
        })
    }

    /// 服务实际监听的 HTTP 端点，例如 `http://127.0.0.1:53124`。
    pub fn endpoint(&self) -> &str {
        &self.host.endpoint
    }
}

/// 托管 RustFS 的专用线程。
struct ServerHost {
    endpoint: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerHost {
    async fn start(volume: PathBuf) -> Result<Self> {
        let _exclusive = START_LOCK.lock().await;
        let (ready, waiting) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("rustfs".into())
            .stack_size(SERVER_STACK)
            .spawn(move || serve(volume, ready))
            .map_err(|error| Error::Embedded(format!("启动线程失败: {error}")))?;
        let started = tokio::time::timeout(START_TIMEOUT, waiting)
            .await
            .map_err(|_| Error::Embedded("启动超时".into()))?;
        match started {
            Ok(Ok((endpoint, stop))) => Ok(Self {
                endpoint,
                stop: Some(stop),
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(Error::Embedded("启动线程提前退出".into())),
        }
    }
}

impl Drop for ServerHost {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 专用线程主体：建独立 runtime，启动服务，等关闭信号。
type Ready = tokio::sync::oneshot::Sender<Result<(String, tokio::sync::oneshot::Sender<()>)>>;

fn serve(volume: PathBuf, ready: Ready) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(SERVER_STACK)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(Error::Embedded(format!("runtime: {error}"))));
            return;
        }
    };
    runtime.block_on(async move {
        let server = match start(&volume).await {
            Ok(server) => server,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let (stop, stopped) = tokio::sync::oneshot::channel();
        if ready.send(Ok((server.endpoint(), stop))).is_err() {
            return;
        }
        let _ = stopped.await;
        server.shutdown().await;
    });
}

/// 起服务。端口被抢占时重新探测。
async fn start(volume: &Path) -> Result<RustFSServer> {
    let volume = volume.to_string_lossy().into_owned();
    let mut last = String::new();
    for _ in 0..START_ATTEMPTS {
        let port = find_available_port()
            .map_err(|error| Error::Embedded(format!("找不到可用端口: {error}")))?;
        let result = RustFSServerBuilder::new()
            .address(format!("127.0.0.1:{port}"))
            .access_key(ACCESS_KEY)
            .secret_key(SECRET_KEY)
            .region(REGION)
            .volume(volume.clone())
            .build()
            .await;
        match result {
            Ok(server) => return Ok(server),
            Err(error) => last = error.to_string(),
        }
    }
    Err(Error::Embedded(format!("启动失败: {last}")))
}

/// 建桶。已存在时当作成功，方便反复启动同一个数据目录。
async fn ensure_bucket(endpoint: &str, bucket: &str) -> Result<()> {
    use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};

    let config = aws_sdk_s3::Config::builder()
        // 关掉了默认 feature，behavior version 要显式给，否则构造请求时会 panic。
        .behavior_version(BehaviorVersion::latest())
        .credentials_provider(Credentials::new(
            ACCESS_KEY,
            SECRET_KEY,
            None,
            None,
            "strata-embedded",
        ))
        .region(Region::new(REGION))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();
    let client = aws_sdk_s3::Client::from_conf(config);
    if client.head_bucket().bucket(bucket).send().await.is_ok() {
        return Ok(());
    }
    client
        .create_bucket()
        .bucket(bucket)
        .send()
        .await
        .map_err(|error| Error::Embedded(format!("建桶 {bucket} 失败: {error}")))?;
    Ok(())
}

#[async_trait::async_trait]
impl BlobStore for EmbeddedBlobStore {
    async fn put(&self, sha: &str, bytes: &[u8]) -> Result<()> {
        self.inner.put(sha, bytes).await
    }

    async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        self.inner.get(sha).await
    }

    async fn range(&self, sha: &str, start: u64, len: u64) -> Result<Vec<u8>> {
        self.inner.range(sha, start, len).await
    }

    async fn contains(&self, sha: &str) -> Result<bool> {
        self.inner.contains(sha).await
    }

    async fn remove(&self, sha: &str) -> Result<()> {
        self.inner.remove(sha).await
    }
}

/// 把配置里的相对数据卷解析成数据目录下的绝对路径。
pub fn volume_path(root: &Path, configured: Option<&PathBuf>) -> PathBuf {
    match configured {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => root.join(path),
        None => root.join("rustfs"),
    }
}
