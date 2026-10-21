//! 只读进程只安装读取表面，并且不打开 MCP 标准输入。

use std::fs;
use std::path::Path;

use ability_support::{InstanceId, MethodId};
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use modalprism_agent::{boot, mcp_gateway, parse_args};
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
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n",
    );
}

#[test]
fn argv_rejects_unknown_flags() {
    let Err(error) = parse_args(["--http", "127.0.0.1:1"]) else {
        panic!("未知参数应当失败");
    };
    assert_eq!(error.to_string(), "未知参数: --http");
    let Err(error) = parse_args(["--data"]) else {
        panic!("缺少取值应当失败");
    };
    assert_eq!(error.to_string(), "缺少参数值: --data");
    let command = parse_args(["--data", "var", "--config", "config"]).unwrap();
    assert_eq!(command.data.as_os_str(), "var");
    assert_eq!(command.config.as_os_str(), "config");
}

#[tokio::test]
async fn boot_exposes_only_the_read_surface() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = boot(data.path(), config.path()).await.unwrap();
    assert!(data.path().join("control.db").is_file());
    assert!(data.path().join("lance").is_dir());
    assert!(data.path().join("staging").is_dir());

    let mut ids: Vec<_> = running
        .gateway()
        .catalog()
        .into_iter()
        .map(|entry| {
            entry
                .instance
                .as_str()
                .split('.')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids,
        vec![
            "config_inspect",
            "health",
            "strata_read",
            "type_article",
            "type_hotlist",
        ]
    );
    for blocked in [
        "strata_write",
        "strata_queue",
        "committer",
        "planner",
        "runner",
        "accessor_http",
    ] {
        let Err(error) = running
            .gateway()
            .invoke(InvokeRequest::new(
                InstanceId::new(blocked).unwrap(),
                MethodId::new("apply").unwrap(),
                json!({}),
            ))
            .await
        else {
            panic!("{blocked} 应当不可调用");
        };
        let msg = error.to_string();
        assert!(
            msg.contains("实例未开放")
                || msg.contains("not found")
                || msg.contains("未找到")
                || msg.contains("不存在"),
            "{blocked}: {msg}"
        );
    }

    let projected = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("type_article").unwrap(),
            MethodId::new("project").unwrap(),
            json!({ "rid": "r_01ARZ3NDEKTSV4RRFFQ69G5FAV" }),
        ))
        .await
        .unwrap();
    assert!(projected.data.is_null());

    let health = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("health").unwrap(),
            MethodId::new("status").unwrap(),
            json!({ "source": "demo" }),
        ))
        .await
        .unwrap();
    assert_eq!(health.data["status"], "ok");
    assert_eq!(health.data["alert"], false);

    let checked = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("config_inspect").unwrap(),
            MethodId::new("validate").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(checked.data["ok"], true);

    let Err(_) = running
        .host()
        .invoke(InvokeRequest::new(
            InstanceId::new("health").unwrap(),
            MethodId::new("evaluate").unwrap(),
            json!({}),
        ))
        .await
    else {
        panic!("health.evaluate 不在只读表面");
    };
}

#[tokio::test]
async fn mcp_catalog_names_are_instance_method() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = boot(data.path(), config.path()).await.unwrap();
    let tools = mcp_gateway(running.gateway());
    let catalog = tools.catalog();
    assert!(
        catalog
            .iter()
            .any(|entry| entry.instance.as_str() == "strata_read.kinds")
    );
    assert!(
        catalog.iter().all(|entry| !entry.stateful),
        "只读方法的目录项应为无状态，以便 readOnlyHint 为 true"
    );
    let kinds = tools
        .get_entry(
            &InstanceId::new("strata_read.kinds").unwrap(),
            &MethodId::new("kinds").unwrap(),
        )
        .unwrap();
    assert!(!kinds.stateful);
    assert!(
        catalog
            .iter()
            .any(|entry| entry.instance.as_str() == "health.status")
    );
    let mut names: Vec<_> = catalog
        .iter()
        .map(|entry| entry.instance.as_str().to_string())
        .collect();
    let count = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), count);
    assert!(names.iter().all(|name| name.contains('.')));

    let health = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("health.status").unwrap(),
            MethodId::new("status").unwrap(),
            json!({ "source": "demo" }),
        ))
        .await
        .unwrap();
    assert_eq!(health.data["status"], "ok");

    let kinds = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("strata_read.kinds").unwrap(),
            MethodId::new("kinds").unwrap(),
            json!({ "rid": "r_01ARZ3NDEKTSV4RRFFQ69G5FAV" }),
        ))
        .await
        .unwrap();
    assert!(kinds.data.as_array().is_some());
}
