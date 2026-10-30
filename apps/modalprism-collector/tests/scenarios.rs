//! 场景化集成基线（A1–A5、A8–A10）：向导 / 项目归属 / 前置校验 / 试搜·fetch·parse。
//! 全部走领域 API（`/api/v1`），不碰真网：fetch/parse 打本机假站。

use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::InvokeRequest;
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
    // 内置访问器需在启动时就存在（访问器为重启门控：运行时新建不会进入 applied）。
    write(
        dir,
        "accessors/http.yaml",
        "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    // API 建信源走 rss + feed-items：整库校验要求二者作为模块存在，且访问器为重启门控，须在启动时落盘。
    write(
        dir,
        "accessors/rss.yaml",
        "id: accessor/rss\nkind: accessor\nname: rss\nimplementation: builtin:rss\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    write(
        dir,
        "adapters/feed-items.yaml",
        "id: adapter/feed-items\nkind: adapter\nname: Feed 条目\nimplementation: builtin:feed-items\nrole: search\nconsumes:\n  - {kind: raw, part_role: document}\nproduces: [discovery]\nlimits:\n  timeout: 30s\n  concurrency: 1\n  retry:\n    attempts: 0\n    backoff: exponential\n    base: 1s\n    max: 30s\n",
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
                    "accessor": "accessor/http",
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

// A1：项目硬隔离——信源归属创建项目，他项目默认不可见；标记共享后跨项目只读可见。
#[tokio::test]
async fn a1_project_isolation_and_sharing() {
    let boot = boot().await;
    // seed_source 经 ability 建信源，正文无 project 标注 → 视为归属默认项目。
    let slug = seed_source(&boot.running, "隔离信源").await;

    let (status, projects) = get_json(&boot.http, &format!("{}/api/v1/projects", boot.base)).await;
    assert_eq!(status, 200);
    let default_id = projects["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["isDefault"] == json!(true))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 新建一个私有项目。
    let (status, created) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects", boot.base),
        &json!({"name": "私有项目"}),
    )
    .await;
    assert_eq!(status, 201);
    let other_id = created["id"].as_str().unwrap().to_string();

    // 列出某项目可见的信源 id（信源列表由后端按 x-project-id 过滤）。
    let visible = |project: String| {
        let http = boot.http.clone();
        let base = boot.base.clone();
        async move {
            let response = http
                .get(format!("{base}/api/v1/catalog/sources?limit=200"))
                .header("x-project-id", &project)
                .send()
                .await
                .unwrap();
            let body: Value = response.json().await.unwrap();
            body["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
        }
    };

    // 默认项目看得见自己的信源；私有项目看不见（硬隔离）。
    assert!(
        visible(default_id.clone()).await.contains(&slug),
        "默认项目应看到归属它的信源"
    );
    assert!(
        !visible(other_id.clone()).await.contains(&slug),
        "私有项目不应看到未共享的信源"
    );

    // 私有项目资源清单也不含该信源（按 project/shared 派生）。
    let (_, resources) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{other_id}/resources?kind=source", boot.base),
    )
    .await;
    assert!(
        !resources["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == json!(slug)),
        "未共享信源不应出现在私有项目清单"
    );

    // 共享可见性：在默认项目下新建一个共享信源（创建即写入 project/shared）。
    let response = boot
        .http
        .post(format!("{}/api/v1/catalog/sources", boot.base))
        .header("x-project-id", &default_id)
        .header("idempotency-key", "a1-shared-src")
        .json(&json!({
            "name": "共享信源",
            "accessMethodId": "rss",
            "adapterIds": ["feed-items"],
            "enabled": true,
            "endpoint": "https://example.com/shared",
            "schedule": {"mode": "fixed", "intervalMs": 3600000},
            "shared": true,
        }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let created_src: Value = response.json().await.unwrap();
    assert_eq!(status, 201, "创建共享信源应成功: {created_src}");
    assert_eq!(created_src["shared"], json!(true));
    assert_eq!(created_src["project"], json!(default_id));
    let shared_slug = created_src["id"].as_str().unwrap().to_string();
    let shared_etag = created_src["etag"].as_str().unwrap().to_string();

    // 私有项目可见该共享信源，且带 shared 标记。
    let response = boot
        .http
        .get(format!("{}/api/v1/catalog/sources?limit=200", boot.base))
        .header("x-project-id", &other_id)
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    let shared = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == json!(shared_slug))
        .cloned()
        .expect("私有项目应看到被共享的信源");
    assert_eq!(shared["shared"], json!(true));

    // 跨项目对共享信源只读：私有项目删除 → 404。
    let response = boot
        .http
        .delete(format!(
            "{}/api/v1/catalog/sources/{shared_slug}",
            boot.base
        ))
        .header("x-project-id", &other_id)
        .header("if-match", &shared_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404, "他项目对共享信源只读，删除应 404");

    // 计划同样按项目隔离：默认项目的计划不出现在私有项目，且跨项目不可改、不可引用。
    let response = boot
        .http
        .post(format!("{}/api/v1/collection/plans", boot.base))
        .header("x-project-id", &default_id)
        .header("idempotency-key", "a1-plan-isolation")
        .json(&json!({
            "name": "隔离计划",
            "targets": [format!("source/{slug}")],
            "trigger": {"kind": "recurring", "interval": "1h"}
        }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let plan: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "默认项目建计划应成功: {plan}");
    assert_eq!(plan["project"], json!(default_id));
    let plan_id = plan["id"].as_str().unwrap().to_string();
    let plan_etag = plan["etag"].as_str().unwrap().to_string();

    let response = boot
        .http
        .get(format!("{}/api/v1/collection/plans?limit=200", boot.base))
        .header("x-project-id", &other_id)
        .send()
        .await
        .unwrap();
    let plans: Value = response.json().await.unwrap();
    assert!(
        !plans["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == json!(plan_id)),
        "他项目不应看到默认项目的计划: {plans}"
    );

    let response = boot
        .http
        .patch(format!("{}/api/v1/collection/plans/{plan_id}", boot.base))
        .header("x-project-id", &other_id)
        .header("if-match", &plan_etag)
        .json(&json!({"name": "越权改名"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404, "他项目改计划应 404");

    let response = boot
        .http
        .post(format!("{}/api/v1/collection/plans", boot.base))
        .header("x-project-id", &other_id)
        .header("idempotency-key", "a1-plan-cross-project")
        .json(&json!({
            "name": "越权计划",
            "targets": [format!("source/{slug}")],
            "trigger": {"kind": "recurring", "interval": "1h"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        400,
        "他项目不能把别家信源写进自己的计划"
    );

    // 归属项目（默认）仍可删除该共享信源。
    let response = boot
        .http
        .delete(format!(
            "{}/api/v1/catalog/sources/{shared_slug}",
            boot.base
        ))
        .header("x-project-id", &default_id)
        .header("if-match", &shared_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202, "归属项目删除共享信源应受理");
    boot.running.shutdown().await.unwrap();
}

// A2：起步清单为只读、实时计算；写端点已移除，不再阻塞任何操作。
#[tokio::test]
async fn a2_wizard_is_readonly_and_computed() {
    let boot = boot().await;
    let (status, created) = post_json(
        &boot.http,
        &format!("{}/api/v1/projects", boot.base),
        &json!({"name": "向导项目"}),
    )
    .await;
    assert_eq!(status, 201);
    let project_id = created["id"].as_str().unwrap().to_string();

    // GET 只读清单：建项目环节已完成，新项目尚无归属信源 → 配置信源为待办。
    let (status, wizard) = get_json(
        &boot.http,
        &format!("{}/api/v1/projects/{project_id}/wizard", boot.base),
    )
    .await;
    assert_eq!(status, 200);
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
    assert_eq!(step(&wizard, "create_project")["met"], json!(true));
    assert_eq!(
        step(&wizard, "configure_sources")["status"],
        json!("pending")
    );
    assert!(!step(&wizard, "configure_sources")["met"].as_bool().unwrap());
    // 未满足环节给出「去哪儿做」。
    assert!(!step(&wizard, "configure_sources")["hint"].as_str().unwrap().is_empty());

    // 写端点已移除：建档/校验/完成/跳过/收口全部 404。
    for (method, path) in [
        ("POST", format!("/api/v1/projects/{project_id}/wizard")),
        (
            "POST",
            format!("/api/v1/projects/{project_id}/wizard/steps/configure_sources/check"),
        ),
        (
            "POST",
            format!("/api/v1/projects/{project_id}/wizard/dismiss"),
        ),
    ] {
        let request = match method {
            "GET" => boot.http.get(format!("{}{path}", boot.base)),
            _ => boot.http.post(format!("{}{path}", boot.base)).json(&json!({})),
        };
        let status = request.send().await.unwrap().status().as_u16();
        assert!(
            status == 404 || status == 405,
            "{method} {path} 应已下线（实际 {status}）"
        );
    }
    boot.running.shutdown().await.unwrap();
}


// A3：缺搜索密钥 → HTTP 412（结构化 precondition，不是裸 409/428）；缺模型则在就绪检查里报出。
#[tokio::test]
async fn a3_missing_search_key_returns_412() {
    let boot = boot().await;
    let base = &boot.base;

    // 缺搜索密钥：默认引擎切到 bing 但未配引用名 → 试搜 412。
    apply_setting(&boot, "search.defaultEngine", json!("bing")).await;
    // 端点可达性预热（结果不参与断言）。
    let _ = get_json(&boot.http, &format!("{base}/api/v1/settings/effective")).await;
    let _ = get_json(
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

    // 缺模型：llm 就绪项报告未配置，并给出配置入口。
    let (status, report) =
        get_json(&boot.http, &format!("{base}/api/v1/preconditions?scope=llm")).await;
    assert_eq!(status, 200);
    assert_eq!(report["scopes"][0]["met"], json!(false));
    assert_eq!(
        report["scopes"][0]["requirements"][0]["type"],
        json!("model")
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

// A9b：parse 正向 —— 依赖 rawprism 仓颉 Guest。Host 的 RuntimeKind 已支持 Cangjie，装载需
// 现场构建(cjpm)或预构建产物；环境缺二者时 boot 的 install_parse_guest 返回 false，
// /system/status.capabilities.parse=false，此时按前置不满足 skip（负向契约见 a9），避免环境噪声。
#[tokio::test]
async fn a9b_parse_positive_with_rawprism_guest() {
    let boot = boot().await;

    // 就绪探针：rawprism 未装载则 skip，而不是断言失败（框架支持 ≠ 本机已构建产物）。
    let (probe_status, sys) =
        get_json(&boot.http, &format!("{}/api/v1/system/status", boot.base)).await;
    assert_eq!(probe_status, 200, "{sys}");
    if sys["capabilities"]["parse"].as_bool() != Some(true) {
        eprintln!("skip a9b: rawprism 仓颉 Guest 未装载（缺 cjpm 工具链或预构建产物）");
        boot.running.shutdown().await.unwrap();
        return;
    }

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

// P1：采集计划一等实体的 CRUD + pause/resume，以及批量作业即时派发。
#[tokio::test]
async fn p1_collection_plan_crud_and_job() {
    let boot = boot().await;
    let slug = seed_source(&boot.running, "计划信源").await;

    let (status, plan) = post_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans", boot.base),
        &json!({
            "name": "计划信源 · 采集计划",
            "targets": [format!("source/{slug}")],
            "trigger": {"kind": "recurring", "interval": "1h"},
            "quota": {"perRunItems": 10, "totalItems": 100}
        }),
    )
    .await;
    assert_eq!(status, 200, "{plan}");
    let plan_id = plan["id"].as_str().unwrap().to_string();
    let etag = plan["etag"].as_str().unwrap().to_string();
    assert_eq!(plan["status"], json!("active"), "{plan}");
    assert_eq!(plan["targets"].as_array().unwrap().len(), 1, "{plan}");

    let (status, list) = get_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans?limit=200", boot.base),
    )
    .await;
    assert_eq!(status, 200, "{list}");
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == json!(plan_id)),
        "{list}"
    );

    let response = boot
        .http
        .post(format!(
            "{}/api/v1/collection/plans/{plan_id}/pause",
            boot.base
        ))
        .header("if-match", &etag)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let paused: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{paused}");
    assert_eq!(paused["status"], json!("paused"), "{paused}");

    let resume_etag = paused["etag"].as_str().unwrap().to_string();
    let response = boot
        .http
        .post(format!(
            "{}/api/v1/collection/plans/{plan_id}/resume",
            boot.base
        ))
        .header("if-match", &resume_etag)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let resumed: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{resumed}");
    assert_eq!(resumed["status"], json!("active"), "{resumed}");

    let (status, job) = post_json(
        &boot.http,
        &format!("{}/api/v1/collection/jobs", boot.base),
        &json!({"targets": [format!("source/{slug}")], "name": "一次性作业"}),
    )
    .await;
    assert_eq!(status, 200, "{job}");
    assert_eq!(job["failed"].as_array().unwrap().len(), 0, "{job}");
    assert_eq!(job["dispatched"].as_array().unwrap().len(), 1, "{job}");

    let response = boot
        .http
        .delete(format!("{}/api/v1/collection/plans/{plan_id}", boot.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "删除计划应返回 200");
    let (_, list) = get_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans?limit=200", boot.base),
    )
    .await;
    assert!(
        !list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == json!(plan_id)),
        "{list}"
    );

    boot.running.shutdown().await.unwrap();
}

// P2：计划的额度与停止策略随实体读写；PATCH 整份替换 quota/stopPolicy（清空即移除）。
#[tokio::test]
async fn p2_plan_quota_and_edit() {
    let boot = boot().await;
    let slug = seed_source(&boot.running, "额度信源").await;

    let (status, plan) = post_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans", boot.base),
        &json!({
            "name": "额度计划",
            "targets": [format!("source/{slug}")],
            "trigger": {"kind": "recurring", "interval": "1h"},
            "quota": {"perRunItems": 5, "totalItems": 50, "maxAge": "30d"},
            "stopPolicy": {"onBudgetExhausted": "pause", "onFailureThreshold": 3}
        }),
    )
    .await;
    assert_eq!(status, 200, "{plan}");
    let plan_id = plan["id"].as_str().unwrap().to_string();
    let etag = plan["etag"].as_str().unwrap().to_string();
    assert_eq!(plan["quota"]["totalItems"], json!(50), "{plan}");
    assert_eq!(plan["quota"]["perRunItems"], json!(5), "{plan}");
    assert_eq!(plan["quota"]["maxAge"], json!("30d"), "{plan}");
    assert_eq!(
        plan["stopPolicy"]["onBudgetExhausted"],
        json!("pause"),
        "{plan}"
    );
    assert_eq!(plan["stopPolicy"]["onFailureThreshold"], json!(3), "{plan}");

    // 编辑：改间隔、整份替换 quota（去掉 perRunItems/maxAge）、stopPolicy 改 skip、改名。
    let response = boot
        .http
        .patch(format!("{}/api/v1/collection/plans/{plan_id}", boot.base))
        .header("if-match", &etag)
        .json(&json!({
            "name": "额度计划·改",
            "trigger": {"kind": "recurring", "interval": "2h"},
            "quota": {"totalItems": 80},
            "stopPolicy": {"onBudgetExhausted": "skip"}
        }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let edited: Value = response.json().await.unwrap();
    assert_eq!(status, 200, "{edited}");
    assert_eq!(edited["name"], json!("额度计划·改"), "{edited}");
    assert_eq!(edited["trigger"]["interval"], json!("2h"), "{edited}");
    assert_eq!(edited["quota"]["totalItems"], json!(80), "{edited}");
    assert!(
        edited["quota"].get("perRunItems").is_none(),
        "整份替换应移除 perRunItems: {edited}"
    );
    assert!(
        edited["quota"].get("maxAge").is_none(),
        "整份替换应移除 maxAge: {edited}"
    );
    assert_eq!(
        edited["stopPolicy"]["onBudgetExhausted"],
        json!("skip"),
        "{edited}"
    );
    assert!(
        edited["stopPolicy"].get("onFailureThreshold").is_none(),
        "应移除 onFailureThreshold: {edited}"
    );

    boot.running.shutdown().await.unwrap();
}

// P3：页码式分页契约——列表响应带 total、offset 与游标同义可跳转、越界页返回空页、非法 offset 拒绝。
#[tokio::test]
async fn p3_list_offset_and_total() {
    let boot = boot().await;
    let slug = seed_source(&boot.running, "分页信源").await;
    for name in ["分页计划A", "分页计划B", "分页计划C"] {
        let (status, created) = post_json(
            &boot.http,
            &format!("{}/api/v1/collection/plans", boot.base),
            &json!({
                "name": name,
                "targets": [format!("source/{slug}")],
                "trigger": {"kind": "recurring", "interval": "1h"}
            }),
        )
        .await;
        assert_eq!(status, 200, "{created}");
    }

    // 全量基准：total 应等于全量条数。
    let (status, all) = get_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans?limit=200", boot.base),
    )
    .await;
    assert_eq!(status, 200, "{all}");
    let total = all["total"].as_u64().expect("plans 应返回 total");
    assert_eq!(
        total as usize,
        all["items"].as_array().unwrap().len(),
        "{all}"
    );
    assert!(total >= 3, "至少三个计划: {all}");

    // offset 翻页与全量序列一致（第 2 页首条 == 全量第 2 条）。
    let (_, page2) = get_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans?limit=1&offset=1", boot.base),
    )
    .await;
    assert_eq!(
        page2["items"][0]["id"], all["items"][1]["id"],
        "offset 页应与全量切片一致: {page2}"
    );
    assert_eq!(page2["total"], json!(total), "{page2}");

    // 越界页：空 items、total 不变。
    let (_, beyond) = get_json(
        &boot.http,
        &format!(
            "{}/api/v1/collection/plans?limit=1&offset={}",
            boot.base, total
        ),
    )
    .await;
    assert!(beyond["items"].as_array().unwrap().is_empty(), "{beyond}");

    // 非法 offset → 400。
    let (status, body) = get_json(
        &boot.http,
        &format!("{}/api/v1/collection/plans?limit=1&offset=abc", boot.base),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    // 其它列表口径：sources / content / logs 均须返回数值 total。
    for path in [
        "/api/v1/catalog/sources?limit=1",
        "/api/v1/content/items?limit=1",
        "/api/v1/observability/logs?limit=1",
    ] {
        let (status, page) = get_json(&boot.http, &format!("{}{path}", boot.base)).await;
        assert_eq!(status, 200, "{page}");
        assert!(page["total"].is_number(), "{path} 应返回数值 total: {page}");
    }

    boot.running.shutdown().await.unwrap();
}

// M7：模型密钥值只写不读、PATCH 编辑、连通测试与 CLI 发现的契约。
#[tokio::test]
async fn m7_model_secret_and_probe() {
    let boot = boot().await;

    // 登记内置密钥模型：投影标记来源，但绝不回传值。
    let (status, created) = post_json(
        &boot.http,
        &format!("{}/api/v1/settings/models", boot.base),
        &json!({
            "name": "内置密钥模型",
            "kind": "remoteApi",
            "endpoint": "http://127.0.0.1:9/v1",
            "secretValue": "sk-test-inline",
            "enabled": true,
            "model": "gpt-6-astra"
        }),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().unwrap_or_default().to_string();
    assert_eq!(created["secretInline"], json!(true), "应标记存在内置密钥");
    assert_eq!(created["secretPresent"], json!(true));
    assert!(created.get("secretValue").is_none(), "投影不得回传密钥值");

    // 列表同样不泄露密钥值。
    let (status, listed) = get_json(
        &boot.http,
        &format!("{}/api/v1/settings/models?limit=200", boot.base),
    )
    .await;
    assert_eq!(status, 200, "{listed}");
    assert!(
        !listed.to_string().contains("sk-test-inline"),
        "列表泄露了密钥值"
    );

    // 含控制字符的密钥被拒绝。
    let (status, rejected) = post_json(
        &boot.http,
        &format!("{}/api/v1/settings/models", boot.base),
        &json!({"name": "坏密钥", "kind": "remoteApi", "endpoint": "http://127.0.0.1:9/v1", "secretValue": "sk\nx", "enabled": true}),
    )
    .await;
    assert_eq!(status, 400, "{rejected}");

    // PATCH：更新端点并清除内置密钥（secretValue 空串）。
    let patched = boot
        .http
        .patch(format!("{}/api/v1/settings/models/{id}", boot.base))
        .json(&json!({"endpoint": "http://127.0.0.1:9/v1", "secretValue": ""}))
        .send()
        .await
        .unwrap();
    let patch_status = patched.status().as_u16();
    let patched: Value = patched.json().await.unwrap();
    assert_eq!(patch_status, 200, "{patched}");
    assert_eq!(patched["secretInline"], json!(false), "空串应清除内置密钥");

    // 连通测试（草稿配置、不可达端点）：结构化 503，而非崩溃。
    let (status, failed) = post_json(
        &boot.http,
        &format!("{}/api/v1/settings/models/test", boot.base),
        &json!({"kind": "remoteApi", "endpoint": "http://127.0.0.1:9/v1", "model": "x", "secretValue": "sk"}),
    )
    .await;
    assert_eq!(status, 503, "{failed}");
    assert_eq!(
        failed["error"]["code"].as_str(),
        Some("dependencyUnavailable")
    );

    // 连通测试（CLI 草稿、不存在的命令）：同样给出可解释失败。
    let (status, cli_failed) = post_json(
        &boot.http,
        &format!("{}/api/v1/settings/models/test", boot.base),
        &json!({"kind": "localCli", "endpoint": "definitely-not-a-real-cli-xyz --json"}),
    )
    .await;
    assert_eq!(status, 503, "{cli_failed}");

    // CLI 发现：返回全部候选与命中状态。
    let (status, found) = get_json(
        &boot.http,
        &format!("{}/api/v1/settings/models/cliCandidates", boot.base),
    )
    .await;
    assert_eq!(status, 200, "{found}");
    let items = found["items"].as_array().cloned().unwrap_or_default();
    assert!(
        items.iter().any(|item| item["id"] == json!("claude")),
        "候选应包含 Claude Code: {found}"
    );

    // 清理：删除测试模型。
    let deleted = boot
        .http
        .delete(format!("{}/api/v1/settings/models/{id}", boot.base))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status().as_u16(), 200);

    boot.running.shutdown().await.unwrap();
}
