use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use console_api::{ConsoleState, router};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn meta_returns_generation() {
    let (_data, config, services) = fixture().await;
    let app = router(ConsoleState {
        services,
        http_addr: "127.0.0.1:17880".into(),
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/console/api/v1/meta")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json.get("configGeneration").is_some());
    drop(_data);
    drop(config);
}

async fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<assembly::Services>) {
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
    let services = assembly::Services::open(
        data.path(),
        config.path(),
        assembly::Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
    )
    .await
    .unwrap();
    (data, config, services)
}
