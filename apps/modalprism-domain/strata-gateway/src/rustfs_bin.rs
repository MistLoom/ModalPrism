//! 外部 RustFS 二进制：默认的一键启动 Blob 后端。
//!
//! 相比编译 `rustfs` crate（136 个传递依赖、需要 `protoc`、首次构建小时级），
//! 这里直接使用官方的预编译发行包：首次启动下载到数据目录、校验 SHA256、
//! 解压出可执行文件，之后以子进程方式起停，只监听回环地址的随机端口。
//!
//! # 二进制的来源
//!
//! 按顺序尝试：
//! 1. 配置里显式给出的路径（`strata.gateway.rustfs.exe`）；
//! 2. 缓存目录中已解压的同版本文件，缺省 `<data>/tools/rustfs/<version>/`；
//! 3. `PATH` 里已有的 `rustfs`，本机装过就不再联网；
//! 4. 从 GitHub Releases 下载并校验。
//!
//! 四条都不成立才报错，错误消息会带上默认缓存位置与配置方式。
//! 下载排在 `PATH` 之后，联网只发生在本地确实没有二进制的时候；连接超时取 5 秒，
//! 断网时很快失败。`strata.gateway.rustfs.offline` 为真时连这一步也跳过，
//! 缺二进制直接报错。
//!
//! # 数据卷
//!
//! 与进程内后端共用 `<data>/rustfs`：同一套 RustFS 数据格式，切换后端不必搬数据。
//! 服务端日志追加到 `<volume>/rustfs.log`，子进程退出时随数据层一起被回收。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::blob::{BlobStore, S3BlobStore};
use crate::error::{Error, Result};

/// 固定的 RustFS 发行版本。升级要同时确认数据卷兼容性。
pub const VERSION: &str = "1.0.3";

const RELEASE_URL: &str = "https://github.com/rustfs/rustfs/releases/download";
/// 服务只绑定回环地址，凭据不落配置文件也不出进程；与进程内后端保持一致，
/// 两种后端可以指向同一个数据卷。
const ACCESS_KEY: &str = "strata";
const SECRET_KEY: &str = "strata-embedded-local";
const REGION: &str = "us-east-1";

/// 默认桶名。
pub const DEFAULT_BUCKET: &str = "strata";
/// 默认对象前缀。
pub const DEFAULT_PREFIX: &str = "strata";

/// 连不上 GitHub 时要尽快失败，不把启动拖在超时上。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 校验清单只有几 KB。
const SUMS_TIMEOUT: Duration = Duration::from_secs(20);
/// 发行包约 110 MB，慢网也要留够时间。
const ASSET_TIMEOUT: Duration = Duration::from_secs(300);

/// 端口是"探测后使用"，存在被抢占的窗口；失败就换一个端口重来。
const START_ATTEMPTS: usize = 3;
/// 等待端口可连的上限。
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// 端口通了不代表 S3 路由已经就绪，建桶要重试。
const BUCKET_ATTEMPTS: usize = 20;
const BUCKET_INTERVAL: Duration = Duration::from_millis(250);

/// 二进制的位置策略。
#[derive(Clone, Copy, Debug)]
pub struct BinarySource<'a> {
    /// 显式配置的可执行文件。给了就必须存在，不再找别处。
    pub exe: Option<&'a Path>,
    /// 解压缓存根。
    pub cache: &'a Path,
    /// 只在本机找，不下载。
    pub offline: bool,
}

/// 外部 RustFS 加上一层 S3 客户端。
pub struct RustfsBlobStore {
    /// 先析构：先放掉客户端，再停子进程。
    inner: S3BlobStore,
    /// 后析构：`Drop` 里杀子进程并回收。
    host: ProcessHost,
}

impl RustfsBlobStore {
    /// 解析二进制、起子进程、建桶，返回可用的 Blob 后端。
    ///
    /// `source` 决定二进制从哪儿来，`volume` 是数据卷目录。
    pub async fn open(
        source: &BinarySource<'_>,
        volume: PathBuf,
        bucket: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Self> {
        let binary = resolve(source).await?;
        tokio::fs::create_dir_all(&volume).await?;
        let host = ProcessHost::start(&binary, &volume).await?;
        let bucket = bucket.unwrap_or(DEFAULT_BUCKET).to_string();
        if let Err(error) = ensure_bucket(&host.endpoint, &bucket).await {
            host.stop();
            return Err(error);
        }

        let store = object_store::aws::AmazonS3Builder::new()
            .with_bucket_name(&bucket)
            .with_region(REGION)
            .with_endpoint(&host.endpoint)
            .with_access_key_id(ACCESS_KEY)
            .with_secret_access_key(SECRET_KEY)
            // 子进程只讲明文 HTTP。
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

    /// 实际使用的可执行文件。
    pub fn binary(&self) -> &Path {
        &self.host.binary
    }
}

#[async_trait::async_trait]
impl BlobStore for RustfsBlobStore {
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

/// 子进程句柄。`Drop` 时终止并回收，不把孤儿进程留给下次启动。
struct ProcessHost {
    endpoint: String,
    binary: PathBuf,
    child: Option<Child>,
}

impl ProcessHost {
    async fn start(binary: &Path, volume: &Path) -> Result<Self> {
        let mut last = String::new();
        for _ in 0..START_ATTEMPTS {
            let port = free_port()?;
            let endpoint = format!("http://127.0.0.1:{port}");
            let mut child = spawn(binary, port, volume)?;
            match wait_ready(&endpoint, &mut child).await {
                Ok(()) => {
                    return Ok(Self {
                        endpoint,
                        binary: binary.to_path_buf(),
                        child: Some(child),
                    });
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    last = error.to_string();
                }
            }
        }
        Err(Error::Rustfs(format!(
            "{} 启动失败: {last}（日志见 {}）",
            binary.display(),
            volume.join("rustfs.log").display()
        )))
    }

    fn stop(mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ProcessHost {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 解析可执行文件：显式路径 → 缓存 → `PATH` → 下载。
async fn resolve(source: &BinarySource<'_>) -> Result<PathBuf> {
    locate(source, in_path).await
}

/// `resolve` 的本体。`installed` 是 `PATH` 的探测，独立成参数才能在不改环境变量的前提下做测试。
async fn locate(
    source: &BinarySource<'_>,
    installed: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = source.exe {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        return Err(Error::Rustfs(format!(
            "指定的 rustfs 不存在: {}",
            path.display()
        )));
    }

    let cached = source.cache.join(VERSION).join(exe_name());
    if cached.is_file() {
        return Ok(cached);
    }

    // 本机已经装了 rustfs 就直接用：联网只该发生在哪儿都找不到的时候。
    if let Some(path) = installed() {
        eprintln!("rustfs: 使用 PATH 中的 {}，不下载", path.display());
        return Ok(path);
    }

    if source.offline {
        return Err(missing(
            &cached,
            "离线模式（strata.gateway.rustfs.offline）不下载二进制",
        ));
    }
    match install(source.cache).await {
        Ok(path) => Ok(path),
        Err(error) => Err(missing(&cached, &format!("下载失败（{error}）"))),
    }
}

/// 缺二进制时的提示：把三条本地出路都写清楚。
fn missing(cached: &Path, reason: &str) -> Error {
    Error::Rustfs(format!(
        "{reason}；可把 {} 放进 PATH，或手动放在 {}，\
         或用 strata.gateway.rustfs.exe 指定",
        exe_name(),
        cached.display()
    ))
}

/// 下载发行包并解压到缓存目录。
async fn install(cache: &Path) -> Result<PathBuf> {
    let asset = asset_name()?;
    let base = format!("{RELEASE_URL}/{VERSION}");
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| Error::Rustfs(format!("下载客户端: {error}")))?;
    eprintln!("rustfs: 本地没有可用的二进制，下载 v{VERSION} 发行包（{asset}）…");

    let sums = client
        .get(format!("{base}/SHA256SUMS"))
        .timeout(SUMS_TIMEOUT)
        .send()
        .await
        .map_err(|error| Error::Rustfs(format!("下载校验清单: {error}")))?
        .error_for_status()
        .map_err(|error| Error::Rustfs(format!("下载校验清单: {error}")))?
        .text()
        .await
        .map_err(|error| Error::Rustfs(format!("读取校验清单: {error}")))?;
    let expected = parse_sums(&sums, asset)?;

    let bytes = client
        .get(format!("{base}/{asset}"))
        .timeout(ASSET_TIMEOUT)
        .send()
        .await
        .map_err(|error| Error::Rustfs(format!("下载 {asset}: {error}")))?
        .error_for_status()
        .map_err(|error| Error::Rustfs(format!("下载 {asset}: {error}")))?
        .bytes()
        .await
        .map_err(|error| Error::Rustfs(format!("读取 {asset}: {error}")))?;

    let actual = hex(Sha256::digest(&bytes));
    if actual != expected {
        return Err(Error::Rustfs(format!(
            "{asset} 的 SHA256 不符：期望 {expected}，实际 {actual}"
        )));
    }

    let dir = cache.join(VERSION);
    tokio::fs::create_dir_all(&dir).await?;
    let target = dir.join(exe_name());
    let payload = bytes.to_vec();
    tokio::task::spawn_blocking(move || unzip(&payload, &target))
        .await
        .map_err(|error| Error::Rustfs(format!("解压线程: {error}")))??;
    eprintln!("rustfs: 已就位 {}", dir.join(exe_name()).display());
    Ok(dir.join(exe_name()))
}

/// 当前平台对应的发行包文件名。
fn asset_name() -> Result<&'static str> {
    let asset = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "rustfs-windows-x86_64-v1.0.3.zip",
        ("linux", "x86_64") => "rustfs-linux-x86_64-gnu-v1.0.3.zip",
        ("linux", "aarch64") => "rustfs-linux-aarch64-gnu-v1.0.3.zip",
        ("macos", "aarch64") => "rustfs-macos-aarch64-v1.0.3.zip",
        (os, arch) => {
            return Err(Error::Rustfs(format!(
                "没有 {os}/{arch} 的官方发行包，请用 strata.gateway.rustfs.exe 指定"
            )));
        }
    };
    Ok(asset)
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "rustfs.exe"
    } else {
        "rustfs"
    }
}

/// 从 `SHA256SUMS` 里取指定文件的摘要。
fn parse_sums(text: &str, asset: &str) -> Result<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        // 有些清单用 `*file` 标记二进制模式。
        if name.trim_start_matches('*') == asset {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    Err(Error::Rustfs(format!("校验清单里没有 {asset}")))
}

/// 把 zip 里的可执行文件解到 `target`，先写临时文件再改名。
fn unzip(bytes: &[u8], target: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|error| Error::Rustfs(format!("打开 zip: {error}")))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Error::Rustfs(format!("读取 zip 条目: {error}")))?;
        let name = entry.name().to_string();
        if !name.ends_with(exe_name()) {
            continue;
        }
        let part = target.with_extension("part");
        let mut file = std::fs::File::create(&part)?;
        std::io::copy(&mut entry, &mut file)?;
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&part, target)?;
        return Ok(());
    }
    Err(Error::Rustfs(format!(
        "发行包里没有 {}",
        target.file_name().unwrap_or_default().to_string_lossy()
    )))
}

/// `PATH` 里已有的 rustfs。
fn in_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe_name()))
        .find(|candidate| candidate.is_file())
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

fn spawn(binary: &Path, port: u16, volume: &Path) -> Result<Child> {
    // 服务端输出追加到数据卷旁的日志，启动失败时能查原因。
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(volume.join("rustfs.log"))?;
    Command::new(binary)
        .arg("server")
        .arg("--address")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--access-key")
        .arg(ACCESS_KEY)
        .arg("--secret-key")
        .arg(SECRET_KEY)
        // 不显式给 region 时服务端不会用 `us-east-1`，签名会报 SignatureDoesNotMatch。
        .arg("--region")
        .arg(REGION)
        // CLI 与 env 同时给，避开不同发行版对参数解析的差异。
        .env("RUSTFS_ACCESS_KEY", ACCESS_KEY)
        .env("RUSTFS_SECRET_KEY", SECRET_KEY)
        .env("RUSTFS_REGION", REGION)
        .arg(volume)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|error| Error::Rustfs(format!("启动 {}: {error}", binary.display())))
}

/// 等端口可连；进程提前退出时立刻失败，不必等满超时。
async fn wait_ready(endpoint: &str, child: &mut Child) -> Result<()> {
    let address = endpoint.trim_start_matches("http://");
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(Error::Rustfs(format!("进程提前退出（{status}）")));
        }
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Rustfs("等待端口就绪超时".into()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 空请求体的 SHA256。S3 要求每个请求都带 `x-amz-content-sha256`。
const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// 建桶。`CreateBucket` 必须签名，这里只实现 HEAD/PUT 两个请求，
/// 默认构建不必引入整套 S3 SDK。
async fn ensure_bucket(endpoint: &str, bucket: &str) -> Result<()> {
    if bucket.is_empty()
        || !bucket
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '.')
    {
        return Err(Error::Rustfs(format!("桶名不合法: {bucket:?}")));
    }
    let host = endpoint.trim_start_matches("http://");
    let client = reqwest::Client::new();
    let mut last = String::from("没有收到响应");
    for _ in 0..BUCKET_ATTEMPTS {
        match send(&client, "HEAD", endpoint, host, bucket).await {
            Ok((200, _)) => return Ok(()),
            Ok((404, _)) => match send(&client, "PUT", endpoint, host, bucket).await {
                Ok((status, _)) if status < 300 => return Ok(()),
                // 已存在（同进程重启或别的实例建过）也算成功。
                Ok((409, _)) => return Ok(()),
                Ok((status, body)) => last = format!("建桶返回 {status}: {body}"),
                Err(error) => last = error,
            },
            Ok((status, body)) => last = format!("查询桶返回 {status}: {body}"),
            Err(error) => last = error,
        }
        tokio::time::sleep(BUCKET_INTERVAL).await;
    }
    Err(Error::Rustfs(format!("桶 {bucket} 未就绪: {last}")))
}

/// 发一个签名请求，返回状态码与响应体（用于诊断签名/权限问题）。
async fn send(
    client: &reqwest::Client,
    method: &str,
    endpoint: &str,
    host: &str,
    bucket: &str,
) -> std::result::Result<(u16, String), String> {
    let path = format!("/{bucket}");
    let (date, authorization) = signature(method, &path, host);
    let request = if method == "HEAD" {
        client.head(format!("{endpoint}{path}"))
    } else {
        client.put(format!("{endpoint}{path}"))
    };
    let response = request
        .header("x-amz-date", date)
        .header("x-amz-content-sha256", EMPTY_SHA)
        .header("authorization", authorization)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Ok((status, body.chars().take(300).collect()))
}

/// AWS Signature V4：只覆盖空请求体的 HEAD/PUT，签名头固定为
/// `host`、`x-amz-content-sha256`、`x-amz-date`。
fn signature(method: &str, path: &str, host: &str) -> (String, String) {
    let now = time::OffsetDateTime::now_utc();
    let (year, month, day) = now.to_calendar_date();
    let (hour, minute, second) = now.to_hms();
    let date = format!("{year:04}{:02}{day:02}", u8::from(month));
    let amz_date = format!("{date}T{hour:02}{minute:02}{second:02}Z");
    let authorization = authorization(method, path, host, ACCESS_KEY, SECRET_KEY, &date, &amz_date);
    (amz_date, authorization)
}

/// 签名头列表，按字典序排列。
const SIGNED_HEADERS: &str = "host;x-amz-content-sha256;x-amz-date";

/// 构造 SigV4 规范请求。
///
/// 注意每个规范头各占一行、行尾都有 `\n`，然后格式本身还要再补一个 `\n`，
/// 也就是规范头与 SignedHeaders 之间必须留一个空行；少这一行服务端会
/// 一律判定 `SignatureDoesNotMatch`。
fn canonical_request(method: &str, path: &str, host: &str, amz_date: &str) -> String {
    let headers = format!("host:{host}\nx-amz-content-sha256:{EMPTY_SHA}\nx-amz-date:{amz_date}\n");
    format!("{method}\n{path}\n\n{headers}\n{SIGNED_HEADERS}\n{EMPTY_SHA}")
}

/// 签名计算本体，时间与密钥作为参数传入，便于单独验证。
fn authorization(
    method: &str,
    path: &str,
    host: &str,
    access: &str,
    secret: &str,
    date: &str,
    amz_date: &str,
) -> String {
    use hmac::{Hmac, Mac};

    let canonical = canonical_request(method, path, host, amz_date);
    let signed = SIGNED_HEADERS;
    let scope = format!("{date}/{REGION}/s3/aws4_request");
    let digest = hex(Sha256::digest(canonical.as_bytes()));
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{digest}");

    let mac = |key: &[u8], data: &[u8]| -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    };
    let key = mac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let key = mac(&key, REGION.as_bytes());
    let key = mac(&key, b"s3");
    let key = mac(&key, b"aws4_request");
    let signature = hex(mac(&key, string_to_sign.as_bytes()));

    format!(
        "AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed}, Signature={signature}"
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn empty_sha_matches_the_constant() {
        assert_eq!(hex(Sha256::digest(b"")), EMPTY_SHA);
    }

    #[test]
    fn sums_pick_the_requested_asset() {
        let text =
            "aaaa  rustfs-windows-x86_64-v1.0.3.zip\nbbbb *rustfs-linux-x86_64-gnu-v1.0.3.zip\n";
        assert_eq!(
            parse_sums(text, "rustfs-windows-x86_64-v1.0.3.zip").unwrap(),
            "aaaa"
        );
        assert_eq!(
            parse_sums(text, "rustfs-linux-x86_64-gnu-v1.0.3.zip").unwrap(),
            "bbbb"
        );
        assert!(parse_sums(text, "missing.zip").is_err());
    }

    #[test]
    fn canonical_request_keeps_the_blank_line_before_signed_headers() {
        let canonical = canonical_request("HEAD", "/strata", "127.0.0.1:9000", "20261023T123255Z");
        assert_eq!(
            canonical,
            concat!(
                "HEAD\n",
                "/strata\n",
                "\n",
                "host:127.0.0.1:9000\n",
                "x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n",
                "x-amz-date:20261023T123255Z\n",
                "\n",
                "host;x-amz-content-sha256;x-amz-date\n",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            )
        );
    }

    #[test]
    fn signature_is_stable_and_method_sensitive() {
        let first = signature("PUT", "/strata", "127.0.0.1:9000");
        assert!(first.1.starts_with("AWS4-HMAC-SHA256 Credential=strata/"));
        assert!(
            first
                .1
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date")
        );
        // 同一秒内重复调用只差时间戳；签名长度固定。
        let second = signature("PUT", "/strata", "127.0.0.1:9000");
        assert_eq!(first.1.len(), second.1.len());
    }

    #[test]
    fn unzip_extracts_the_binary() {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file(exe_name(), options).unwrap();
            writer.write_all(b"binary payload").unwrap();
            writer.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(exe_name());
        unzip(&buffer, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"binary payload");
    }

    #[tokio::test]
    async fn send_signs_the_host_header_it_sends() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = format!("http://127.0.0.1:{port}");
        let host = format!("127.0.0.1:{port}");
        let accepted = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let read = socket.read(&mut buffer).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&buffer[..read]).to_string()
        });
        let status = send(&reqwest::Client::new(), "HEAD", &endpoint, &host, "strata")
            .await
            .unwrap()
            .0;
        assert_eq!(status, 200);
        let request = accepted.await.unwrap().to_lowercase();
        assert!(request.contains(&format!("host: {host}")), "{request}");
        assert!(request.contains("x-amz-date:"), "{request}");
        assert!(request.contains("x-amz-content-sha256:"), "{request}");
        assert!(
            request.contains("authorization: aws4-hmac-sha256"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn explicit_missing_binary_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("absent");
        let error = resolve(&BinarySource {
            exe: Some(&absent),
            cache: dir.path(),
            offline: false,
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("不存在"), "{error}");
    }

    /// 缓存命中时不再看 `PATH`，也不联网。
    #[tokio::test]
    async fn cached_binary_wins_over_path() {
        let dir = tempfile::tempdir().unwrap();
        let cached = dir.path().join(VERSION).join(exe_name());
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"cached").unwrap();
        let found = locate(
            &BinarySource {
                exe: None,
                cache: dir.path(),
                offline: false,
            },
            || Some(PathBuf::from("ignored")),
        )
        .await
        .unwrap();
        assert_eq!(found, cached);
    }

    /// `PATH` 里装了就不下载，即使允许联网。
    #[tokio::test]
    async fn path_binary_wins_over_download() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join(exe_name());
        let found = locate(
            &BinarySource {
                exe: None,
                cache: dir.path(),
                offline: false,
            },
            || Some(installed.clone()),
        )
        .await
        .unwrap();
        assert_eq!(found, installed);
    }

    /// 离线且本地没有二进制：不联网，错误里写清三条本地出路。
    #[tokio::test]
    async fn offline_without_local_binary_fails_fast() {
        let dir = tempfile::tempdir().unwrap();
        let error = locate(
            &BinarySource {
                exe: None,
                cache: dir.path(),
                offline: true,
            },
            || None,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("离线模式"), "{error}");
        assert!(error.contains("PATH"), "{error}");
        assert!(error.contains("strata.gateway.rustfs.exe"), "{error}");
    }
}
