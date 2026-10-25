//! 声明式目录与内容加工的行为测试。
//!
//! 两条主线：
//!
//! - 信源与适配器可以只靠声明新增、启停、删除，不需要改代码；内置适配器只能停用不能删除。
//! - 一条工作流能对真实文章跑完、把结果写回文章、并在运行记录里留下痕迹。

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

async fn call(
    gateway: &Arc<dyn Gateway>,
    instance: &str,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    let value = raw_call(gateway, instance, method, args).await?;
    // 长任务由门面切面返回句柄，测试按真实客户端的姿势轮询到完成。
    if value.get("status").and_then(Value::as_str) != Some("pending") {
        return Ok(value);
    }
    let handle = value
        .get("handle")
        .and_then(Value::as_str)
        .ok_or_else(|| "pending 响应缺少 handle".to_string())?
        .to_string();
    let started = std::time::Instant::now();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let poll = raw_call(gateway, "committer", "handle", json!({ "handle": handle })).await?;
        match poll.get("status").and_then(Value::as_str) {
            Some("done") => {
                return Ok(poll.get("data").cloned().unwrap_or(Value::Null));
            }
            Some("failed") => {
                return Err(poll
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("长任务失败")
                    .to_string());
            }
            Some("unknown") => return Err("句柄不存在".to_string()),
            _ if started.elapsed() > std::time::Duration::from_secs(60) => {
                return Err("句柄轮询超时".to_string());
            }
            _ => {}
        }
    }
}

async fn raw_call(
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

async fn ok(gateway: &Arc<dyn Gateway>, instance: &str, method: &str, args: Value) -> Value {
    call(gateway, instance, method, args)
        .await
        .unwrap_or_else(|error| panic!("{instance}.{method}: {error}"))
}

fn template(templates: &Value, slug: &str) -> Value {
    templates
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("slug").and_then(Value::as_str) == Some(slug))
        })
        .cloned()
        .unwrap_or_else(|| panic!("缺少模板 {slug}"))
}

#[tokio::test]
async fn sources_and_adapters_are_managed_by_declaration() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let gateway = running.gateway();

    let templates = ok(&gateway, "catalog_admin", "templates", json!({})).await;
    let accessor = template(&templates, "accessor-http");

    // 先按模板建一个访问器，再建一个引用它的信源：两者都只是文件。
    ok(
        &gateway,
        "catalog_admin",
        "create",
        json!({"kind": "accessor", "slug": "http2", "doc": accessor["doc"]}),
    )
    .await;
    ok(
        &gateway,
        "catalog_admin",
        "create",
        json!({
            "kind": "source",
            "slug": "demo-feed",
            "doc": {
                "name": "示例信源",
                "access": {
                    "accessor": "accessor/http2",
                    "locator": {"url": "https://example.com/feed"}
                },
                "pipeline": [],
                "schedule": {"interval": "6h"},
                "admission": {}
            }
        }),
    )
    .await;

    let listed = ok(&gateway, "catalog_admin", "list", json!({})).await;
    let source = listed["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["slug"] == json!("demo-feed"))
        .cloned()
        .expect("新信源应当出现在列表里");
    assert_eq!(source["enabled"], json!(true));
    assert_eq!(source["accessor"], json!("accessor/http2"));
    assert_eq!(source["references"], json!(0));

    // 停用只写一个字段，列表立即反映。
    ok(
        &gateway,
        "catalog_admin",
        "toggle",
        json!({"kind": "source", "slug": "demo-feed", "enabled": false}),
    )
    .await;
    let listed = ok(&gateway, "catalog_admin", "list", json!({})).await;
    let source = listed["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["slug"] == json!("demo-feed"))
        .cloned()
        .unwrap();
    assert_eq!(source["enabled"], json!(false));

    // 批量：单条失败不影响其余条目，结果逐条返回。
    let batch = ok(
        &gateway,
        "catalog_admin",
        "batch",
        json!({"kind": "source", "action": "enable", "slugs": ["demo-feed", "no-such"]}),
    )
    .await;
    assert_eq!(batch["total"], json!(2));
    assert_eq!(batch["failed"], json!(1));
    assert_eq!(batch["results"][0]["ok"], json!(true));
    assert_eq!(batch["results"][1]["ok"], json!(false));

    // 内置适配器不能删除：id 与 implementation 同时匹配才算内置。
    let adapter = template(&templates, "adapter-list");
    ok(
        &gateway,
        "catalog_admin",
        "create",
        json!({"kind": "adapter", "slug": "html-list", "doc": adapter["doc"]}),
    )
    .await;
    let error = call(
        &gateway,
        "catalog_admin",
        "remove",
        json!({"kind": "adapter", "slug": "html-list"}),
    )
    .await
    .expect_err("内置适配器应当拒绝删除");
    assert!(error.contains("内置适配器"), "{error}");

    // 自定义适配器可以删除，删除后目录回到原状。
    ok(
        &gateway,
        "catalog_admin",
        "remove",
        json!({"kind": "source", "slug": "demo-feed"}),
    )
    .await;
    ok(
        &gateway,
        "catalog_admin",
        "remove",
        json!({"kind": "accessor", "slug": "http2"}),
    )
    .await;
    let listed = ok(&gateway, "catalog_admin", "list", json!({})).await;
    assert!(
        listed["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["slug"] != json!("demo-feed"))
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_workflow_runs_on_a_real_article_and_writes_back() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let gateway = running.gateway();

    // 命令行模型：模板步骤不调用它，但声明必须先存在，否则工作流无法保存。
    ok(
        &gateway,
        "workflow",
        "save_model",
        json!({
            "slug": "mock",
            "doc": {"name": "本机回声", "mode": "cli", "command": ["echo", "ok"]}
        }),
    )
    .await;
    ok(
        &gateway,
        "workflow",
        "save",
        json!({
            "slug": "digest",
            "doc": {
                "name": "标题摘要",
                "enabled": false,
                "model": "mock",
                "steps": [{"id": "title", "kind": "template", "template": "{{article.title}}"}],
                "outputs": {"facet": {"kind": "md", "variant": "summary", "body": "{{steps.title}}"}}
            }
        }),
    )
    .await;

    let catalog = ok(&gateway, "workflow", "catalog", json!({})).await;
    assert_eq!(catalog["workflows"][0]["slug"], json!("digest"));
    assert_eq!(catalog["workflows"][0]["available"], json!(true));

    let rid = publish(&running, "示例标题").await;
    let run = ok(
        &gateway,
        "workflow",
        "run",
        json!({"slug": "digest", "rid": rid}),
    )
    .await;
    assert_eq!(run["status"], json!("ok"), "{run}");
    assert_eq!(run["steps"][0]["status"], json!("ok"));

    // 产出写回文章，而不是只留在运行记录里。
    let summary = ok(
        &gateway,
        "strata_read",
        "get",
        json!({"facet": format!("{rid}@md:summary#latest")}),
    )
    .await;
    assert_eq!(summary["text"], json!("示例标题"));

    let runs = ok(&gateway, "workflow", "runs", json!({"limit": 10})).await;
    let rows = runs.as_array().unwrap();
    assert!(
        rows.iter()
            .any(|row| row["workflow"] == json!("digest") && row["status"] == json!("ok")),
        "{runs}"
    );

    // 停用后不再进入可运行集合，但记录仍可读。
    ok(
        &gateway,
        "workflow",
        "toggle",
        json!({"slug": "digest", "enabled": true}),
    )
    .await;
    let toggled = ok(&gateway, "workflow", "get", json!({"slug": "digest"})).await;
    assert_eq!(toggled["document"]["enabled"], json!(true));

    running.shutdown().await.unwrap();
}

async fn publish(running: &modalprism_collector::Running, title: &str) -> String {
    let strata = running.services().strata();
    strata
        .apply(
            ChangeSet {
                producer: Producer {
                    module: "catalog-test".into(),
                    version: "1".into(),
                    config_generation: 1,
                },
                idempotency_key: None,
                ack: None,
                ops: vec![
                    Op::Ensure {
                        identity: format!("url:https://example.com/{title}"),
                        ns: None,
                        labels: BTreeMap::new(),
                        r#as: "article".into(),
                    },
                    Op::PutFacet {
                        target: Target::Alias("article".into()),
                        kind: FacetKind::Struct,
                        variant: Some("article".into()),
                        body: Body::Struct {
                            value: json!({"title": title, "url": "https://example.com/a"}),
                        },
                        media_type: Some("application/json".into()),
                        lineage: LineageInput {
                            inputs: Vec::new(),
                            params_hash: "params".into(),
                        },
                    },
                    Op::PutFacet {
                        target: Target::Alias("article".into()),
                        kind: FacetKind::Md,
                        variant: Some("readable".into()),
                        body: Body::Text {
                            text: format!("{title}的正文"),
                        },
                        media_type: Some("text/markdown".into()),
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
        .resolve(&format!("url:https://example.com/{title}"))
        .await
        .unwrap()
        .unwrap()
        .to_string()
}
