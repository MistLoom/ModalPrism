//! M-04 验收：本地假 `accessor/api` 分页源，≥10,000 条按批落盘与中断续跑。
//!
//! 设计要求（MODALPRISM-DESIGN M-04）：
//! - 计划能稳定追加到 ≥10,000 条而不崩溃；
//! - 中途 pause 或杀掉进程再 continue，已落盘条数不丢、进度（游标）可查；
//! - 每页 apply + 提交 accepted_cursor（批式路径），而非整任务一次性提交。

use std::fs;
use std::path::Path;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::InvokeRequest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PER_PAGE: usize = 100;
const PAGES: usize = 100; // 100 页 × 100 条 = 10,000 条
const TOTAL: usize = PER_PAGE * PAGES;

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
        "id: system\nkind: system\nname: ModalPrism\npolicy:\n  preset: standard\n  egress:\n    private_networks: allow\n  robots:\n    mode: ignore\n  commit:\n    flush_interval: 1s\nstrata:\n  gateway:\n    blob_backend: dir\n",
    );
    write(
        dir,
        "accessors/http.yaml",
        "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    write(
        dir,
        "accessors/api.yaml",
        "id: accessor/api\nkind: accessor\nname: api\nimplementation: builtin:api\nlimits:\n  timeout: 300s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
}

fn binds() -> Binds {
    Binds {
        http: "127.0.0.1:0".parse().unwrap(),
        proxy: "127.0.0.1:0".parse().unwrap(),
        push: "127.0.0.1:0".parse().unwrap(),
    }
}

struct Boot {
    running: modalprism_collector::Running,
    state: domain_api::Shared,
    http: reqwest::Client,
    base: String,
}

impl Boot {
    async fn tool(&self, verb: &str, args: Value) -> Result<Value, Value> {
        domain_api::tools::call(&self.state, "proj_default", verb, args, true)
            .await
            .map_err(|error| error.body())
    }

    /// 内容条数是否已达到 n：用 offset 探针（单次请求），
    /// 避开 /content/items 的 COUNT_CAP=5000 截断与分页循环。
    async fn has_at_least(&self, n: usize) -> bool {
        let body: Value = self
            .http
            .get(format!(
                "{}/api/v1/content/items?limit=1&offset={}&scope=all",
                self.base,
                n - 1
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["items"].as_array().is_some_and(|items| !items.is_empty())
    }

    /// 廉价计数：直接读 total 字段（受 COUNT_CAP=5000 截断，仅小阈值用）。
    async fn total_capped(&self) -> usize {
        let body: Value = self
            .http
            .get(format!(
                "{}/api/v1/content/items?limit=1&scope=all",
                self.base
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["total"].as_u64().unwrap_or(0) as usize
    }

    /// 等待可见内容条数追平阈值：批式提交推进 accepted_cursor，
    /// 可见投影按 flush_interval 滞后，轮次结束后需等 flush。
    /// 阈值 ≤5000 用廉价 total 轮询；更大阈值用 offset 探针。
    async fn wait_total(&self, min: usize) {
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let reached = if min > 5000 {
                self.has_at_least(min).await
            } else {
                self.total_capped().await >= min
            };
            if reached {
                return;
            }
        }
        panic!("可见内容条数未在超时内达到 {min}");
    }

    /// 等待计划回到空闲（最近一轮不在 running），返回 plan.get 结果。
    async fn wait_round(&self, slug: &str) -> Value {
        for _ in 0..600 {
            let plan = self
                .tool("plan.get", json!({"planId": slug}))
                .await
                .unwrap_or_else(|error| panic!("plan.get: {error}"));
            let running = plan["recentRuns"][0]["status"] == json!("running");
            if !running {
                return plan;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        panic!("计划一轮未在超时内结束");
    }
}

async fn boot(data: &Path, config: &Path) -> Boot {
    let running = modalprism_collector::boot(data, config, binds()).await.unwrap();
    let state = running.api();
    let base = format!("http://{}", running.http());
    Boot {
        state,
        running,
        http: reqwest::Client::new(),
        base,
    }
}

/// 本地假分页 JSON API：`/items?page=N&per_page=100`，固定 100 页、每页 100 条。
/// 每条带唯一 url（urlTemplate 用它建身份），页与页之间游标推进。
async fn fake_api() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = vec![0u8; 8192];
            let Ok(read) = socket.read(&mut buffer).await else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let first_line = request.lines().next().unwrap_or_default().to_string();
            // 解析查询参数：page 与 per_page。
            let query = first_line
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.split_once('?'))
                .map(|(_, query)| query.to_string())
                .unwrap_or_default();
            let param = |key: &str| -> Option<usize> {
                query.split('&').find_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    (k == key).then(|| v.parse().ok()).flatten()
                })
            };
            let page = param("page").unwrap_or(1);
            let per_page = param("per_page").unwrap_or(PER_PAGE);
            // robots.txt：出口网关抓取前会查询，返回合法允许全文的 robots。
            if first_line.contains("/robots.txt") {
                let body = "User-agent: *\nAllow: /\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                continue;
            }
            if page > PAGES {
                // 越界页：空列表，游标终止。
                let body = format!(
                    "{{\"crates\": [], \"meta\": {{\"next_page\": null, \"total\": {TOTAL}}}}}"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                continue;
            }
            let start = (page - 1) * per_page;
            let items: Vec<String> = (start..start + per_page)
                .map(|n| {
                    format!(
                        "{{\"name\": \"crate-{n}\", \"url\": \"http://{addr}/crate/{n}\", \"description\": \"条目 {n} 的说明\"}}"
                    )
                })
                .collect();
            let next = if page < PAGES { page + 1 } else { PAGES + 1 };
            let body = format!(
                "{{\"crates\": [{}], \"meta\": {{\"next_page\": \"?page={next}\", \"total\": {TOTAL}}}}}",
                items.join(",")
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}/items")
}

/// 建信源（accessor/api + offset 分页）与计划，返回计划 slug。
async fn seed(boot: &Boot, base_url: &str) -> String {
    let request = InvokeRequest::new(
        InstanceId::new("catalog_admin").unwrap(),
        MethodId::new("create").unwrap(),
        json!({
            "kind": "source",
            "slug": "fake-api",
            "doc": {
                "name": "假站分页 API",
                "project": "proj_default",
                "shared": true,
                "sourceType": "api",
                "access": {
                    "accessor": "accessor/api",
                    "locator": {"endpoint": base_url},
                    "spec": {
                        "pages": 50,
                        "pagination": {"type": "offset", "param": "page", "start": 1, "step": 1}
                    }
                },
                "pipeline": [{
                    "run": ["adapter/json-list"],
                    "spec": {"adapter/json-list": {"items": "/crates", "url": "url", "summary": "description", "summaryIsBody": true}}
                }],
                "schedule": {"interval": "1d"},
                "admission": {}
            }
        }),
    );
    boot.running
        .gateway()
        .invoke(request)
        .await
        .unwrap_or_else(|error| panic!("建信源: {error}"));

    boot.tool(
        "plan.new",
        json!({
            "id": "batch-accept",
            "name": "万级验收计划",
            "targets": ["source/fake-api"],
            "trigger": {"kind": "recurring", "interval": "1h"},
            "quota": {"maxAge": "3650d", "perRunItems": 20000},
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("plan.new: {error}"));
    "batch-accept".to_string()
}

#[tokio::test]
async fn batch_ten_thousand_items_and_resume_after_kill() {
    let base_url = fake_api().await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());

    // 第一轮：独立进程（Boot A）跑 50 页 × 100 条 = 5,000 条后退出（模拟进程被杀）。
    {
        let boot = boot(data.path(), config.path()).await;
        let slug = seed(&boot, &base_url).await;
        let run = boot
            .tool("plan.run", json!({"planId": slug}))
            .await
            .unwrap_or_else(|error| panic!("第一轮 plan.run: {error}"));
        let plan = boot.wait_round(&slug).await;
        assert_eq!(
            run["dispatched"][0]["failed"].as_u64().unwrap_or(0),
            0,
            "第一轮不应有失败任务: {run}\nplan: {plan}"
        );
        let plan = boot
            .tool("plan.get", json!({"planId": slug}))
            .await
            .unwrap_or_else(|error| panic!("plan.get: {error}"));
        let cursor = &plan["cursors"][0]["acceptedCursor"];
        assert!(
            cursor.is_object(),
            "第一轮后游标应可查且非空: {plan}"
        );
        // 可见投影按 flush_interval 滞后于 accepted_cursor，等待 flush 后核对条数。
        boot.wait_total(5000).await;
        boot.running.shutdown().await.unwrap();
    }

    // 第二轮：全新进程（Boot B，同一数据目录）从上一接受游标继续，补齐到 ≥10,000 条。
    {
        let boot = boot(data.path(), config.path()).await;
        let run = boot
            .tool("plan.run", json!({"planId": "batch-accept"}))
            .await
            .unwrap_or_else(|error| panic!("第二轮 plan.run: {error}"));
        let plan = boot.wait_round("batch-accept").await;
        assert_eq!(
            run["dispatched"][0]["failed"].as_u64().unwrap_or(0),
            0,
            "第二轮不应有失败任务: {run}\nplan: {plan}"
        );
        println!(
            "DIAG round2 run={}",
            serde_json::to_string(&run).unwrap_or_default()
        );
        println!(
            "DIAG round2 cursors={}",
            serde_json::to_string(&plan["cursors"]).unwrap_or_default()
        );
        // 可见投影 flush 后用 offset 探针核对条数（避开 COUNT_CAP 截断）。
        boot.wait_total(TOTAL).await;
        // 游标可查：进度继续推进。
        let plan = boot
            .tool("plan.get", json!({"planId": "batch-accept"}))
            .await
            .unwrap_or_else(|error| panic!("plan.get: {error}"));
        assert!(
            plan["cursors"][0]["acceptedCursor"].is_object(),
            "第二轮后游标仍应可查: {plan}"
        );

        // pause/resume 语义：暂停不拨 enabled，暂停期间 run 被拒。
        boot.tool("plan.pause", json!({"planId": "batch-accept"}))
            .await
            .unwrap_or_else(|error| panic!("plan.pause: {error}"));
        let plan = boot
            .tool("plan.get", json!({"planId": "batch-accept"}))
            .await
            .unwrap_or_else(|error| panic!("plan.get after pause: {error}"));
        assert_eq!(plan["paused"], json!(true), "暂停应写独立 paused 字段");
        assert_eq!(plan["enabled"], json!(true), "暂停不应改动 enabled");
        let run = boot.tool("plan.run", json!({"planId": "batch-accept"})).await;
        assert!(run.is_err(), "暂停期间 plan.run 应被拒绝");
        boot.tool("plan.resume", json!({"planId": "batch-accept"}))
            .await
            .unwrap_or_else(|error| panic!("plan.resume: {error}"));
    }
}

/// M-05 验收：计划声明 `storage.root` + `classify` 后，接受条目按
/// `{effectiveRoot}/{sourceId}/{kind}/{itemId}{ext}` 落为可读文件树；
/// `plan get` 能读到 effectiveRoot 与 classifyRule。
#[tokio::test]
async fn storage_root_classify_writes_readable_tree() {
    let base_url = fake_api().await;
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let root = data.path().join("storage-root");
    config_dir(config.path());

    let boot = boot(data.path(), config.path()).await;
    // 信源声明 pages: 2（200 条），走 accessor/api 分页。
    let request = InvokeRequest::new(
        InstanceId::new("catalog_admin").unwrap(),
        MethodId::new("create").unwrap(),
        json!({
            "kind": "source",
            "slug": "fake-api",
            "doc": {
                "name": "假站分页 API",
                "project": "proj_default",
                "shared": true,
                "sourceType": "api",
                "access": {
                    "accessor": "accessor/api",
                    "locator": {"endpoint": base_url},
                    "spec": {
                        "pages": 2,
                        "pagination": {"type": "offset", "param": "page", "start": 1, "step": 1}
                    }
                },
                "pipeline": [{
                    "run": ["adapter/json-list"],
                    "spec": {"adapter/json-list": {"items": "/crates", "url": "url", "summary": "description", "summaryIsBody": true}}
                }],
                "schedule": {"interval": "1d"},
                "admission": {}
            }
        }),
    );
    boot.running
        .gateway()
        .invoke(request)
        .await
        .unwrap_or_else(|error| panic!("建信源: {error}"));

    boot.tool(
        "plan.new",
        json!({
            "id": "storage-plan",
            "name": "存储根验收计划",
            "targets": ["source/fake-api"],
            "trigger": {"kind": "recurring", "interval": "1h"},
            "storage": {"root": root.to_string_lossy(), "classify": true},
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("plan.new: {error}"));

    boot.tool("plan.run", json!({"planId": "storage-plan"}))
        .await
        .unwrap_or_else(|error| panic!("plan.run: {error}"));
    boot.wait_round("storage-plan").await;
    boot.wait_total(200).await;

    // plan get 暴露 effectiveRoot 与 classifyRule。
    let plan = boot
        .tool("plan.get", json!({"planId": "storage-plan"}))
        .await
        .unwrap_or_else(|error| panic!("plan.get: {error}"));
    assert_eq!(
        plan["storage"]["classify"], json!(true),
        "plan.get 应返回 classify: {plan}"
    );
    assert_eq!(
        plan["storage"]["effectiveRoot"],
        json!(root.to_string_lossy()),
        "plan.get 应返回 effectiveRoot"
    );
    assert!(
        plan["storage"]["classifyRule"]
            .as_str()
            .unwrap_or_default()
            .contains("{sourceId}"),
        "plan.get 应返回 classifyRule: {plan}"
    );

    // 文件树落位：{root}/fake-api/text/*.md 至少 200 个。
    let text_dir = root.join("fake-api").join("text");
    let count = std::fs::read_dir(&text_dir)
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    assert!(
        count >= 200,
        "分类文件树应在 {text_dir:?} 下落 ≥200 个文件，实际 {count}"
    );
}
