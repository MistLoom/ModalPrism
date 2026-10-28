use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, SourceId};
use frame_port_fun::FunPort;
use serde_json::json;

async fn boot(root: &std::path::Path) -> FunPort {
    let host = AbilityHost::new();
    host.install_str("store", ability_store::provider())
        .await
        .unwrap();
    host.install_str("store.persist", ability_store_persist::provider())
        .await
        .unwrap();
    host.install_str("task", ability_task::provider())
        .await
        .unwrap();
    host.load(
        LoadSpec::new(
            SourceId::new("store").unwrap(),
            InstanceId::new("store").unwrap(),
        )
        .with_config(json!({
            "data_root": root.join("data").to_string_lossy(),
            "temp_root": root.join("tmp").to_string_lossy(),
            "watch": false
        })),
    )
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("store.persist").unwrap(),
        InstanceId::new("store.persist").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("task").unwrap(),
        InstanceId::new("task").unwrap(),
    ))
    .await
    .unwrap();
    FunPort::new(host.into_gateway())
}

#[tokio::test]
async fn submit_next_parallel_ready() {
    let root = std::env::temp_dir().join(format!(
        "leaf-task-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fun = boot(&root).await;
    fun.invoke_str(
        "task.submit",
        json!({
            "project_id": "p1",
            "tasks": [
                { "id": "a", "name": "A" },
                { "id": "b", "name": "B" },
                { "id": "c", "name": "C", "depends_on": ["a", "b"] }
            ]
        }),
        None,
    )
    .await
    .unwrap();
    let next = fun
        .invoke_str(
            "task.next",
            json!({ "project_id": "p1", "claimed_by": "w1" }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(next.data["empty"], false);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn reject_cycle() {
    let root = std::env::temp_dir().join(format!(
        "leaf-task-cyc-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fun = boot(&root).await;
    let err = fun
        .invoke_str(
            "task.submit",
            json!({
                "project_id": "p2",
                "tasks": [
                    { "id": "a", "name": "A", "depends_on": ["b"] },
                    { "id": "b", "name": "B", "depends_on": ["a"] }
                ]
            }),
            None,
        )
        .await;
    assert!(err.is_err());
    let _ = std::fs::remove_dir_all(root);
}
