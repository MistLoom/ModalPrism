//! 领域公共 API 契约：错误模型、ETag、幂等、Operation、生产闭环与维护预演。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use assembly::Binds;
use config::builtin_presets;
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

    // 生产输入：文本冻结。
    let input_set: Value = http
        .post(format!("{base}/api/v1/production/inputSets"))
        .json(&json!({"name": "测试输入", "inputs": [{"kind": "text", "text": "一条测试素材"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let input_set_id = input_set["id"].as_str().unwrap().to_string();
    assert_eq!(input_set["inputs"][0]["kind"], "text");

    // 工作流与不可变版本。
    let workflow: Value = http
        .post(format!("{base}/api/v1/production/workflows"))
        .json(&json!({"name": "模板直出", "description": "不依赖模型的契约测试"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    let workflows: Value = http
        .get(format!("{base}/api/v1/production/workflows"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workflow_row = workflows["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == workflow_id)
        .unwrap();
    let workflow_etag = workflow_row["etag"].as_str().unwrap().to_string();
    let version: Value = http
        .post(format!(
            "{base}/api/v1/production/workflows/{workflow_id}/versions"
        ))
        .header("if-match", workflow_etag)
        .json(&json!({"steps": [{"id": "直出", "kind": "template", "parameters": {"template": "{{input.text}}"}}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let version_id = version["id"].as_str().unwrap().to_string();

    // 运行与候选。
    let response = http
        .post(format!("{base}/api/v1/production/runs"))
        .json(
            &json!({"inputSetId": input_set_id, "workflowVersionId": version_id, "mode": "manual"}),
        )
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");
    let candidate_id = finished["resultSummary"]["candidateId"]
        .as_str()
        .unwrap()
        .to_string();
    let candidates: Value = http
        .get(format!("{base}/api/v1/production/candidates"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let candidate = candidates["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == candidate_id)
        .unwrap();
    assert_eq!(candidate["status"], "awaitingReview");
    let candidate_etag = candidate["etag"].as_str().unwrap().to_string();

    // 审核 -> 草稿 -> 更新，ETag 保护。
    let review: Value = http
        .post(format!(
            "{base}/api/v1/production/candidates/{candidate_id}/reviews"
        ))
        .header("if-match", &candidate_etag)
        .json(&json!({"decision": "approve", "comment": "可以发布"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(review["decision"], "approve");
    let draft: Value = http
        .post(format!("{base}/api/v1/production/drafts"))
        .json(&json!({"candidateIds": [candidate_id], "title": "测试草稿", "body": "一条测试素材"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let draft_id = draft["id"].as_str().unwrap().to_string();
    let draft_etag = draft["etag"].as_str().unwrap().to_string();
    let response = http
        .patch(format!("{base}/api/v1/production/drafts/{draft_id}"))
        .header("if-match", draft_etag)
        .json(&json!({"candidateIds": [candidate_id], "title": "测试草稿 v2", "body": "更新后的正文"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // 渠道与发布：投递失败必须记在 Delivery，Publication 不能报告成功。
    let channel: Value = http
        .post(format!("{base}/api/v1/publication/channels"))
        .json(&json!({"name": "本地回调", "kind": "webhook", "enabled": true, "settings": {"url": "http://127.0.0.1:9/hook"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = channel["id"].as_str().unwrap().to_string();

    // 渠道地址仅回传主机，带掩码更新时保留原密钥。
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
    let hook_channel: Value = http
        .post(format!("{base}/api/v1/publication/channels"))
        .json(&json!({"name": "掩码渠道", "kind": "webhook", "enabled": true, "settings": {"url": hook_url}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hook_channel["settings"]["url"], masked_url);
    let hook_channel_id = hook_channel["id"].as_str().unwrap().to_string();
    let listed: Value = http
        .get(format!("{base}/api/v1/publication/channels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = listed.to_string();
    assert!(!text.contains("secret-hook"), "{text}");
    let hook_etag = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == hook_channel_id)
        .and_then(|item| item["etag"].as_str())
        .unwrap()
        .to_string();
    let updated: Value = http
        .patch(format!("{base}/api/v1/publication/channels/{hook_channel_id}"))
        .header("if-match", hook_etag)
        .json(&json!({"name": "掩码渠道 v2", "kind": "webhook", "enabled": true, "settings": {"url": masked_url}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["settings"]["url"], masked_url);
    let response = http
        .post(format!("{base}/api/v1/publication/notifications/test"))
        .json(&json!({"channelId": hook_channel_id}))
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

    let response = http
        .post(format!("{base}/api/v1/publications"))
        .json(&json!({"draftId": draft_id, "channelIds": [channel_id]}))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert!(matches!(
        finished["resultSummary"]["status"].as_str().unwrap(),
        "failed" | "partiallyPublished"
    ));
    let deliveries: Value = http
        .get(format!("{base}/api/v1/publication/deliveries"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!deliveries["items"].as_array().unwrap().is_empty());
    let failed_delivery = deliveries["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["status"] == "failed")
        .cloned()
        .unwrap();
    let response = http
        .post(format!(
            "{base}/api/v1/publication/deliveries/{}/retry",
            failed_delivery["id"].as_str().unwrap()
        ))
        .json(&json!({"reason": "契约测试"}))
        .send()
        .await
        .unwrap();
    let finished = accepted(&http, &base, response).await;
    assert_eq!(finished["status"], "succeeded");

    // 定时发布：到点前保持 scheduled，到点后自动执行。
    let scheduled = time::OffsetDateTime::now_utc() + time::Duration::seconds(2);
    let scheduled_text = scheduled
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let response = http
        .post(format!("{base}/api/v1/publications"))
        .json(&json!({"draftId": draft_id, "channelIds": [channel_id], "scheduledAt": scheduled_text}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let value: Value = response.json().await.unwrap();
    let scheduled_op = value["operationId"].as_str().unwrap().to_string();
    let publications: Value = http
        .get(format!("{base}/api/v1/publications?limit=20"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        publications["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["status"] == "scheduled")
    );
    let finished = operation(&http, &base, &scheduled_op).await;
    assert_eq!(finished["status"], "succeeded");

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
        .post(format!("{base}/api/v1/publication/channels"))
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
        .get(format!("{base}/api/v1/publication/channels"))
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
