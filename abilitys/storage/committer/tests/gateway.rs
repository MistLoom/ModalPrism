//! 经 AbilityHost Gateway 调用 committer 与 strata_read。

use std::collections::BTreeMap;
use std::sync::Arc;

use ability_committer::provider as committer_provider;
use ability_config::provider as config_provider;
use ability_core::AbilityHost;
use ability_strata_read::provider as strata_read_provider;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use frame_core::{Gateway, InvokeRequest};
use config::builtin_presets;
use serde_json::json;
use tempfile::tempdir;

fn write_config_dir(dir: &std::path::Path) {
    for preset in builtin_presets() {
        let path = dir.join("presets").join(preset.file_name);
        std::fs::create_dir_all(path.parent().unwrap()).expect("目录");
        std::fs::write(path, preset.source).expect("预设");
    }
    std::fs::write(
        dir.join("system.yaml"),
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n",
    )
    .expect("system");
}

#[tokio::test]
async fn strata_read_resolve_via_gateway() {
    let data = tempdir().expect("临时目录");
    let config = json!({
        "dataDir": data.path().to_string_lossy(),
        "system": json!({}),
    });
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");
    host.install_str("committer", committer_provider())
        .await
        .expect("安装");
    host.install_str("strata_read", strata_read_provider())
        .await
        .expect("安装");
    host.install_str("config", config_provider())
        .await
        .expect("安装");
    let config_dir = tempdir().expect("配置");
    write_config_dir(config_dir.path());
    let bind = json!({
        "dataDir": data.path().to_string_lossy(),
        "configDir": config_dir.path().to_string_lossy(),
        "system": json!({}),
    });
    host.load(LoadSpec::new(
        SourceId::new("config").expect("source"),
        InstanceId::new("config").expect("instance"),
    ).with_config(bind.clone()))
        .await
        .expect("加载");
    host.load(LoadSpec {
        source: SourceId::new("committer").expect("source"),
        as_name: InstanceId::new("committer").expect("instance"),
        config: config.clone(),
        bindings: Some(BTreeMap::from([(
            "config".to_string(),
            InstanceId::new("config").expect("instance"),
        )])),
    })
    .await
    .expect("加载");
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
    let config_dir = tempdir().expect("配置目录");
    write_config_dir(config_dir.path());
    let config = json!({
        "dataDir": data.path().to_string_lossy(),
        "configDir": config_dir.path().to_string_lossy(),
        "system": json!({}),
    });
    let strata = json!({
        "dataDir": data.path().to_string_lossy(),
        "system": json!({}),
    });
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");
    host.install_str("config", config_provider())
        .await
        .expect("安装");
    host.install_str("committer", committer_provider())
        .await
        .expect("安装");
    host.load(
        LoadSpec::new(
            SourceId::new("config").expect("source"),
            InstanceId::new("config").expect("instance"),
        )
            .with_config(config),
    )
        .await
        .expect("加载 config");
    let mut bindings = BTreeMap::new();
    bindings.insert(
        "config".to_string(),
        InstanceId::new("config").expect("instance"),
    );
    let source = SourceId::new("committer").expect("source");
    let instance = InstanceId::new("committer").expect("instance");
    host.load(
        LoadSpec {
            source,
            as_name: instance.clone(),
            config: strata,
            bindings: Some(bindings),
        },
    )
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
