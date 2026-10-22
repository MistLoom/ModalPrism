//! 远程 Provider：duplex 协议；断言见 guest_conformance。

mod common;

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_probe::{
    chain_provider, ctx_provider, echo_provider, effect_provider, feedback_provider, hub_provider,
    provider, stamp_provider, sub_provider, surface_provider, wrap_provider,
};
use ability_sdk::AbilitySpec;
use ability_sdk::prelude::*;
use ability_support::Ability;
use ability_support::LoadSpec;
use common::fixture::{boot_duplex, duplex_provider, iid, install_connected, mid, sid};
use common::guest_conformance::{
    assert_event_lanes, assert_handle_middleware, assert_hub_guest_surface,
    assert_probe_invoke_emit_unload, assert_protocol_filter_retains, assert_push_effect_undo,
    assert_register_aspect_fails, assert_share_chain_status, assert_stamp_reload_new_session,
    assert_stamp_reload_same_impl, assert_subscribe_stream, assert_wrap_alias_call_emit,
};
use frame_core::InvokeRequest;
use serde_json::json;

fn stamp_duplex_provider(stamp: &'static str) -> Arc<dyn ability_support::AbilityProvider> {
    AbilitySpec::must("stamp")
        .on(
            "run",
            "印记",
            json!({ "type": "object" }),
            handler!(|_ctx, _args| Ok(json!({ "stamp": stamp }))),
        )
        .into_provider()
}

async fn boot_stamp(stamp: &'static str) -> Arc<dyn Ability> {
    boot_duplex(stamp_duplex_provider(stamp), json!({})).await
}

async fn install_duplex_loaded(
    host: &AbilityHost,
    source: &str,
    provider: Arc<dyn ability_support::AbilityProvider>,
) {
    host.install(sid(source), duplex_provider(provider)).await;
    host.load(LoadSpec::new(sid(source), iid(source)))
        .await
        .unwrap();
}

async fn install_duplex_only(
    host: &AbilityHost,
    source: &str,
    provider: Arc<dyn ability_support::AbilityProvider>,
) {
    host.install(sid(source), duplex_provider(provider)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_probe_invoke_emit_unload() {
    let ability = boot_duplex(provider(), json!({})).await;
    let host = AbilityHost::new();
    install_connected(&host, "probe", ability).await;
    assert_probe_invoke_emit_unload(&host, "external").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_wrap_calls_in_process_echo() {
    let wrap = boot_duplex(wrap_provider(), json!({})).await;
    let host = AbilityHost::new();
    host.install(sid("probe.echo"), echo_provider()).await;
    host.load(LoadSpec::new(sid("probe.echo"), iid("probe.echo")))
        .await
        .unwrap();
    install_connected(&host, "probe.wrap", wrap).await;
    assert_wrap_alias_call_emit(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_unload_reload_new_remote() {
    let host = AbilityHost::new();
    install_connected(&host, "stamp", boot_stamp("a").await).await;
    let gw = host.clone().into_gateway();
    let a = gw
        .invoke(InvokeRequest::new(iid("stamp"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(a.data["stamp"], "a");

    host.unload(&iid("stamp")).await.unwrap();
    install_connected(&host, "stamp", boot_stamp("b").await).await;
    let b = gw
        .invoke(InvokeRequest::new(iid("stamp"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(b.data["stamp"], "b");
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_stamp_reload_same_impl() {
    let host = AbilityHost::new();
    install_duplex_only(&host, "stamp", stamp_provider()).await;
    assert_stamp_reload_same_impl(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_stamp_reload_new_session() {
    let host = AbilityHost::new();
    install_duplex_only(&host, "stamp", stamp_provider()).await;
    assert_stamp_reload_new_session(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_feedback() {
    let host = AbilityHost::new();
    install_duplex_loaded(&host, "probe.feedback", feedback_provider()).await;
    ability_support::feedback::purge();
    let gw = host.clone().into_gateway();
    let resp = gw
        .invoke(InvokeRequest::new(
            iid("probe.feedback"),
            mid("ping"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.data["ok"], true);
    // 同进程：客端 commit 一次，宿主 accept 再落一次。
    let n = ability_support::feedback::list()
        .iter()
        .filter(|r| r.desc == "probe-feedback-rust")
        .count();
    assert!(n >= 2, "feedback 应同时留在客端与宿主，实际 {n}");
    ability_support::feedback::purge();
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_hub_surface() {
    let host = AbilityHost::new();
    install_duplex_only(&host, "probe.hub", hub_provider()).await;
    assert_hub_guest_surface(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_subscribe() {
    let host = AbilityHost::new();
    install_duplex_loaded(&host, "probe.sub", sub_provider()).await;
    assert_subscribe_stream(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_handle_middleware() {
    let host = AbilityHost::new();
    install_duplex_loaded(&host, "probe.chain", chain_provider()).await;
    assert_handle_middleware(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_push_effect_undo() {
    let host = AbilityHost::new();
    install_duplex_loaded(&host, "probe.effect", effect_provider()).await;
    assert_push_effect_undo(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_share_chain_status() {
    let host = AbilityHost::new();
    install_duplex_loaded(&host, "probe.ctx", ctx_provider()).await;
    assert_share_chain_status(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn duplex_surface() {
    let host = AbilityHost::new();
    install_duplex_loaded(&host, "probe.surface", surface_provider()).await;
    assert_event_lanes(&host).await;
    assert_protocol_filter_retains(&host).await;
    assert_register_aspect_fails(&host).await;
}
