//! runner 经 Gateway 委托 planner.run_task。

use std::collections::BTreeMap;
use std::sync::Arc;

use ability_collector_planner::provider as planner_provider;
use ability_collector_runner::provider as runner_provider;
use ability_config::provider as config_provider;
use ability_core::AbilityHost;
use ability_paid_gateway::provider as paid_provider;
use ability_strata_queue::provider as queue_provider;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use frame_core::{Gateway, InvokeRequest};
use config::builtin_presets;
use serde_json::json;
use tempfile::tempdir;

fn write_config(dir: &std::path::Path) {
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

fn bind_config(data: &std::path::Path, config_dir: &std::path::Path) -> serde_json::Value {
    json!({
        "dataDir": data,
        "configDir": config_dir,
        "system": json!({}),
        "proxyUrl": "http://127.0.0.1:9",
        "browserProgram": "moli",
    })
}

async fn load(
    host: &AbilityHost,
    id: &str,
    config: serde_json::Value,
    bindings: Option<BTreeMap<String, InstanceId>>,
) {
    let source = SourceId::new(id).expect("source");
    let instance = InstanceId::new(id).expect("instance");
    host.load(
        LoadSpec {
            source,
            as_name: instance,
            config,
            bindings,
        },
    )
        .await
        .expect("加载");
}

#[tokio::test]
async fn runner_delegates_run_task_to_planner() {
    let data = tempdir().expect("临时目录");
    let config_dir = tempdir().expect("配置目录");
    write_config(config_dir.path());
    let bind = bind_config(data.path(), config_dir.path());
    let strata_config = json!({
        "dataDir": data.path(),
        "system": json!({}),
    });

    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");

    for (id, provider) in [
        ("config", config_provider()),
        ("paid_gateway", paid_provider()),
        ("strata_queue", queue_provider()),
        ("planner", planner_provider()),
        ("runner", runner_provider()),
    ] {
        host.install_str(id, provider).await.expect("安装");
    }

    load(&host, "config", bind.clone(), None).await;
    load(&host, "paid_gateway", bind.clone(), None).await;
    load(&host, "strata_queue", strata_config, None).await;

    let mut planner_bindings = BTreeMap::new();
    for dep in ["config", "paid_gateway", "strata_queue"] {
        planner_bindings.insert(
            dep.to_string(),
            InstanceId::new(dep).expect("instance"),
        );
    }
    load(&host, "planner", bind.clone(), Some(planner_bindings)).await;

    let mut runner_bindings = BTreeMap::new();
    runner_bindings.insert(
        "planner".to_string(),
        InstanceId::new("planner").expect("instance"),
    );
    load(&host, "runner", json!({}), Some(runner_bindings)).await;

    let runner_instance = InstanceId::new("runner").expect("instance");
    let result = host
        .invoke(InvokeRequest::new(
            runner_instance,
            MethodId::new("run_task").expect("method"),
            json!({
                "task": {
                    "taskId": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                    "sourceId": "missing-source",
                    "accessor": "http",
                    "generation": 0,
                    "attempt": 0,
                    "queue": "default",
                    "reason": "schedule",
                    "locator": { "url": "https://example.com" },
                    "spec": {},
                    "leaseUntil": "2026-01-01T00:00:00Z",
                }
            }),
        ))
        .await;

    match result {
        Ok(response) => {
            assert!(
                response.data.get("nacked").is_some() || response.data.get("ok").is_some()
            );
        }
        Err(error) => {
            assert!(
                error.to_string().contains("planner"),
                "runner 应经 ctx 调用 planner: {error}"
            );
        }
    }
}
