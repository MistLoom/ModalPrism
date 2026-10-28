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
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
    // 写白名单冒烟用的最小引用物：rss 访问器与 feed 条目适配器。
    write(
        dir,
        "accessors/rss.yaml",
        "id: accessor/rss\nkind: accessor\nname: rss\nimplementation: builtin:rss\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\nspec:\n  pages: 1\n",
    );
    write(
        dir,
        "adapters/feed-items.yaml",
        "id: adapter/feed-items\nkind: adapter\nname: Feed 条目\nimplementation: builtin:feed-items\nrole: search\nconsumes:\n  - {kind: raw, part_role: document}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: exponential\n    base: 1s\n    max: 30s\n",
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
    assert!(data.path().join("gateway.db").is_file());
    // 测试固定用本地目录后端，不下载 RustFS 发行包。
    assert!(data.path().join("blobs").is_dir());
    assert!(data.path().join("staging").is_dir());

    let mut ids: Vec<_> = running
        .host()
        .catalog()
        .into_iter()
        .map(|entry| entry.instance.as_str().to_string())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids,
        vec![
            "catalog_admin",
            "config_inspect",
            "health",
            "strata_read",
            "type_article",
            "type_hotlist",
            "web",
            "workflow",
        ]
    );
    let catalog = running.gateway().catalog();
    assert!(
        catalog
            .iter()
            .all(|entry| ids.iter().any(|id| id == entry.instance.as_str()))
    );
    // 安装了能力不等于整个能力都对外：MCP 面只开只读面 + 写白名单。
    assert!(catalog.iter().all(|entry| {
        surface::exposes(
            surface::AGENT_MCP,
            entry.instance.as_str(),
            entry.method.as_str(),
        ) || surface::mcp_write_allowed(entry.instance.as_str(), entry.method.as_str())
    }));
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
        assert!(error.to_string().contains("实例未开放"));
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
    // 目录能力均声明 stateful:false，readOnlyHint 统一为 true；写不写由面控制，不由该提示控制。
    assert!(
        catalog.iter().all(|entry| !entry.stateful),
        "目录项应为无状态，以便 readOnlyHint 为 true"
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

#[tokio::test]
async fn mcp_face_opens_new_queries_and_refuses_writes() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = boot(data.path(), config.path()).await.unwrap();
    let tools = mcp_gateway(running.gateway());

    let names: Vec<String> = tools
        .catalog()
        .into_iter()
        .map(|entry| entry.instance.as_str().to_string())
        .collect();
    for expected in [
        "catalog_admin.list",
        "catalog_admin.templates",
        "workflow.catalog",
        "workflow.runs",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "MCP 面缺少 {expected}"
        );
    }
    for closed in [
        "workflow.save",
        "workflow.run",
        "workflow.tick",
        "catalog_admin.remove",
        "catalog_admin.proxy",
        "catalog_admin.batch",
    ] {
        assert!(
            names.iter().all(|name| name != closed),
            "MCP 面不应开放 {closed}"
        );
    }

    let listed = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("catalog_admin.list").unwrap(),
            MethodId::new("list").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    assert!(listed.data["sources"].as_array().is_some());
    assert!(listed.data["adapters"].as_array().is_some());

    let Err(_) = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("workflow.save").unwrap(),
            MethodId::new("save").unwrap(),
            json!({ "slug": "daily", "doc": {} }),
        ))
        .await
    else {
        panic!("workflow.save 不在 MCP 面");
    };
}

/// MCP 写白名单：可恢复写（声明文件草稿）可调用，破坏性/设置写有明确拒绝。
#[tokio::test]
async fn mcp_write_whitelist_allows_recoverable_and_refuses_destructive() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = boot(data.path(), config.path()).await.unwrap();
    let tools = mcp_gateway(running.gateway());

    let created = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("catalog_admin.create").unwrap(),
            MethodId::new("create").unwrap(),
            json!({
                "kind": "source",
                "slug": "mcp_smoke",
                "doc": {
                    "id": "source/mcp_smoke",
                    "kind": "source",
                    "name": "MCP 冒烟",
                    "access": {
                        "accessor": "accessor/rss",
                        "locator": { "url": "https://example.com/feed.xml" }
                    },
                    "pipeline": [{ "run": ["adapter/feed-items"] }],
                    "schedule": { "interval": "1h" },
                    "admission": { "initialBackfill": { "limit": 8, "maxAge": "365d" } }
                }
            }),
        ))
        .await
        .expect("写白名单内的 catalog_admin.create 应当可调用");
    assert!(created.data["id"].is_string());

    let Err(_) = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("catalog_admin.remove").unwrap(),
            MethodId::new("remove").unwrap(),
            json!({ "kind": "source", "slug": "mcp_smoke" }),
        ))
        .await
    else {
        panic!("catalog_admin.remove 是破坏性写，MCP 面必须拒绝");
    };

    let Err(_) = tools
        .invoke(InvokeRequest::new(
            InstanceId::new("catalog_admin.proxy").unwrap(),
            MethodId::new("proxy").unwrap(),
            json!({ "value": "http://127.0.0.1:9" }),
        ))
        .await
    else {
        panic!("catalog_admin.proxy 是设置类写，MCP 面必须拒绝");
    };
}
