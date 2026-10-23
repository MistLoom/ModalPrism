//! 跨应用契约：同一数据目录下，collector 与 agent 的只读输出逐字段相等。
//!
//! 两侧都链接 `abilitys/` 下的共享能力，差异只允许出现在各自的开放白名单，
//! 不允许出现在方法声明或返回形状上。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use serde_json::{Value, json};
use strata_model::{ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, Target};

/// 两侧都以同一批实例名暴露的共享只读面。`type_*` 只在 agent 侧链接，不在此列。
const SHARED: &[&str] = &["strata_read", "health", "config_inspect"];

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
}

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

async fn invoke(
    gateway: &Arc<dyn Gateway>,
    instance: &str,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    let request = InvokeRequest::new(
        InstanceId::new(instance).unwrap(),
        MethodId::new(method).unwrap(),
        args,
    );
    gateway
        .invoke(request)
        .await
        .map(|response| response.data)
        .map_err(|error| error.to_string())
}

fn declared(gateway: &Arc<dyn Gateway>) -> Vec<(String, String)> {
    let mut rows: Vec<_> = gateway
        .catalog()
        .into_iter()
        .filter(|entry| SHARED.contains(&entry.instance.as_str()))
        .map(|entry| {
            (
                entry.instance.as_str().to_string(),
                entry.method.as_str().to_string(),
            )
        })
        .collect();
    rows.sort();
    rows
}

/// 同一数据目录：两侧都能读到同一份已发布内容，并且返回形状逐字段相等。
#[tokio::test]
async fn shared_read_surfaces_agree_across_apps() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let collector = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();

    let strata = collector.services().strata();
    let rid = publish_fixture(&strata).await;

    let agent = modalprism_agent::boot(data.path(), config.path())
        .await
        .unwrap();

    // 方法声明必须一致：共享能力强制的契约。
    assert_eq!(
        declared(&collector.gateway()),
        declared(&agent.gateway()),
        "collector 与 agent 的共享方法声明不一致"
    );

    let module = invoke(&agent.gateway(), "config_inspect", "modules", json!({}))
        .await
        .unwrap();
    let module_id = module
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("system")
        .to_string();

    let facet = format!("{rid}@struct:page#latest");
    let text = format!("{rid}@text#latest");
    let cases: Vec<(&str, &str, Value)> = vec![
        ("strata_read", "kinds", json!({ "rid": rid })),
        (
            "strata_read",
            "has",
            json!({ "rid": rid, "kind": "struct", "variant": "page" }),
        ),
        ("strata_read", "get", json!({ "facet": facet })),
        (
            "strata_read",
            "slice",
            json!({ "span": format!("{text}[0..6]") }),
        ),
        (
            "strata_read",
            "resolve",
            json!({ "identity": format!("url:{FIXTURE_URL}") }),
        ),
        ("strata_read", "history", json!({ "facet": facet })),
        ("strata_read", "lineage", json!({ "facet": facet })),
        ("strata_read", "related", json!({ "rid": rid })),
        (
            "strata_read",
            "query",
            json!({ "query": { "from": "resources", "select": ["rid"], "limit": 5 } }),
        ),
        ("strata_read", "changes", json!({ "after": 0 })),
        ("health", "status", json!({ "source": "demo" })),
        ("config_inspect", "validate", json!({})),
        ("config_inspect", "diff", json!({})),
        ("config_inspect", "modules", json!({})),
        ("config_inspect", "explain", json!({ "module": module_id })),
    ];

    let mut failures = Vec::new();
    for (instance, method, args) in cases {
        let left = invoke(&collector.gateway(), instance, method, args.clone()).await;
        let right = invoke(&agent.gateway(), instance, method, args).await;
        match (left, right) {
            (Ok(left), Ok(right)) => {
                if left != right {
                    failures.push(format!("{instance}.{method}: {left} != {right}"));
                }
            }
            (Err(left), Err(right)) => {
                if left != right {
                    failures.push(format!("{instance}.{method}: {left} != {right}"));
                }
            }
            (left, right) => failures.push(format!("{instance}.{method}: {left:?} != {right:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "跨应用输出不一致：\n{}",
        failures.join("\n")
    );

    // 读面必须真的读到内容，而不是两侧一起返回空。
    let page = invoke(
        &collector.gateway(),
        "strata_read",
        "get",
        json!({ "facet": facet }),
    )
    .await
    .unwrap();
    assert_eq!(page["struct"]["title"], "示例文章");
    assert_eq!(
        invoke(
            &agent.gateway(),
            "strata_read",
            "slice",
            json!({ "span": format!("{text}[0..6]") })
        )
        .await
        .unwrap(),
        json!("示例")
    );
    let health = invoke(
        &agent.gateway(),
        "health",
        "status",
        json!({ "source": "demo" }),
    )
    .await
    .unwrap();
    assert_eq!(health["key"], "source/demo");
    assert_eq!(health["status"], "ok");
    assert_eq!(health["alert"], false);

    collector.shutdown().await.unwrap();
}

const FIXTURE_URL: &str = "https://example.com/contract";

async fn publish_fixture(strata: &strata_client::Strata) -> String {
    strata
        .apply(
            ChangeSet {
                producer: Producer {
                    module: "contract-test".into(),
                    version: "1".into(),
                    config_generation: 1,
                },
                idempotency_key: None,
                ack: None,
                ops: vec![
                    Op::Ensure {
                        identity: format!("url:{FIXTURE_URL}"),
                        ns: None,
                        labels: BTreeMap::new(),
                        r#as: "article".into(),
                    },
                    Op::PutFacet {
                        target: Target::Alias("article".into()),
                        kind: FacetKind::Struct,
                        variant: Some("page".into()),
                        body: Body::Struct {
                            value: json!({ "title": "示例文章" }),
                        },
                        media_type: Some("application/json".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "params".into(),
                        },
                    },
                    Op::PutFacet {
                        target: Target::Alias("article".into()),
                        kind: FacetKind::Text,
                        variant: None,
                        body: Body::Text {
                            text: "示例文章正文".into(),
                        },
                        media_type: Some("text/plain".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "params".into(),
                        },
                    },
                ],
            },
            ApplyOpts::default(),
        )
        .await
        .unwrap();
    strata.flush().await.unwrap();
    strata
        .resolve(&format!("url:{FIXTURE_URL}"))
        .await
        .unwrap()
        .unwrap()
        .to_string()
}
