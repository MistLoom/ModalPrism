use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use console_api::{ConsoleState, router};
use modalprism_assembly::{ConsoleGateway, console_expose, install_collector};
use serde_json::json;
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<dyn frame_core::Gateway>) {
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
    std::fs::write(
        config.path().join("sources/demo-rss.yaml"),
        "id: demo-rss\nkind: source\nname: demo\nspec:\n  accessor: builtin:rss\n  schedule: \"*/30 * * * *\"\n  entry: https://example.com/feed.xml\n",
    )
    .unwrap();
    let snapshot = config::validate(config.path(), &config::LoadOptions::from_process()).unwrap();
    let bind = json!({
        "dataDir": data.path(),
        "configDir": config.path(),
        "system": snapshot.system,
        "proxyUrl": "http://127.0.0.1:9",
        "browserProgram": "moli",
        "proxyListen": "127.0.0.1:0",
        "pushListen": "127.0.0.1:0",
    });
    let strata = json!({ "dataDir": data.path(), "system": bind["system"].clone() });
    let host = install_collector(bind, strata).await.unwrap();
    let gateway = ConsoleGateway::new(host, console_expose());
    (data, config, gateway)
}

fn app(gateway: Arc<dyn frame_core::Gateway>) -> axum::Router {
    router(ConsoleState {
        gateway,
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

#[tokio::test]
async fn meta_and_sources_list() {
    let (_data, _config, gateway) = fixture().await;
    let app = app(gateway);
    let (status, body) = get_json(app.clone(), "/console/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("http").is_some());
    let (status, body) = get_json(app, "/console/api/v1/sources").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("items").is_some());
}
