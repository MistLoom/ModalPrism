//! Guest 子进程（Node / Python）冒烟：全量断言见 `proc_remote`（duplex）与 `guest_conformance`。

mod common;

use ability_core::AbilityHost;
use common::fixture::{install_guest_subprocess};
use common::guest_conformance::{
    assert_event_lanes, assert_probe_invoke_emit_unload, assert_protocol_filter_retains,
    assert_register_aspect_fails,
};
use ability_support::{InstanceId, MethodId};
use frame_core::InvokeRequest;
use serde_json::json;

fn sid(s: &str) -> ability_support::SourceId {
    ability_support::SourceId::new(s).unwrap()
}

fn iid(s: &str) -> ability_support::InstanceId {
    ability_support::InstanceId::new(s).unwrap()
}

fn mid(s: &str) -> ability_support::MethodId {
    ability_support::MethodId::new(s).unwrap()
}

fn require_node() {
    assert!(
        std::process::Command::new("node")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        "需要 Node.js 以运行 guest 子进程合规测试",
    );
}

fn require_python() {
    assert!(
        std::process::Command::new("python")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        "需要 Python 以运行 guest 子进程合规测试",
    );
}

/// 仓颉工具链检查：有 cjpm 则跑 E2E；无则显式跳过（不假绿，CI Notes/矩阵保持真值）。
fn require_cangjie() -> bool {
    let ok = std::process::Command::new("cjpm")
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("跳过：本机无 cjpm（仓颉工具链），仓颉 Guest 探针不在此环境运行");
    }
    ok
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_ts_invoke_emit_unload() {
    require_node();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-ts").await;
    assert_probe_invoke_emit_unload(&host, "external-ts").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_ts_long_task_handle_poll() {
    require_node();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-ts").await;
    let gw = host.clone().into_gateway();

    // 提交长任务：立即返回 task_handle，不等待完成
    let resp = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "ms": 120 }),
        ))
        .await
        .unwrap();
    let handle = resp.data["task_handle"].as_str().expect("task_handle").to_string();
    assert!(!handle.is_empty());

    // 轮询：未完成 → running
    let poll = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(poll.data["state"], "running");

    // 轮询直至完成：done + 结果数据
    let mut final_state = None;
    for _ in 0..50 {
        let poll = gw
            .invoke(InvokeRequest::new(
                InstanceId::new("probe").unwrap(),
                MethodId::new("slow").unwrap(),
                json!({ "handle": handle }),
            ))
            .await
            .unwrap();
        if poll.data["state"] != "running" {
            final_state = Some(poll.data.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let final_data = final_state.expect("长任务应在超时前完成");
    assert_eq!(final_data["state"], "done");
    assert_eq!(final_data["data"]["slept"], 120);

    // 完成后 handle 释放：再轮询得 not_found
    let gone = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(gone.data["state"], "not_found");

    // 失败路径：failed + 原始错误消息
    let resp = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "ms": 20, "fail": true }),
        ))
        .await
        .unwrap();
    let handle = resp.data["task_handle"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let poll = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(poll.data["state"], "failed");
    assert_eq!(poll.data["error"], "boom");

    host.unload(&InstanceId::new("probe").unwrap()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_py_invoke_emit_unload() {
    require_python();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-py").await;
    assert_probe_invoke_emit_unload(&host, "external-py").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_py_long_task_handle_poll() {
    require_python();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-py").await;
    let gw = host.clone().into_gateway();

    // 提交长任务：立即返回 task_handle，不等待完成
    let resp = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "ms": 120 }),
        ))
        .await
        .unwrap();
    let handle = resp.data["task_handle"].as_str().expect("task_handle").to_string();
    assert!(!handle.is_empty());

    // 轮询直至完成：done + 结果数据（Python 线程后台，可能在首次 poll 前已完成）
    let mut final_state = None;
    for _ in 0..50 {
        let poll = gw
            .invoke(InvokeRequest::new(
                InstanceId::new("probe").unwrap(),
                MethodId::new("slow").unwrap(),
                json!({ "handle": handle }),
            ))
            .await
            .unwrap();
        if poll.data["state"] != "running" {
            final_state = Some(poll.data.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let final_data = final_state.expect("长任务应在超时前完成");
    assert_eq!(final_data["state"], "done");
    assert_eq!(final_data["data"]["slept"], 120);

    // 完成后 handle 释放：再轮询得 not_found
    let gone = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(gone.data["state"], "not_found");

    // 失败路径：failed + 原始错误消息
    let resp = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "ms": 20, "fail": true }),
        ))
        .await
        .unwrap();
    let handle = resp.data["task_handle"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let poll = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(poll.data["state"], "failed");
    assert_eq!(poll.data["error"], "boom");

    host.unload(&InstanceId::new("probe").unwrap()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_cj_invoke_emit_unload() {
    if !require_cangjie() {
        return;
    }
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-cj").await;
    assert_probe_invoke_emit_unload(&host, "external-cj").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_cj_long_task_handle_poll() {
    if !require_cangjie() {
        return;
    }
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-cj").await;
    let gw = host.clone().into_gateway();

    // 提交长任务：立即返回 task_handle，不等待完成
    let resp = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "ms": 120 }),
        ))
        .await
        .unwrap();
    let handle = resp.data["task_handle"].as_str().expect("task_handle").to_string();
    assert!(!handle.is_empty());

    // 轮询直至完成：done + 结果数据（仓颉后台协程，可能在首次 poll 前已完成）
    let mut final_state = None;
    for _ in 0..50 {
        let poll = gw
            .invoke(InvokeRequest::new(
                InstanceId::new("probe").unwrap(),
                MethodId::new("slow").unwrap(),
                json!({ "handle": handle }),
            ))
            .await
            .unwrap();
        if poll.data["state"] != "running" {
            final_state = Some(poll.data.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let final_data = final_state.expect("长任务应在超时前完成");
    assert_eq!(final_data["state"], "done");
    assert_eq!(final_data["data"]["slept"], 120);

    // 完成后 handle 释放：再轮询得 not_found
    let gone = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(gone.data["state"], "not_found");

    // 失败路径：failed + 原始错误消息
    let resp = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "ms": 20, "fail": true }),
        ))
        .await
        .unwrap();
    let handle = resp.data["task_handle"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let poll = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("probe").unwrap(),
            MethodId::new("slow").unwrap(),
            json!({ "handle": handle }),
        ))
        .await
        .unwrap();
    assert_eq!(poll.data["state"], "failed");
    assert_eq!(poll.data["error"], "boom");

    host.unload(&InstanceId::new("probe").unwrap()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_surface_ts_guest_ctx() {
    require_node();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe.surface", "external/ability-probe-surface-ts").await;
    assert_event_lanes(&host).await;
    assert_protocol_filter_retains(&host).await;
    assert_register_aspect_fails(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_surface_py_guest_ctx() {
    require_python();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe.surface", "external/ability-probe-surface-py").await;
    assert_event_lanes(&host).await;
    assert_protocol_filter_retains(&host).await;
    assert_register_aspect_fails(&host).await;
}
