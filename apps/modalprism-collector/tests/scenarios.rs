//! 场景化集成基线（A1–A5、A8–A10）：向导 / 项目归属 / 前置校验 / 试搜·fetch·parse。
//! 全部走领域 API（`/api/v1`），不碰真网：fetch/parse 打本机假站。

use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

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

struct Boot {
    base: String,
    running: modalprism_collector::Running,
    http: reqwest::Client,
    // 临时目录必须活到测试结束，否则运行中的实例会被抽走文件。
    _data: tempfile::TempDir,
    _config: tempfile::TempDir,
}

async fn boot() -> Boot {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    Boot {
        base,
        running,
        http: reqwest::Client::new(),
        _data: data,
        _config: config,
    }
}

/// 在配置目录里落一个可用信源（走目录能力，与 catalog.rs 同姿势）。
async fn seed_source(running: &modalprism_collector::Running, name: &str) -> String {
    let gateway = running.gateway();
    let request = InvokeRequest::new(
        InstanceId::new("catalog_admin").unwrap(),
        MethodId::new("templates").unwrap(),
        json!({}),
    );
    let templates = gateway.invoke(request).await.unwrap().data;
    let accessor = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["slug"] == json!("accessor-http"))
        .cloned()
        .expect("缺少 accessor-http 模板");
    for (kind, slug, doc) in [
        ("accessor", "probe-http", accessor["doc"].clone()),
        (
            "source",
            "probe-feed",
            json!({
                "name": name,
                "access": {
                    "accessor": "accessor/probe-http",
                    "locator": {"url": "https://example.com/feed"}
                },
                "pipeline": [],
                "schedule": {"interval": "6h"},
                "admission": {}
            }),
        ),
    ] {
        let request = InvokeRequest::new(
            InstanceId::new("catalog_admin").unwrap(),
            MethodId::new("create").unwrap(),
            json!({"kind": kind, "slug": slug, "doc": doc}),
        );
        gateway.invoke(request).await.unwrap();
    }
    "probe-feed".to_string()
}

async fn get_json(http: &reqwest::Client, url: &str) -> (u16, Value) {
    let response = http.get(url).send().await.unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap())
}

async fn post_json(http: &reqwest::Client, url: &str, body: &Value) -> (u16, Value) {
    let response = http.post(url).json(body).send().await.unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap())
}

/// 应用一项设置（走完整 ETag + 幂等键 + Operation 流程）。
async fn apply_setting(boot: &Boot, key: &str, value: Value) {
    let response = boot
        .http
        .get(format!("{}/api/v1/settings/effective", boot.base))
        .send()
        .await
        .unwrap();
    let etag = response.headers()["etag"].to_str().unwrap().to_string();
    let response = boot
        .http
        .post(format!("{}/api/v1/settings/apply", boot.base))
        .header("if-match", &etag)
        .header("idempotency-key", format!("scenario-{key}-{value}"))
        .json(&json!({"changes": {key: value}, "reason": "场景测试"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202, "设置应用应被受理");
    let started: Value = response.json().await.unwrap();
    let id = started["operationId"].as_str().unwrap().to_string();
    for _ in 0..200 {
        let (_, op) = get_json(&boot.http, &format!("{}/api/v1/operations/{id}", boot.base)).await;
        if op["status"] != json!("succeeded") && op["status"] != json!("pending") {}
        if op["status"] == json!("succeeded") {
            // 确认值真的生效，避免“应用成功但读到旧值”的假绿。
            let (_, effective) = get_json(
                &boot.http,
                &format!("{}/api/v1/settings/effective", boot.base),
            )
            .await;
            assert_eq!(
                effective["values"][key], value,
                "设置 {key} 未生效: {effective}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("设置 {key} 应用未结束");
}

/// 本机假站：返回一段固定 HTML，供 fetch/parse 场景使用。
async fn fake_site(marker: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = vec![0u8; 4096];
            let Ok(read) = socket.read(&mut buffer).await else {
                break;
            };
            let _ = read;
            let body = format!("<html><body><h1>{marker}</h1></body></html>");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}/page")
}

// A1：默认项目承接旧数据，未归属信源在默认项目下软接展示。
#[tokio::test]
async fn a1_default_project_soft_binds_legacy_sources() {
    let boot = boot().await;
    let slug = seed_source(&boot.running, "软接信源").await;

    let (status, projects) = get_json(&boot.http, &format!("{}/api/v1/projects", boot.base)).await;
    assert_eq!(status, 200);
    let default_project = projects["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["isDefault"] == json!(true))
        .cloned()
        .expect("默认项目应已播种");
    let project_id = default_project["id"].as_str().unwrap().to_string();

    // 未归属信源：默认项目视角下以 legacy 软接展示。
    let (status, resources) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/resources", boot.base),
    )
    .await;
    assert_eq!(status, 200);
    let legacy = resources["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == json!(slug))
        .cloned()
        .expect("旧信源应出现在默认项目资源清单");
    assert_eq!(legacy["legacy"], json!(true));

    // 显式归属到新项目后，默认项目不再软接它。
    let (status, created) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects", boot.base),
        &json!({"name": "独立项目"}),
    )
    .await;
    assert_eq!(status, 201);
    let other_id = created["id"].as_str().unwrap().to_string();
    let response = boot
        .http
        .post(format!(
            "{}/api/v1/projects/{other_id}/resources",
            boot.base
        ))
        .json(&json!({"kind": "source", "id": slug}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let (_, resources) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/resources", boot.base),
    )
    .await;
    assert!(
        !resources["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == json!(slug)),
        "已被归属的信源不应再以 legacy 展示在默认项目"
    );
    boot.running.shutdown().await.unwrap();
}

// A2：向导 —— 建档、check（缺信源/有信源）、手动完成、跳过、dismiss。
#[tokio::test]
async fn a2_wizard_check_bind_and_dismiss() {
    let boot = boot().await;
    let slug = seed_source(&boot.running, "向导信源").await;
    let (status, created) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects", boot.base),
        &json!({"name": "向导项目"}),
    )
    .await;
    assert_eq!(status, 201);
    let project_id = created["id"].as_str().unwrap().to_string();

    // 初始：无向导文档时返回待开始视图。
    let (status, wizard) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard", boot.base),
    )
    .await;
    assert_eq!(status, 200);

    // 建档：第一步立即完成。
    let (status, wizard) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard", boot.base),
        &json!({}),
    )
    .await;
    assert_eq!(status, 201);
    let step = |value: &Value, id: &str| {
        value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == json!(id))
            .cloned()
            .unwrap()
    };
    assert_eq!(step(&wizard, "create_project")["status"], json!("done"));

    // check：未归属信源 → configure_sources 未满足，且不改状态。
    let (status, check) = post_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/wizard/steps/configure_sources/check",
            boot.base
        ),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(check["met"], json!(false));
    assert_eq!(check["detail"]["reason"], json!("need_source"));
    assert_eq!(
        step(&check["wizard"], "configure_sources")["status"],
        json!("pending")
    );

    // bind：把信源归属到本项目后再 check → 满足并自动置 done。
    let response = boot
        .http
        .post(format!(
            "{}/api/v1/projects/{project_id}/resources",
            boot.base
        ))
        .json(&json!({"kind": "source", "id": slug}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let (status, check) = post_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/wizard/steps/configure_sources/check",
            boot.base
        ),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(check["met"], json!(true));
    assert_eq!(
        step(&check["wizard"], "configure_sources")["status"],
        json!("done")
    );

    // 手动完成与跳过：顺序门闩下按步推进，中间可选步骤逐个跳过。
    let (status, _) = post_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/wizard/steps/scenario_template/complete",
            boot.base
        ),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    for step_id in [
        "verify_collection",
        "model_gate",
        "processing_output",
        "api_surface",
    ] {
        let (status, wizard) = post_json(
            &boot.http,
            &format!(
                "{}/api/v1/projects/{project_id}/wizard/steps/{step_id}/skip",
                boot.base
            ),
            &json!({}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(step(&wizard, step_id)["status"], json!("skipped"));
    }
    let (_, wizard) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard", boot.base),
    )
    .await;
    assert_eq!(step(&wizard, "scenario_template")["status"], json!("done"));
    assert_eq!(step(&wizard, "api_surface")["status"], json!("skipped"));
    assert_eq!(wizard["status"], json!("completed"));

    // dismiss：收口后向导不再打扰。
    let (status, dismissed) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard/dismiss", boot.base),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(dismissed["status"], json!("dismissed"));
    boot.running.shutdown().await.unwrap();
}

// A11：向导顺序门闩 —— 乱序 complete/skip 必须 412，状态不被改动；按序则放行。
#[tokio::test]
async fn a11_wizard_order_gate_rejects_out_of_order() {
    let boot = boot().await;
    let (status, created) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects", boot.base),
        &json!({"name": "乱序项目"}),
    )
    .await;
    assert_eq!(status, 201);
    let project_id = created["id"].as_str().unwrap().to_string();
    let (status, _) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard", boot.base),
        &json!({}),
    )
    .await;
    assert_eq!(status, 201);
    let step = |value: &Value, id: &str| {
        value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == json!(id))
            .cloned()
            .unwrap()
    };

    // 乱序：scenario_template 尚 pending 时跳过 verify_collection → 412 且状态不变。
    let (status, body) = post_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/wizard/steps/verify_collection/skip",
            boot.base
        ),
        &json!({}),
    )
    .await;
    assert_eq!(status, 412);
    assert_eq!(body["error"]["code"], json!("preconditionFailed"));
    let (_, wizard) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard", boot.base),
    )
    .await;
    assert_eq!(
        step(&wizard, "verify_collection")["status"],
        json!("pending")
    );

    // 乱序：直接完成最后一步 → 同样 412。
    let (status, _) = post_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/wizard/steps/api_surface/complete",
            boot.base
        ),
        &json!({}),
    )
    .await;
    assert_eq!(status, 412);

    // 按序：依次完成 create_project（已 done）→ scenario_template → 放行。
    let (status, wizard) = post_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/wizard/steps/scenario_template/complete",
            boot.base
        ),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(step(&wizard, "scenario_template")["status"], json!("done"));
    boot.running.shutdown().await.unwrap();
}

// A3：缺搜索密钥 / 缺模型 → HTTP 412（结构化 precondition，不是裸 409/428）。
#[tokio::test]
async fn a3_missing_search_key_or_model_returns_412() {
    let boot = boot().await;
    let base = &boot.base;

    // 缺搜索密钥：默认引擎切到 bing 但未配引用名 → 试搜 412。
    apply_setting(&boot, "search.defaultEngine", json!("bing")).await;
    let (_, eff) = get_json(&boot.http, &format!("{base}/api/v1/settings/effective")).await;
    let (_, rep) = get_json(
        &boot.http,
        &format!("{base}/api/v1/preconditions?scope=search"),
    )
    .await;
    let (status, error) = post_json(
        &boot.http,
        &format!("{base}/api/v1/search"),
        &json!({"query": "测试"}),
    )
    .await;
    assert_eq!(status, 412, "{error}");
    assert_eq!(error["error"]["code"], json!("preconditionFailed"));
    let field_errors = error["error"]["fieldErrors"].as_array().unwrap();
    assert!(
        field_errors
            .iter()
            .any(|item| item["code"] == json!("engine_key")
                && item["field"] == json!("search.bingKeyRef")),
        "{error}"
    );
    // 前置探测同样报告未就绪，且带“去哪配”。
    let (status, report) = get_json(
        &boot.http,
        &format!("{base}/api/v1/preconditions?scope=search"),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(report["scopes"][0]["met"], json!(false));
    assert_eq!(
        report["scopes"][0]["requirements"][0]["type"],
        json!("engine_key")
    );
    apply_setting(&boot, "search.defaultEngine", json!("ddg")).await;

    // 缺模型：工作流含模型步骤而未配置默认模型 → 建运行即 412。
    let (status, input_set) = post_json(
        &boot.http,
        &format!("{base}/api/v1/production/inputSets"),
        &json!({"name": "412输入", "inputs": [{"kind": "text", "text": "素材"}]}),
    )
    .await;
    assert_eq!(status, 201, "{input_set}");
    let (status, workflow) = post_json(
        &boot.http,
        &format!("{base}/api/v1/production/workflows"),
        &json!({"name": "模型直出", "description": "缺模型前置校验"}),
    )
    .await;
    assert_eq!(status, 201, "{workflow}");
    let workflow_id = workflow["id"].as_str().unwrap().to_string();
    // 工作流 ETag 在列表行里（与既有契约一致）。
    let (status, workflows) =
        get_json(&boot.http, &format!("{base}/api/v1/production/workflows")).await;
    assert_eq!(status, 200, "{workflows}");
    let etag = workflows["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == json!(workflow_id))
        .and_then(|item| item["etag"].as_str())
        .expect("工作流行应带 ETag")
        .to_string();
    let response = boot
        .http
        .post(format!("{base}/api/v1/production/workflows/{workflow_id}/versions"))
        .header("if-match", &etag)
        .json(&json!({"steps": [{"id": "改写", "kind": "model", "parameters": {"prompt": "改写 {{input.text}}"}}]}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let version: Value = response.json().await.unwrap();
    assert_eq!(status, 201, "{version}");
    let version_id = version["id"].as_str().unwrap().to_string();
    let (status, error) = post_json(
        &boot.http,
        &format!("{base}/api/v1/production/runs"),
        &json!({"inputSetId": input_set["id"], "workflowVersionId": version_id, "mode": "manual"}),
    )
    .await;
    assert_eq!(status, 412, "{error}");
    assert_eq!(error["error"]["code"], json!("preconditionFailed"));
    let field_errors = error["error"]["fieldErrors"].as_array().unwrap();
    assert!(
        field_errors
            .iter()
            .any(|item| item["code"] == json!("model")
                && item["field"] == json!("ai.defaultModelId")),
        "{error}"
    );
    boot.running.shutdown().await.unwrap();
}

// A4：付费引擎接通后的行为——
// ① 密钥引用指向环境但环境中无密钥 → 412（缺什么、去哪配）；
// ② 密钥齐备 + 本机假站（端点覆盖）→ 200，结果归一化。
#[tokio::test]
async fn a4_paid_engines_respect_key_gate_and_normalized_hits() {
    // ① serpapi 引用名指向不存在的环境变量 → 412 engine_key。
    let boot_one = boot().await;
    apply_setting(&boot_one, "search.defaultEngine", json!("serpapi")).await;
    apply_setting(
        &boot_one,
        "search.serpApiKeyRef",
        json!("env:MODALPRISM_TEST_NOPE"),
    )
    .await;
    let (status, error) = post_json(
        &boot_one.http,
        &format!("{}/api/v1/search", boot_one.base),
        &json!({"query": "测试"}),
    )
    .await;
    assert_eq!(status, 412, "{error}");
    assert_eq!(error["error"]["code"], json!("preconditionFailed"));
    let field_errors = error["error"]["fieldErrors"].as_array().unwrap();
    assert!(
        field_errors
            .iter()
            .any(|item| item["code"] == json!("engine_key")
                && item["field"] == json!("search.serpApiKeyRef")),
        "{error}"
    );
    boot_one.running.shutdown().await.unwrap();

    // ② bing：value 字面量密钥 + 端点覆盖到本机假站 → 200 归一化结果。
    let body = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        BING_MOCK.len(),
        BING_MOCK
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await;
            let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, body.as_bytes()).await;
        }
    });
    domain_api::search::set_endpoint_override("bing", &format!("http://{addr}/search"));

    let boot_two = boot().await;
    apply_setting(&boot_two, "search.defaultEngine", json!("bing")).await;
    apply_setting(&boot_two, "search.bingKeyRef", json!("value:test-key-1")).await;
    let (status, result) = post_json(
        &boot_two.http,
        &format!("{}/api/v1/search", boot_two.base),
        &json!({"query": "测试"}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["engine"], json!("bing"));
    assert_eq!(result["count"], json!(2));
    let hits = result["results"].as_array().unwrap();
    assert_eq!(hits[0]["title"], json!("Bing 标题"));
    assert_eq!(hits[0]["url"], json!("https://example.com/1"));
    assert_eq!(hits[0]["source"], json!("bing"));
    boot_two.running.shutdown().await.unwrap();
}

const BING_MOCK: &str = r#"{"webPages":{"value":[{"name":"Bing 标题","url":"https://example.com/1","snippet":"摘要一"},{"name":"第二条","url":"https://example.org/2","snippet":"摘要二"}]}}"#;

// A5：fetch 打本机假站，拿到正文。
#[tokio::test]
async fn a5_fetch_reads_local_fake_site() {
    let boot = boot().await;
    let url = fake_site("正文标记").await;
    let (status, body) = post_json(
        &boot.http,
        &format!("{}/api/v1/fetch", boot.base),
        &json!({"url": url}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!(200));
    let text = body["text"].as_str().unwrap();
    assert!(text.contains("正文标记"), "{body}");
    assert_eq!(body["truncated"], json!(false));
    boot.running.shutdown().await.unwrap();
}

// A8：空 query / 未知引擎 → 400 validationFailed。
#[tokio::test]
async fn a8_search_rejects_empty_query_and_unknown_engine() {
    let boot = boot().await;
    let (status, error) = post_json(
        &boot.http,
        &format!("{}/api/v1/search", boot.base),
        &json!({"query": "   "}),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    assert_eq!(error["error"]["code"], json!("validationFailed"));
    let (status, error) = post_json(
        &boot.http,
        &format!("{}/api/v1/search", boot.base),
        &json!({"query": "测试", "engine": "nope"}),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    let field_errors = error["error"]["fieldErrors"].as_array().unwrap();
    assert!(
        field_errors
            .iter()
            .any(|item| item["field"] == json!("engine")),
        "{error}"
    );
    boot.running.shutdown().await.unwrap();
}

// A9：parse 负向 —— 缺输入 400；强制不装 rawprism Guest 时 412。
#[tokio::test]
async fn a9_parse_negative_paths() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot_opts(
        data.path(),
        config.path(),
        binds(),
        modalprism_collector::BootOpts {
            disable_parse_guest: true,
        },
    )
    .await
    .unwrap();
    let base = format!("http://{}", running.http());
    wait_http(&base).await;
    let http = reqwest::Client::new();

    let (status, error) = post_json(&http, &format!("{base}/api/v1/parse"), &json!({})).await;
    assert_eq!(status, 400, "{error}");
    assert_eq!(error["error"]["code"], json!("validationFailed"));

    let url = fake_site("待解析").await;
    let (status, error) =
        post_json(&http, &format!("{base}/api/v1/parse"), &json!({"url": url})).await;
    assert_eq!(status, 412, "{error}");
    assert_eq!(error["error"]["code"], json!("preconditionFailed"));
    let field_errors = error["error"]["fieldErrors"].as_array().unwrap();
    assert!(
        field_errors
            .iter()
            .any(|item| item["code"] == json!("parser")),
        "{error}"
    );
    running.shutdown().await.unwrap();
}

// A9b：parse 正向 —— rawprism Guest 已装配：text 直解与假站 URL fetch→scan 均返回区域。
#[tokio::test]
async fn a9b_parse_positive_with_rawprism_guest() {
    let boot = boot().await;

    // ① text 直解：JSON 内嵌文本应被识别出至少一个区域。
    let (status, result) = post_json(
        &boot.http,
        &format!("{}/api/v1/parse", boot.base),
        &json!({"text": "before {\"a\": 1} after"}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert!(
        result["regions"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "scan 应返回至少一个区域: {result}"
    );

    // ② URL 路径：fetch 本机假站文本再 scan。
    let url = fake_site("待解析").await;
    let (status, result) = post_json(
        &boot.http,
        &format!("{}/api/v1/parse", boot.base),
        &json!({"url": url}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert!(
        result["regions"].as_array().is_some(),
        "URL parse 应返回区域结构: {result}"
    );
    boot.running.shutdown().await.unwrap();
}

// A10：前置探测聚合 —— 无模型/无信源时逐项未就绪，未知 scope 400。
#[tokio::test]
async fn a10_preconditions_report_each_scope() {
    let boot = boot().await;
    let (status, report) =
        get_json(&boot.http, &format!("{}/api/v1/preconditions", boot.base)).await;
    assert_eq!(status, 200);
    assert_eq!(report["met"], json!(false));
    let scopes = report["scopes"].as_array().unwrap();
    let llm = scopes
        .iter()
        .find(|item| item["scope"] == json!("llm"))
        .unwrap();
    assert_eq!(llm["met"], json!(false));
    assert_eq!(
        llm["requirements"][0]["setting"],
        json!("ai.defaultModelId")
    );
    let search = scopes
        .iter()
        .find(|item| item["scope"] == json!("search"))
        .unwrap();
    assert_eq!(search["met"], json!(true), "默认 ddg 无需密钥");
    let collection = scopes
        .iter()
        .find(|item| item["scope"] == json!("collection"))
        .unwrap();
    assert_eq!(collection["met"], json!(false));

    let (status, error) = get_json(
        &boot.http,
        &format!("{}/api/v1/preconditions?scope=nope", boot.base),
    )
    .await;
    assert_eq!(status, 400, "{error}");
    assert_eq!(error["error"]["code"], json!("validationFailed"));
    boot.running.shutdown().await.unwrap();
}

// A12：search projectId 语义 ——
// ① 项目不存在 → 404；② 成功搜索回显 projectId 并落 search_record 绑定；
// ③ 不传 projectId 行为不变（响应无 projectId、不落记录）。
#[tokio::test]
async fn a12_search_project_id_validation_and_binding() {
    let body = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        BING_MOCK.len(),
        BING_MOCK
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await;
            let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, body.as_bytes()).await;
        }
    });
    domain_api::search::set_endpoint_override("bing", &format!("http://{addr}/search"));

    let boot = boot().await;
    apply_setting(&boot, "search.defaultEngine", json!("bing")).await;
    apply_setting(&boot, "search.bingKeyRef", json!("value:test-key-1")).await;

    // ① 不存在的项目 → 404 notFound。
    let (status, error) = post_json(
        &boot.http,
        &format!("{}/api/v1/search", boot.base),
        &json!({"query": "测试", "projectId": "proj_nope"}),
    )
    .await;
    assert_eq!(status, 404, "{error}");
    assert_eq!(error["error"]["code"], json!("notFound"));

    // 建项目后搜索成功：响应回显 projectId。
    let (status, project) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects", boot.base),
        &json!({"name": "搜索项目"}),
    )
    .await;
    assert_eq!(status, 201, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();

    // ② 成功搜索：回显 projectId，资源清单出现 search_record。
    let (status, result) = post_json(
        &boot.http,
        &format!("{}/api/v1/search", boot.base),
        &json!({"query": "测试", "projectId": project_id}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["projectId"], json!(project_id));
    assert_eq!(result["count"], json!(2));
    let (_, resources) = get_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/resources?kind=search_record",
            boot.base
        ),
    )
    .await;
    let records = resources["items"].as_array().unwrap();
    assert_eq!(records.len(), 1, "{resources}");
    assert_eq!(records[0]["kind"], json!("search_record"));

    // ③ 不传 projectId：行为不变，响应无 projectId、不落记录。
    let (status, result) = post_json(
        &boot.http,
        &format!("{}/api/v1/search", boot.base),
        &json!({"query": "测试"}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert!(result.get("projectId").is_none(), "{result}");
    let (_, resources) = get_json(
        &boot.http,
        &format!(
            "{}/api/v1/projects/{project_id}/resources?kind=search_record",
            boot.base
        ),
    )
    .await;
    assert_eq!(resources["items"].as_array().unwrap().len(), 1);
    boot.running.shutdown().await.unwrap();
}
