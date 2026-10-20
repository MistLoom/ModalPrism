use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use console_api::{ConsoleState, router};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<host::HostResources>) {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    for preset in config::builtin_presets() {
        let path = config.path().join("presets").join(preset.file_name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, preset.source).unwrap();
    }
    std::fs::write(
        config.path().join("system.yaml"),
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n",
    )
    .unwrap();
    let services = host::HostResources::open(
        data.path(),
        config.path(),
        host::Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
    )
    .await
    .unwrap();
    (data, config, services)
}

fn app(services: Arc<host::HostResources>) -> axum::Router {
    router(ConsoleState {
        services,
        http_addr: "127.0.0.1:17880".into(),
    })
}

async fn get_json(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .header("host", "127.0.0.1:17880")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
    };
    (status, json)
}

async fn post_json(
    app: axum::Router,
    path: &str,
    body: &str,
    extra: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "127.0.0.1:17880")
        .header("content-type", "application/json");
    for (k, v) in extra {
        builder = builder.header(*k, *v);
    }
    let response = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
    (status, json)
}

async fn put_json(app: axum::Router, path: &str, body: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .header("host", "127.0.0.1:17880")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
    (status, json)
}

#[tokio::test]
async fn meta_ok_and_timeline() {
    let (_d, _c, services) = fixture().await;
    let app = app(services);
    let (status, json) = get_json(app.clone(), "/console/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("configGeneration").is_some());
    let (status, _) = get_json(app, "/console/api/v1/timeline").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn overview_and_sources() {
    let (_d, _c, services) = fixture().await;
    let app = app(services);
    let (status, json) = get_json(app.clone(), "/console/api/v1/overview").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("sourceCount").is_some());
    let (status, json) = get_json(app.clone(), "/console/api/v1/sources").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["items"].as_array().is_some());
    let (status, _) = get_json(app, "/console/api/v1/sources/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn records_empty() {
    let (_d, _c, services) = fixture().await;
    let app = app(services);
    let (status, json) = get_json(app, "/console/api/v1/records?limit=5").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn config_files_whitelist_and_traversal() {
    let (_d, _c, services) = fixture().await;
    let app = app(services);
    let (status, json) = get_json(app.clone(), "/console/api/v1/config/files").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["items"].as_array().unwrap().iter().any(|p| p.as_str().unwrap_or("").starts_with("presets/")));
    assert!(json.get("groups").is_some());
    let (status, _) = get_json(app.clone(), "/console/api/v1/config/files/../system.yaml").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = put_json(
        app.clone(),
        "/console/api/v1/config/files/evil.yaml",
        r#"{"text":"x","confirm":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = put_json(
        app,
        "/console/api/v1/config/files/accessors/api.yaml",
        r#"{"text":"x","confirm":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn config_write_validate_rollback() {
    let (_d, config, services) = fixture().await;
    let app = app(services);
    let (_status, json) = get_json(app.clone(), "/console/api/v1/config/files").await;
    let preset_path = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p.as_str().unwrap_or("").starts_with("presets/"))
        .and_then(|p| p.as_str())
        .unwrap();
    let path = format!("/console/api/v1/config/files/{preset_path}");
    let (status, file) = get_json(app.clone(), &path).await;
    assert_eq!(status, StatusCode::OK);
    let original = file["text"].as_str().unwrap();
    let bad = "::: not yaml";
    let (status, _) = post_json(
        app.clone(),
        "/console/api/v1/config/file-validate",
        &format!(
            r#"{{"path":"{preset_path}","text":{}}}"#,
            serde_json::to_string(bad).unwrap()
        ),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let disk = std::fs::read_to_string(config.path().join(preset_path)).unwrap();
    assert_eq!(disk, original);
    let patched = format!("{}\n# console test\n", original.trim_end());
    let (status, _) = put_json(
        app.clone(),
        &path,
        &format!(
            r#"{{"text":{},"confirm":true}}"#,
            serde_json::to_string(&patched).unwrap()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        app,
        "/console/api/v1/config/file-rollback",
        &format!(r#"{{"path":"{preset_path}","confirm":true}}"#),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let disk = std::fs::read_to_string(config.path().join(preset_path)).unwrap();
    assert_eq!(disk, original);
}

#[tokio::test]
async fn csrf_blocks_bad_origin_and_content_type() {
    let (_d, _c, services) = fixture().await;
    let app = app(services);
    let (status, _) = post_json(
        app.clone(),
        "/console/api/v1/config/validate",
        "{}",
        &[("origin", "http://evil.example")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/console/api/v1/config/validate")
                .header("host", "127.0.0.1:17880")
                .header("content-type", "text/plain")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn governance_and_config_summary() {
    let (_d, _c, services) = fixture().await;
    let app = app(services);
    let (status, _) = get_json(app.clone(), "/console/api/v1/governance/egress").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get_json(app, "/console/api/v1/config/summary").await;
    assert_eq!(status, StatusCode::OK);
}
