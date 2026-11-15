//! 统一摄入函数（`<data_dir>/inbox` 摄入根）与入口面区分的验收测试。
//!
//! 覆盖：路径越界、符号链接逃出、超过大小上限、MCP 面拒绝 `file`/`path`、
//! CLI 入口读取摄入根内合法文件、持久化定位器不含本机绝对路径。

use std::fs;
use std::path::Path;
use std::sync::Arc;

use assembly::{Binds, Services};
use config::builtin_presets;
use serde_json::json;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn config_dir(dir: &Path) {
    for preset in builtin_presets() {
        write(dir, &format!("presets/{}", preset.file_name), preset.source);
    }
    write(
        dir,
        "system.yaml",
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
}

async fn services(data: &Path, config: &Path) -> Arc<Services> {
    Services::open(data, config, binds()).await.unwrap()
}

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

/// 建符号链接；无权限（Windows 普通用户）时返回 false，测试相应跳过断言。
#[cfg(unix)]
fn try_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn try_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[tokio::test]
async fn cli_can_read_a_regular_file_inside_inbox() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    write(&data.path().join("inbox"), "hello.md", "# Hello inbox\n");
    let args = json!({ "file": "hello.md" });
    let request = assembly::tools::OnceRequest::parse_for_surface(&services, &args, true)
        .await
        .unwrap();
    assert_eq!(request.input, b"# Hello inbox\n".to_vec());
    assert_eq!(request.media_type, "text/markdown");
    // 定位器用内容哈希，不携带本机路径。
    let locator = serde_json::to_value(&request.locator).unwrap();
    let url = locator["url"].as_str().unwrap();
    assert!(url.starts_with("content:"), "定位器应为内容哈希: {url}");
    assert!(!url.contains(data.path().to_str().unwrap()));
}

#[tokio::test]
async fn absolute_path_outside_inbox_is_rejected() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    fs::write(&secret, "secret").unwrap();
    let args = json!({ "file": secret.display().to_string() });
    let error = assembly::tools::OnceRequest::parse_for_surface(&services, &args, true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("路径越界"), "实际: {error}");
}

#[tokio::test]
async fn relative_escape_outside_inbox_is_rejected() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    write(data.path(), "outside.txt", "secret");
    let args = json!({ "file": "../outside.txt" });
    let error = assembly::tools::OnceRequest::parse_for_surface(&services, &args, true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("路径越界"), "实际: {error}");
}

#[tokio::test]
async fn symlink_escape_outside_inbox_is_rejected() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    fs::write(&secret, "secret").unwrap();
    if !try_symlink(&secret, &data.path().join("inbox").join("leak.txt")) {
        // 当前环境无符号链接权限，无法构造逃逸场景。
        return;
    }
    let args = json!({ "file": "leak.txt" });
    let error = assembly::tools::OnceRequest::parse_for_surface(&services, &args, true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("路径越界"), "实际: {error}");
}

#[tokio::test]
async fn oversized_file_is_truncated_streaming() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    let body = vec![b'a'; 4 * 1024 * 1024 + 100];
    fs::create_dir_all(data.path().join("inbox")).unwrap();
    fs::write(data.path().join("inbox").join("big.bin"), &body).unwrap();
    let args = json!({ "file": "big.bin" });
    let request = assembly::tools::OnceRequest::parse_for_surface(&services, &args, true)
        .await
        .unwrap();
    assert_eq!(request.input.len(), 4 * 1024 * 1024);
}

#[tokio::test]
async fn non_regular_file_is_rejected() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    let args = json!({ "file": "subdir" });
    fs::create_dir_all(data.path().join("inbox").join("subdir")).unwrap();
    let error = assembly::tools::OnceRequest::parse_for_surface(&services, &args, true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("普通文件"), "实际: {error}");
}

#[tokio::test]
async fn mcp_default_surface_rejects_file_and_path() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    write(&data.path().join("inbox"), "ok.txt", "hello");
    // 默认面（MCP / HTTP）：file 明确拒绝，即使路径合法。
    let error = assembly::tools::OnceRequest::parse(&services, &json!({ "file": "ok.txt" }))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("file"), "实际: {error}");
    // path 参数在任何入口都拒绝。
    for allow_file in [false, true] {
        let error = assembly::tools::OnceRequest::parse_for_surface(
            &services,
            &json!({ "path": "ok.txt" }),
            allow_file,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("path"), "实际: {error}");
    }
}

#[tokio::test]
async fn content_is_decoded_and_identity_is_content_hash() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    let args = json!({
        "content": base64_of("structured payload"),
        "mediaType": "text/plain",
    });
    let request = assembly::tools::OnceRequest::parse_for_surface(&services, &args, false)
        .await
        .unwrap();
    assert_eq!(request.input, b"structured payload".to_vec());
    assert_eq!(request.media_type, "text/plain");
    let locator = serde_json::to_value(&request.locator).unwrap();
    let url = locator["url"].as_str().unwrap();
    assert!(url.starts_with("content:"));
    assert!(!url.contains("payload"));
}

fn base64_of(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[tokio::test]
async fn missing_input_lists_content_not_file() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let services = services(data.path(), config.path()).await;
    let error = assembly::tools::OnceRequest::parse(&services, &json!({}))
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("content"), "实际: {message}");
    assert!(!message.contains("file"), "实际: {message}");
}
