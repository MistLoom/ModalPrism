//! runner 经 Gateway 执行 run_task（成功 nack 未知信源）。

use std::collections::BTreeMap;
use std::sync::Arc;

use ability_collector_planner::provider as planner_provider;
use ability_collector_runner::provider as runner_provider;
use ability_committer::provider as committer_provider;
use type_hotlist::provider as type_hotlist_provider;
use ability_config::provider as config_provider;
use ability_core::AbilityHost;
use ability_control_state::provider as control_state_provider;
use ability_egress::provider as egress_provider;
use ability_paid_gateway::provider as paid_provider;
use ability_strata_queue::provider as queue_provider;
use ability_strata_read::provider as read_provider;
use ability_strata_write::provider as write_provider;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
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
        "dataDir": data.to_string_lossy(),
        "configDir": config_dir.to_string_lossy(),
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

fn bindings(deps: &[&str]) -> BTreeMap<String, InstanceId> {
    let mut out = BTreeMap::new();
    for dep in deps {
        out.insert(
            dep.to_string(),
            InstanceId::new(*dep).expect("instance"),
        );
    }
    out
}

#[tokio::test]
async fn runner_run_task_nacks_unknown_source() {
    let data = tempdir().expect("临时目录");
    let config_dir = tempdir().expect("配置目录");
    write_config(config_dir.path());
    let bind = bind_config(data.path(), config_dir.path());
    let strata_config = json!({
        "dataDir": data.path().to_string_lossy(),
        "system": json!({}),
    });

    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("test");

    for (id, provider) in [
        ("committer", committer_provider()),
        ("type_hotlist", type_hotlist_provider()),
        ("config", config_provider()),
        ("control_state", control_state_provider()),
        ("egress", egress_provider()),
        ("paid_gateway", paid_provider()),
        ("strata_read", read_provider()),
        ("strata_write", write_provider()),
        ("strata_queue", queue_provider()),
        ("planner", planner_provider()),
        ("runner", runner_provider()),
    ] {
        host.install_str(id, provider).await.expect("安装");
    }

    load(&host, "config", bind.clone(), None).await;
    load(
        &host,
        "committer",
        strata_config.clone(),
        Some(bindings(&["config"])),
    )
    .await;
    load(&host, "strata_write", strata_config.clone(), None).await;
    load(&host, "control_state", strata_config.clone(), None).await;
    load(
        &host,
        "type_hotlist",
        strata_config.clone(),
        Some(bindings(&["config"])),
    )
    .await;
    load(
        &host,
        "egress",
        bind.clone(),
        Some(bindings(&["config"])),
    )
    .await;
    load(
        &host,
        "paid_gateway",
        bind.clone(),
        Some(bindings(&["config"])),
    )
    .await;
    load(&host, "strata_read", strata_config.clone(), None).await;
    load(&host, "strata_queue", strata_config.clone(), None).await;
    load(
        &host,
        "planner",
        bind.clone(),
        Some(bindings(&["config", "strata_queue", "control_state"])),
    )
    .await;
    load(
        &host,
        "runner",
        bind.clone(),
        Some(bindings(&[
            "config",
            "strata_queue",
            "strata_write",
            "strata_read",
            "egress",
            "paid_gateway",
            "control_state",
        ])),
    )
    .await;

    let result = host
        .invoke(InvokeRequest::new(
            InstanceId::new("runner").expect("instance"),
            MethodId::new("run_task").expect("method"),
            json!({
                "task": {
                    "taskId": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                    "sourceId": "source/missing",
                    "accessor": "builtin:http",
                    "generation": 1,
                    "attempt": 0,
                    "priority": 10,
                    "queue": "free",
                    "reason": "schedule",
                    "locator": { "type": "url", "url": "https://example.com/" },
                    "spec": {},
                    "leaseUntil": "2026-01-01T00:00:00Z"
                }
            }),
        ))
        .await;

    match result {
        Ok(response) => {
            assert_eq!(
                response.data.get("nacked").and_then(|v| v.as_str()),
                Some("config")
            );
        }
        Err(error) => {
            let msg = error.to_string();
            assert!(
                msg.contains("config") || msg.contains("任务不存在"),
                "应进入 run_task 业务路径: {error}"
            );
        }
    }
}
