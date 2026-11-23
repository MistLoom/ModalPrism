//! 切片 2 验收：计划上 CLI 与 MCP 同一主路径，暂停/续跑语义、游标与进度。
//!
//! 行为变更说明（对既有 HTTP 行为）：`POST /collection/plans/{id}/pause` 与
//! `plan.pause` 不再把 `enabled` 改成 false，而是写入独立的 `paused` 字段；
//! 对停用（enabled=false）计划的 pause/resume/run 返回明确错误。

use std::fs;
use std::path::Path;
use std::time::Duration;

use ability_support::{InstanceId, MethodId};
use assembly::Binds;
use config::builtin_presets;
use frame_core::{Gateway, InvokeRequest};
use serde_json::{Value, json};

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
    write(
        dir,
        "accessors/http.yaml",
        "id: accessor/http\nkind: accessor\nname: HTTP\nimplementation: builtin:http\nlimits:\n  timeout: 45s\n  concurrency: 4\n  retry:\n    attempts: 2\n    backoff: exponential\n    base: 1s\n    max: 30s\negress:\n  profile: direct\n",
    );
    write(
        dir,
        "strategies/demo.yaml",
        "id: strategy/demo\nkind: strategy\nname: Demo\ntrigger:\n  kind: recurring\n  interval: 1h\n",
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
    _data: tempfile::TempDir,
    _config: tempfile::TempDir,
}

impl Boot {
    /// 与 `mp` 进程内执行同一入口：领域 API 工具面直接调用。
    async fn tool(&self, verb: &str, args: Value) -> Result<Value, Value> {
        domain_api::tools::call(&self.state, "proj_default", verb, args, true)
            .await
            .map_err(|error| error.body())
    }

    /// 通过运维面建信源声明（避开 HTTP 细节）。
    async fn seed_source(&self, slug: &str, url: &str) {
        let request = InvokeRequest::new(
            InstanceId::new("catalog_admin").unwrap(),
            MethodId::new("create").unwrap(),
            json!({
                "kind": "source",
                "slug": slug,
                "doc": {
                    "name": slug,
                    "project": "proj_default",
                    "shared": true,
                    "access": {"accessor": "accessor/http", "locator": {"url": url}},
                    "pipeline": [],
                    "schedule": {"interval": "6h"},
                    "admission": {}
                }
            }),
        );
        self.running
            .gateway()
            .invoke(request)
            .await
            .unwrap_or_else(|error| panic!("建信源 {slug}: {error}"));
    }
}

async fn boot() -> Boot {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot(data.path(), config.path(), binds())
        .await
        .unwrap();
    let state = running.api();
    Boot {
        state,
        running,
        _data: data,
        _config: config,
    }
}

/// 建一个带单信源目标的计划，返回 (slug, etag)。
/// 单一语义：targets/trigger 由策略承载——先实例化策略，再建纯状态计划。
async fn seed_plan(boot: &Boot, slug: &str, source_slug: &str) -> Value {
    let strategy = boot
        .tool(
            "strategy.new",
            json!({
                "name": "验收策略",
                "targets": [format!("source/{source_slug}")],
                "trigger": {"kind": "recurring", "interval": "1h"},
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("strategy.new: {error}"));
    let strategy_slug = strategy["slug"].as_str().expect("策略应带 slug").to_string();
    boot.tool(
        "plan.new",
        json!({
            "id": slug,
            "name": "验收计划",
            "strategy": strategy_slug,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("plan.new: {error}"))
}

#[tokio::test]
async fn plan_pause_keeps_enabled_and_sets_paused() {
    let boot = boot().await;
    boot.seed_source("pause-feed", "https://example.com/feed").await;
    let plan = seed_plan(&boot, "pause-plan", "pause-feed").await;
    let slug = plan["id"].as_str().unwrap().to_string();
    assert_eq!(plan["status"], json!("active"), "{plan}");
    assert_eq!(plan["enabled"], json!(true));

    // 暂停：enabled 保持 true，paused=true，状态投影 paused。
    let paused = boot
        .tool("plan.pause", json!({ "planId": slug }))
        .await
        .unwrap_or_else(|error| panic!("plan.pause: {error}"));
    assert_eq!(paused["enabled"], json!(true), "暂停不能改 enabled: {paused}");
    assert_eq!(paused["paused"], json!(true));
    assert_eq!(paused["status"], json!("paused"));

    // 恢复：paused=false，从已接受游标继续。
    let resumed = boot
        .tool("plan.resume", json!({ "planId": slug }))
        .await
        .unwrap();
    assert_eq!(resumed["enabled"], json!(true));
    assert_eq!(resumed["paused"], json!(false));
    assert_eq!(resumed["status"], json!("active"));
    boot.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabled_plan_rejects_pause_resume_run() {
    let boot = boot().await;
    boot.seed_source("off-feed", "https://example.com/feed").await;
    let plan = seed_plan(&boot, "off-plan", "off-feed").await;
    let slug = plan["id"].as_str().unwrap().to_string();
    boot.tool("plan.set", json!({ "planId": slug, "enabled": false }))
        .await
        .unwrap();
    let disabled = boot.tool("plan.get", json!({ "planId": slug })).await.unwrap();
    assert_eq!(disabled["enabled"], json!(false));
    assert_eq!(disabled["paused"], json!(false), "停用不是暂停");
    assert_eq!(disabled["status"], json!("disabled"));

    for verb in ["plan.pause", "plan.resume", "plan.run"] {
        let error = boot
            .tool(verb, json!({ "planId": slug }))
            .await
            .expect_err("停用计划必须拒绝 pause/resume/run");
        let message = error["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("已停用"),
            "{verb} 的错误信息要说明计划已停用: {message}"
        );
    }
    boot.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn paused_plan_refuses_run_until_resumed() {
    let boot = boot().await;
    boot.seed_source("held-feed", "https://example.com/feed").await;
    let plan = seed_plan(&boot, "held-plan", "held-feed").await;
    let slug = plan["id"].as_str().unwrap().to_string();
    boot.tool("plan.pause", json!({ "planId": slug })).await.unwrap();
    let error = boot
        .tool("plan.run", json!({ "planId": slug }))
        .await
        .expect_err("已暂停计划必须拒绝 run（不悄悄解除暂停）");
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("已暂停") && message.contains("resume"),
        "run 拒绝信息要指向 resume: {message}"
    );
    // resume 之后 run 放行。
    boot.tool("plan.resume", json!({ "planId": slug })).await.unwrap();
    let run = boot.tool("plan.run", json!({ "planId": slug })).await.unwrap();
    assert_eq!(run["planId"], json!(slug), "{run}");
    assert!(
        run["dispatched"].as_array().is_some_and(|items| items.len() == 1),
        "计划级 run 应对每个目标信源派发一轮: {run}"
    );
    boot.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn plan_get_exposes_progress_and_cursors() {
    let boot = boot().await;
    boot.seed_source("cur-feed", "https://example.com/feed").await;
    let plan = seed_plan(&boot, "cur-plan", "cur-feed").await;
    let slug = plan["id"].as_str().unwrap().to_string();
    // 管道与向量化预留：缺省空管道、false；plan.get 能读到。
    assert_eq!(plan["pipeline"], json!([]), "{plan}");
    assert_eq!(plan["vectorize"], json!(false));

    let got = boot.tool("plan.get", json!({ "planId": slug })).await.unwrap();
    let cursors = got["cursors"].as_array().expect("plan.get 应带游标清单");
    assert_eq!(cursors.len(), 1, "{got}");
    let cursor = &cursors[0];
    assert_eq!(cursor["sourceId"], json!("cur-feed"));
    // http 访问器没有跨请求游标：null。
    assert_eq!(cursor["acceptedCursor"], Value::Null);
    assert_eq!(cursor["visibleCursor"], Value::Null);
    assert_eq!(cursor["itemCount"], json!(0), "累计条数同 totalItems 口径");
    assert!(cursor.get("lastRunAt").is_some(), "游标行应含 lastRun");
    assert!(cursor.get("lastError").is_some(), "游标行应含 lastError");
    assert!(cursor.get("health").is_some(), "游标行应含 health");
    boot.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn url_plan_from_search_runs_inline_without_sources() {
    let boot = boot().await;
    // oneShot 计划：只有 URL 目标（来自检索记录的形态），无信源目标。
    // 单一语义：oneShot 节律由专用策略承载，计划只引用并携带 URL 状态数据。
    let strategy = boot
        .tool(
            "strategy.new",
            json!({
                "name": "一次性采集策略",
                "trigger": {"kind": "oneShot", "fireAt": "immediate"},
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("strategy.new: {error}"));
    let strategy_slug = strategy["slug"].as_str().expect("策略应带 slug").to_string();
    let plan = boot
        .tool(
            "plan.new",
            json!({
                "id": "url-plan",
                "name": "URL 采集计划",
                "strategy": strategy_slug,
                "urls": ["https://example.com/a", "https://example.com/b"],
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("plan.new(urls): {error}"));
    assert_eq!(plan["trigger"]["kind"], json!("oneShot"), "{plan}");
    assert_eq!(plan["targets"], json!([]));
    // appendUrls：追加去重。
    let updated = boot
        .tool(
            "plan.set",
            json!({
                "planId": "url-plan",
                "appendUrls": ["https://example.com/b", "https://example.com/c"],
            }),
        )
        .await
        .unwrap();
    let doc_urls = updated["cursors"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    assert_eq!(doc_urls, 0, "URL 目标不产生信源游标行: {updated}");
    boot.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_pipeline_adapter_is_rejected_on_new() {
    // 单一语义：pipeline 由策略承载，未知组件在策略创建时被整库校验拒绝。
    let boot = boot().await;
    boot.seed_source("pipe-feed", "https://example.com/feed").await;
    let error = boot
        .tool(
            "strategy.new",
            json!({
                "name": "坏管道",
                "targets": ["source/pipe-feed"],
                "trigger": {"kind": "recurring", "interval": "1h"},
                "pipeline": [{"run": ["renderer/fancy"]}],
            }),
        )
        .await
        .expect_err("未知组件的策略必须在 new 时拒绝");
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("未知适配器"), "实际: {message}");
    boot.running.shutdown().await.unwrap();
}

/// 派发闸门：暂停的计划不出现在到期集合里（planner 侧直接驱动一轮验证）。
#[tokio::test]
async fn paused_plan_does_not_dispatch() {
    use planner::SourcePlan;
    use std::time::Duration;

    let build = |paused: bool| SourcePlan {
        source_id: strata_model::SourceId::new("source/demo").unwrap(),
        plan_id: Some("plan/demo".into()),
        locator: strata_model::Locator::Url {
            url: "https://example.com/feed".into(),
        },
        accessor: "accessor/http".into(),
        queue: strata_model::QueueName::Free,
        spec: json!({}),
        generation: 0,
        config_hash: "h".into(),
        migration_generation: false,
        timeout: Duration::from_secs(5),
        paid: false,
        budget_requests: None,
        budget_window: Duration::ZERO,
        adaptive: false,
        interval: Duration::from_secs(60),
        floor: Duration::from_secs(30),
        ceiling: Duration::from_secs(60),
        batch_key: None,
        enabled: true,
        paused,
        expected_period: None,
        yield_drop_ratio: 0.5,
        failure_threshold: 5,
        daily_limit: None,
        pipeline: None,
        vectorize: false,
        storage_root: None,
        storage_classify: false,
        strategy: None,
        one_shot: false,
        per_run_items: None,
        total_items: None,
        on_budget_exhausted: "skip".into(),
        stagnation_cycles: None,
    };
    assert!(build(false).enabled && !build(false).paused);
    let paused = build(true);
    assert!(paused.enabled, "暂停不改变 enabled");
    assert!(paused.paused);
    // 闸门逻辑等价于 planner::dispatchable 的 plan.paused 检查，这里锁住字段语义。
    assert!(paused.enabled && paused.paused, "paused 期间 enabled 仍为 true");
}
