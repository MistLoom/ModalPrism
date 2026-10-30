//! 领域公共 API 契约：错误模型、ETag、幂等、Operation、生产闭环与维护预演。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use serde_json::{Value, json};
use strata_model::{ApplyOpts, Body, ChangeSet, FacetKind, LineageInput, Op, Producer, Target};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

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

async fn wait_http(base: &str) {
    for _ in 0..100 {
        if reqwest::get(format!("{base}/api/v1/system/status"))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("领域 API 未就绪");
}

async fn operation(http: &reqwest::Client, base: &str, id: &str) -> Value {
    for _ in 0..200 {
        let value: Value = http
            .get(format!("{base}/api/v1/operations/{id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if matches!(
            value["status"].as_str().unwrap_or_default(),
            "succeeded" | "partiallySucceeded" | "failed" | "cancelled"
        ) {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("操作 {id} 未结束");
}

async fn accepted(http: &reqwest::Client, base: &str, response: reqwest::Response) -> Value {
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 202, "异步入口必须返回 202: {text}");
    let value: Value = serde_json::from_str(&text).unwrap();
    let id = value["operationId"].as_str().unwrap().to_string();
    operation(http, base, &id).await
}

/// 通过运维面直接建声明，避开 HTTP 层无关细节。
async fn ops_call(
    running: &modalprism_collector::Running,
    instance: &str,
    method: &str,
    args: Value,
) -> Value {
    let request = InvokeRequest::new(
        InstanceId::new(instance).unwrap(),
        MethodId::new(method).unwrap(),
        args,
    );
    running
        .gateway()
        .invoke(request)
        .await
        .map(|response| response.data)
        .unwrap_or_else(|error| panic!("{instance}.{method}: {error}"))
}

#[tokio::test]
async fn domain_api_is_the_only_product_surface() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();

    // 状态与请求关联。
    let response = http
        .get(format!("{base}/api/v1/system/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("x-request-id").is_some());
    let status: Value = response.json().await.unwrap();
    assert_eq!(status["status"], "online");
    assert!(status["generation"].is_number());

    // 未知字段被拒绝，错误正文带 requestId。
    let response = http
        .post(format!("{base}/api/v1/settings/validate"))
        .json(&json!({"changes": {}, "unexpected": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "unknownField");
    assert!(error["error"]["requestId"].is_string());

    // 设置 schema、有效值与 ETag。
    let schema: Value = http
        .get(format!("{base}/api/v1/settings/schema"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["fields"].as_array().unwrap().len() >= 10);
    let response = http
        .get(format!("{base}/api/v1/settings/effective"))
        .send()
        .await
        .unwrap();
    let etag = response.headers()["etag"].to_str().unwrap().to_string();
    let effective: Value = response.json().await.unwrap();
    assert_eq!(effective["generation"], 1);

    // 设置应用：If-Match + Idempotency-Key + Operation。
    let key = "test-settings-apply-1";
    let response = http
        .post(format!("{base}/api/v1/settings/apply"))
        .header("if-match", &etag)
        .header("idempotency-key", key)
        .json(&json!({"changes": {"general.displayName": "Prism Test"}, "reason": "契约测试"}))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");
    let updated: Value = http
        .get(format!("{base}/api/v1/settings/effective"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["generation"], 2);
    assert_eq!(updated["values"]["general.displayName"], "Prism Test");

    // 相同幂等键与相同请求回放，不重复应用。
    let replay = http
        .post(format!("{base}/api/v1/settings/apply"))
        .header("if-match", &etag)
        .header("idempotency-key", key)
        .json(&json!({"changes": {"general.displayName": "Prism Test"}, "reason": "契约测试"}))
        .send()
        .await
        .unwrap();
    assert!(replay.headers().get("idempotent-replayed").is_some());
    let conflict = http
        .post(format!("{base}/api/v1/settings/apply"))
        .header("if-match", &etag)
        .header("idempotency-key", key)
        .json(&json!({"changes": {"general.displayName": "Another"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);
    let error: Value = conflict.json().await.unwrap();
    assert_eq!(error["error"]["code"], "idempotencyConflict");

    // 投递目标：外部接口登记、渠道掩码与计划维度的自动投递。
    let endpoint: Value = http
        .post(format!("{base}/api/v1/delivery/endpoints"))
        .json(&json!({"name": "下游 API", "url": "http://127.0.0.1:9/ingest", "enabled": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let endpoint_id = endpoint["id"].as_str().unwrap().to_string();
    assert_eq!(endpoint["enabled"], json!(true));

    // 渠道地址只回传主机；带掩码更新时保留原地址。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_addr = listener.local_addr().unwrap();
    let (line_tx, line_rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 2048];
        let read = socket.read(&mut buffer).await.unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let _ = line_tx.send(request.lines().next().unwrap_or_default().to_string());
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let hook_url = format!("http://{hook_addr}/secret-hook");
    let masked_url = format!("http://{hook_addr}/***");
    let channel: Value = http
        .post(format!("{base}/api/v1/delivery/channels"))
        .json(&json!({"name": "本地回调", "kind": "webhook", "enabled": true, "settings": {"url": hook_url}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(channel["settings"]["url"], masked_url);
    let channel_id = channel["id"].as_str().unwrap().to_string();
    let listed: Value = http
        .get(format!("{base}/api/v1/delivery/channels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = listed.to_string();
    assert!(!text.contains("secret-hook"), "{text}");
    let channel_etag = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == channel_id)
        .and_then(|item| item["etag"].as_str())
        .unwrap()
        .to_string();
    let updated: Value = http
        .patch(format!("{base}/api/v1/delivery/channels/{channel_id}"))
        .header("if-match", channel_etag)
        .json(&json!({"name": "本地回调 v2", "kind": "webhook", "enabled": true, "settings": {"url": masked_url}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["settings"]["url"], masked_url);
    assert_eq!(updated["name"], json!("本地回调 v2"));

    // 连通检测真的发出请求，掩码没有吞掉真实路径。
    let response = http
        .post(format!("{base}/api/v1/delivery/channels/{channel_id}/test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");
    let line = tokio::time::timeout(Duration::from_secs(5), line_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(line.contains("/secret-hook"), "{line}");

    // 目标目录同时给出外部接口与通知渠道。
    let targets: Value = http
        .get(format!("{base}/api/v1/delivery/targets"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kinds: Vec<&str> = targets["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"channel"), "{targets}");
    assert!(kinds.contains(&"endpoint"), "{targets}");

    // 计划行里声明投递目标：底层落成监听该计划的规则，清空即删除规则。
    // 计划校验会核对目标信源是否存在，先用运维面建一个访问器与信源。
    let templates = ops_call(&running, "catalog_admin", "templates", json!({})).await;
    let accessor = templates
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["slug"] == json!("accessor-http"))
        })
        .cloned()
        .expect("缺少 accessor-http 模板");
    ops_call(
        &running,
        "catalog_admin",
        "create",
        json!({"kind": "accessor", "slug": "demo-http", "doc": accessor["doc"]}),
    )
    .await;
    ops_call(
        &running,
        "catalog_admin",
        "create",
        json!({
            "kind": "source",
            "slug": "demo-feed",
            "doc": {
                "name": "示例信源",
                "access": {"accessor": "accessor/demo-http", "locator": {"url": "https://example.com/feed"}},
                "pipeline": [],
                "schedule": {"interval": "6h"},
                "admission": {}
            }
        }),
    )
    .await;

    let response = http
        .post(format!("{base}/api/v1/collection/plans"))
        .json(&json!({
            "name": "投递计划",
            "targets": ["source/demo-feed"],
            "trigger": {"kind": "recurring", "interval": "2h"},
            "deliveryTargets": [{"kind": "endpoint", "id": endpoint_id}],
            "deliveryEnabled": true,
        }))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let plan: Value = response.json().await.unwrap();
    assert!(status.is_success(), "{status} {plan}");
    let plan_id = plan["id"].as_str().unwrap().to_string();
    assert_eq!(plan["deliveryTargets"][0]["id"], json!(endpoint_id));
    assert_eq!(plan["deliveryTargets"][0]["kind"], json!("endpoint"));
    let rule_id = plan["deliveryRuleId"].as_str().unwrap().to_string();

    let plan_etag = http
        .get(format!("{base}/api/v1/collection/plans/{plan_id}"))
        .send()
        .await
        .unwrap()
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .to_string();
    let cleared: Value = http
        .patch(format!("{base}/api/v1/collection/plans/{plan_id}"))
        .header("if-match", plan_etag)
        .json(&json!({"deliveryTargets": []}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cleared["deliveryTargets"], json!(null));
    let rules: Value = http
        .get(format!("{base}/api/v1/delivery/rules"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !rules["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == json!(rule_id)),
        "{rules}"
    );


    // 批量内容动作同样使用 `:action` 路径。
    let response = http
        .post(format!("{base}/api/v1/content/items/batch"))
        .json(&json!({"ids": ["r_00000000000000000000000000"], "action": "delete"}))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");

    // 按筛选物理删除内容同样走 `:purge` 路径，预演不写入。
    let preview: Value = http
        .post(format!("{base}/api/v1/content/items/purge"))
        .json(&json!({"dryRun": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["dryRun"], true);
    assert!(preview["matched"].as_u64().is_some());

    // 日志与审计能按 requestId 追踪。
    let logs: Value = http
        .get(format!("{base}/api/v1/observability/logs?limit=20"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!logs["items"].as_array().unwrap().is_empty());
    let audit: Value = http
        .get(format!("{base}/api/v1/observability/auditEvents?limit=20"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!audit["items"].as_array().unwrap().is_empty());

    // 维护预演与执行：范围、令牌、ETag 三重确认。
    let preview_response = http
        .post(format!("{base}/api/v1/maintenance/resetPreviews"))
        .json(&json!({"domains": ["logs"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_response.status(), 201);
    let preview_etag = preview_response.headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();
    let preview: Value = preview_response.json().await.unwrap();
    let preview_id = preview["id"].as_str().unwrap().to_string();
    let token = preview["confirmationToken"].as_str().unwrap().to_string();
    let stale = http
        .post(format!("{base}/api/v1/maintenance/resets"))
        .header("if-match", "\"preview-stale\"")
        .json(&json!({"previewId": preview_id, "confirmationToken": token}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 412);
    let response = http
        .post(format!("{base}/api/v1/maintenance/resets"))
        .header("if-match", preview_etag)
        .json(&json!({"previewId": preview_id, "confirmationToken": token}))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn maintenance_content_reset_physically_purges_storage() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();

    let changeset = ChangeSet {
        producer: Producer {
            module: "domain-api-it".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops: vec![
            Op::Ensure {
                identity: "ext:domain-api:reset".into(),
                ns: None,
                labels: Default::default(),
                r#as: "item".into(),
            },
            Op::PutFacet {
                target: Target::Alias("item".into()),
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                body: Body::Struct {
                    value: json!({"title": "待清空"}),
                },
                media_type: Some("application/json".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "seed".into(),
                },
            },
        ],
    };
    running
        .services()
        .strata()
        .apply(changeset, ApplyOpts::default())
        .await
        .unwrap();
    running.services().strata().flush().await.unwrap();
    assert_eq!(
        running.services().strata().summary().await.unwrap().total,
        1
    );

    let preview_response = http
        .post(format!("{base}/api/v1/maintenance/resetPreviews"))
        .json(&json!({"domains": ["content"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_response.status(), 201);
    let preview_etag = preview_response.headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();
    let preview: Value = preview_response.json().await.unwrap();
    let response = http
        .post(format!("{base}/api/v1/maintenance/resets"))
        .header("if-match", preview_etag)
        .json(&json!({
            "previewId": preview["id"],
            "confirmationToken": preview["confirmationToken"],
        }))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");
    assert_eq!(
        running.services().strata().summary().await.unwrap().total,
        0
    );
    let counts = running.services().strata().store_counts().await.unwrap();
    assert_eq!(counts.resources, 0);
    assert_eq!(counts.facets, 0);
    assert_eq!(counts.snapshots, 0);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn factory_reset_rebuilds_settings_and_clears_user_data() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();

    let changeset = ChangeSet {
        producer: Producer {
            module: "domain-api-it".into(),
            version: "1".into(),
            config_generation: 1,
        },
        idempotency_key: None,
        ack: None,
        ops: vec![
            Op::Ensure {
                identity: "ext:domain-api:factory".into(),
                ns: None,
                labels: Default::default(),
                r#as: "item".into(),
            },
            Op::PutFacet {
                target: Target::Alias("item".into()),
                kind: FacetKind::Struct,
                variant: Some("article".into()),
                body: Body::Struct {
                    value: json!({"title": "恢复出厂前"}),
                },
                media_type: Some("application/json".into()),
                lineage: LineageInput {
                    inputs: Vec::new(),
                    params_hash: "seed".into(),
                },
            },
        ],
    };
    running
        .services()
        .strata()
        .apply(changeset, ApplyOpts::default())
        .await
        .unwrap();
    running.services().strata().flush().await.unwrap();
    let channel: Value = http
        .post(format!("{base}/api/v1/delivery/channels"))
        .json(&json!({"name": "出厂前渠道", "kind": "webhook", "enabled": true, "settings": {"url": "http://127.0.0.1:9/hook"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(channel["name"], "出厂前渠道");

    let preview_response = http
        .post(format!("{base}/api/v1/maintenance/factoryResetPreviews"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_response.status(), 201);
    let preview_etag = preview_response.headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();
    let preview: Value = preview_response.json().await.unwrap();
    let response = http
        .post(format!("{base}/api/v1/maintenance/factoryResets"))
        .header("if-match", preview_etag)
        .json(&json!({
            "previewId": preview["id"],
            "confirmationToken": preview["confirmationToken"],
        }))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");
    assert_eq!(
        running.services().strata().summary().await.unwrap().total,
        0
    );
    let channels: Value = http
        .get(format!("{base}/api/v1/delivery/channels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(channels["items"].as_array().unwrap().is_empty());
    let effective: Value = http
        .get(format!("{base}/api/v1/settings/effective"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(effective["generation"], 1);
    assert!(effective["values"].get("general.displayName").is_some());

    running.shutdown().await.unwrap();
}

/// 遗留的 `config/workflows/` 曾经让整次配置校验失败，还会被出厂基线一次次复制回来。
/// 现在它会被退役清理：live 与出厂基线都不再保留，恢复出厂不反弹。
#[tokio::test]
async fn factory_reset_tolerates_retired_config_dirs() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let legacy = "id: workflow/1\nkind: workflow\nname: 遗留\n";
    write(config.path(), "workflows/1.yaml", legacy);
    // 真实的残留就在出厂基线里：先把它造出来，再看恢复出厂会不会又带回来。
    let factory = data.path().join("factory").join("config");
    config_dir(&factory);
    write(&factory, "workflows/1.yaml", legacy);
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();

    // 启动已完成退役（断言在 assembly 的 boot 用例里）；这里模拟旧版本又写回一份遗留文件，
    // 验证预演能提前看到它、执行后确实消失。
    write(config.path(), "workflows/2.yaml", legacy);
    let preview_response = http
        .post(format!("{base}/api/v1/maintenance/factoryResetPreviews"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_response.status(), 201);
    let preview_etag = preview_response.headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();
    let preview: Value = preview_response.json().await.unwrap();
    assert!(preview["config"]["baselineAvailable"].as_bool().unwrap_or(false));
    assert!(
        preview["config"]["declarations"].as_u64().unwrap_or(0) > 0,
        "预演应给出声明规模: {preview}"
    );
    let retired: Vec<&str> = preview["config"]["retiredPaths"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["path"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(retired, vec!["workflows"], "预演应列出待退役目录: {preview}");
    let response = http
        .post(format!("{base}/api/v1/maintenance/factoryResets"))
        .header("if-match", preview_etag)
        .json(&json!({
            "previewId": preview["id"],
            "confirmationToken": preview["confirmationToken"],
        }))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded", "{finished}");

    // 遗留目录既不能留在 live 配置，也不能留在出厂基线里。
    assert!(!config.path().join("workflows").exists(), "恢复出厂后不应再出现遗留目录");
    assert!(
        !data.path().join("factory").join("config").join("workflows").exists(),
        "出厂基线里的遗留目录应已退役"
    );

    let settings = http
        .get(format!("{base}/api/v1/settings/effective"))
        .send()
        .await
        .unwrap();
    assert_eq!(settings.status(), 200, "设置应在恢复出厂后立即可读");
    let status: Value = http
        .get(format!("{base}/api/v1/system/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(status["status"], "maintenance", "{status}");

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_paths_use_the_standard_error_shape() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();
    let response = http
        .get(format!(
            "{base}/api/v1/content/items/r_00000000000000000000000000"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "notFound");
    let map: BTreeMap<String, Value> = serde_json::from_value(body["error"].clone()).unwrap();
    assert!(map.contains_key("requestId"));
    assert!(map.contains_key("retryable"));
    running.shutdown().await.unwrap();
}
