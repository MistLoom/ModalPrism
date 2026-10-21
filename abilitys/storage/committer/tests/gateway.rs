//! 经 AbilityHost Gateway 调用 committer 与 strata_read。

use std::sync::Arc;

use ability_committer::provider as committer_provider;
use ability_core::AbilityHost;
use ability_strata_read::provider as strata_read_provider;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use frame_core::{Gateway, InvokeRequest};
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn strata_read_resolve_via_gateway() {
    let data = tempdir().expect("临时目录");
    let config = json!({
        "dataDir": data.path(),
        "system": json!({}),
    });
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");
    host.install_str("strata_read", strata_read_provider())
        .await
        .expect("安装");
    let source = SourceId::new("strata_read").expect("source");
    let instance = InstanceId::new("strata_read").expect("instance");
    host.load(
        LoadSpec::new(source, instance.clone()).with_config(config.clone()),
    )
        .await
        .expect("加载");
    let response = host
        .invoke(InvokeRequest::new(
            instance,
            MethodId::new("resolve").expect("method"),
            json!({ "identity": "missing:key" }),
        ))
        .await
        .expect("调用");
    assert!(response.data.is_null());
}

#[tokio::test]
async fn committer_status_via_gateway() {
    let data = tempdir().expect("临时目录");
    let config = json!({
        "dataDir": data.path(),
        "system": json!({}),
    });
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");
    host.install_str("committer", committer_provider())
        .await
        .expect("安装");
    let source = SourceId::new("committer").expect("source");
    let instance = InstanceId::new("committer").expect("instance");
    host.load(LoadSpec::new(source, instance.clone()).with_config(config))
        .await
        .expect("加载");
    let response = host
        .invoke(InvokeRequest::new(
            instance,
            MethodId::new("status").expect("method"),
            json!({}),
        ))
        .await
        .expect("调用");
    assert!(response.data.get("control").is_some());
}
