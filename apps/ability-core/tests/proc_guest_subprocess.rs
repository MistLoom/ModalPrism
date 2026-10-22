//! Guest 子进程（Node / Python）冒烟：全量断言见 `proc_remote`（duplex）与 `guest_conformance`。

mod common;

use ability_core::AbilityHost;
use common::fixture::{install_guest_subprocess};
use common::guest_conformance::{
    assert_event_lanes, assert_probe_invoke_emit_unload, assert_protocol_filter_retains,
    assert_register_aspect_fails,
};

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

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_ts_invoke_emit_unload() {
    require_node();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-ts").await;
    assert_probe_invoke_emit_unload(&host, "external-ts").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subprocess_probe_py_invoke_emit_unload() {
    require_python();
    let host = AbilityHost::new();
    install_guest_subprocess(&host, "probe", "external/ability-probe-py").await;
    assert_probe_invoke_emit_unload(&host, "external-py").await;
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
