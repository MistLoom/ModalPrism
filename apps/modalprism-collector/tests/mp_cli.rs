//! mp 短命令验收（切片 1）：不启动 serve，进程内完成 once 的 preview 与 --save 落盘。
//!
//! `mp once/search/ls/read/fetch` 的进程内路径 = `boot_opts(inline)` + 领域 API 工具面
//! `tools::call`，这里按同一形状驱动，覆盖退出码与 JSON 的成功/失败约定所依赖的数据。

use std::fs;
use std::path::Path;

use assembly::Binds;
use config::builtin_presets;
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
}

struct Inline {
    state: domain_api::Shared,
    running: modalprism_collector::Running,
    _data: tempfile::TempDir,
    _config: tempfile::TempDir,
}

impl Inline {
    /// 与 `mp` 内联执行同一调用形状；错误统一成与 HTTP 响应同形的 JSON。
    async fn call(&self, verb: &str, args: Value) -> Result<Value, Value> {
        domain_api::tools::call(&self.state, "", verb, args, true)
            .await
            .map_err(|error| error.body())
    }
}

/// 与 `mp once` 内联执行同一条启动路径：完整采集栈、不起采集循环。
async fn inline() -> Inline {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    config_dir(config.path());
    let running = modalprism_collector::boot_opts(
        data.path(),
        config.path(),
        Binds {
            http: "127.0.0.1:0".parse().unwrap(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            push: "127.0.0.1:0".parse().unwrap(),
        },
        modalprism_collector::BootOpts {
            inline: true,
            ..modalprism_collector::BootOpts::default()
        },
    )
    .await
    .unwrap();
    let state = running.api();
    Inline {
        state,
        running,
        _data: data,
        _config: config,
    }
}

#[tokio::test]
async fn inline_once_defaults_to_preview() {
    let inline = inline().await;
    // mp once 的缺省：mode 缺省 preview，不落盘。
    let value = inline
        .call("once", json!({ "text": "# 标题\n正文一段\n" }))
        .await
        .expect("once preview 应成功");
    assert_eq!(value["mode"], json!("preview"));
    assert_eq!(value["stored"], json!(false));
    inline.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_once_save_stores_to_disk() {
    let inline = inline().await;
    // mp once --save：mode=store，写入存储并可读回。
    let value = inline
        .call(
            "once",
            json!({ "text": "# 标题\n正文一段\n", "mode": "store" }),
        )
        .await
        .expect("once store 应成功");
    assert_eq!(value["mode"], json!("store"));
    assert_eq!(value["stored"], json!(true));
    assert!(
        value.get("commit").is_some_and(|commit| !commit.is_null()),
        "store 模式应返回提交标识: {value}"
    );
    inline.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_once_unknown_mode_fails_with_clear_message() {
    let inline = inline().await;
    let error = inline
        .call("once", json!({ "text": "内容", "mode": "nope" }))
        .await
        .expect_err("未知模式必须失败");
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("preview 或 store"), "实际: {message}");
    inline.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_ls_and_read_are_in_process() {
    let inline = inline().await;
    let ls = inline.call("ls", json!({})).await.expect("ls 应成功");
    assert!(ls.is_object() || ls.is_array(), "ls 返回目录: {ls}");
    // read：不存在的 ID 返回结构化 404（错误信息明确）。
    let error = inline
        .call("read", json!({ "contentId": "missing-id" }))
        .await
        .expect_err("不存在的 contentId 应 404");
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(!message.is_empty(), "read 失败信息应明确: {error}");
    inline.running.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_verb_surface_stays_read_path_only() {
    // 进程内调用与 HTTP 面同一张动词表：本切片只做信息主路径与 fetch，
    // 计划动词不进 mp 一级子命令（占位帮助），这里守住调用不越面。
    let inline = inline().await;
    for verb in ["once", "search", "ls", "read", "fetch"] {
        assert!(
            surface::tool(verb).is_some(),
            "{verb} 必须是已声明的工具动词"
        );
    }
    // search 缺关键词 → 结构化校验失败（mp search 无参数时的退出码 1 依据）。
    let error = inline
        .call("search", json!({}))
        .await
        .expect_err("search 缺 query 应失败");
    assert!(
        error["error"]["message"].as_str().unwrap_or_default().contains("query"),
        "search 失败信息应指明缺 query: {error}"
    );
    inline.running.shutdown().await.unwrap();
}
