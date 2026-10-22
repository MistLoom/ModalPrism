//! Guest 合规断言（子进程 / duplex 共用）。

use std::time::Duration;

use ability_core::AbilityHost;
use ability_probe::{echo_provider, note_provider, wrap_provider};
use ability_support::{ErrorKind, EventClass, EventFilter, EventListener, LoadSpec, MethodId};
use frame_core::InvokeRequest;
use serde_json::json;

use super::fixture::{iid, sid};

fn mid(s: &str) -> MethodId {
    MethodId::new(s).unwrap()
}

/// probe：invoke + emit + unload 后 NotFound。
pub async fn assert_probe_invoke_emit_unload(host: &AbilityHost, from: &str) {
    let mut sub = host
        .events()
        .listen(EventFilter {
            class: Some(EventClass::Ability),
            name: Some("pong".into()),
            instance: None,
        })
        .await;

    let gw = host.clone().into_gateway();
    let resp = gw
        .invoke(InvokeRequest::new(iid("probe"), mid("ping"), json!({})))
        .await
        .unwrap();
    assert_eq!(resp.data["ok"], true);
    assert_eq!(resp.data["from"], from);

    let ev = sub.next().await.expect("pong");
    assert_eq!(ev.name(), "pong");

    host.unload(&iid("probe")).await.unwrap();
    let err = gw
        .invoke(InvokeRequest::new(iid("probe"), mid("ping"), json!({})))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

/// probe.wrap：别名 call + push_effect/local + emit done。
pub async fn assert_wrap_alias_call_emit(host: &AbilityHost) {
    let resp = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            iid("probe.wrap"),
            mid("run"),
            json!({ "n": 2 }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.data["wrapped"]["n"], 2);
    assert_eq!(resp.data["fx"], true);
}

/// stamp：同配置重装后 stamp 不变。
pub async fn assert_stamp_reload_same_impl(host: &AbilityHost) {
    let gw = host.clone().into_gateway();
    let spec = LoadSpec::new(sid("stamp"), iid("stamp")).with_config(json!({ "stamp": "x" }));
    host.load(spec.clone()).await.unwrap();
    let a = gw
        .invoke(InvokeRequest::new(iid("stamp"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(a.data["stamp"], "x");

    host.unload(&iid("stamp")).await.unwrap();
    host.load(spec).await.unwrap();

    let b = gw
        .invoke(InvokeRequest::new(iid("stamp"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(b.data["stamp"], "x");
}

/// stamp：reload 换 config.stamp（a→b）。
pub async fn assert_stamp_reload_new_session(host: &AbilityHost) {
    let gw = host.clone().into_gateway();
    host.load(LoadSpec::new(sid("stamp"), iid("stamp")).with_config(json!({ "stamp": "a" })))
        .await
        .unwrap();
    let a = gw
        .invoke(InvokeRequest::new(iid("stamp"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(a.data["stamp"], "a");

    host.unload(&iid("stamp")).await.unwrap();
    host.load(LoadSpec::new(sid("stamp"), iid("stamp")).with_config(json!({ "stamp": "b" })))
        .await
        .unwrap();
    let b = gw
        .invoke(InvokeRequest::new(iid("stamp"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(b.data["stamp"], "b");
}

pub async fn assert_subscribe_stream(host: &AbilityHost) {
    let gw = host.clone().into_gateway();
    let resp = gw
        .invoke(InvokeRequest::new(iid("probe.sub"), mid("ping"), json!({})))
        .await
        .unwrap();
    assert_eq!(resp.data["got"], "tick");
}

pub async fn assert_handle_middleware(host: &AbilityHost) {
    let gw = host.clone().into_gateway();
    let resp = gw
        .invoke(InvokeRequest::new(
            iid("probe.chain"),
            mid("run"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.data["middleware_ran"], true);
}

pub async fn assert_push_effect_undo(host: &AbilityHost) {
    let gw = host.clone().into_gateway();
    let err = gw
        .invoke(InvokeRequest::new(
            iid("probe.effect"),
            mid("run"),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Failed);

    let peek = gw
        .invoke(InvokeRequest::new(
            iid("probe.effect"),
            mid("peek"),
            json!({}),
        ))
        .await
        .unwrap();
    assert!(peek.data["fx"].is_null());
}

pub async fn assert_event_lanes(host: &AbilityHost) {
    let resp = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            iid("probe.surface"),
            mid("lanes"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.data["middleware"], true);
    assert_eq!(resp.data["pipeline"], true);
    assert_eq!(resp.data["parallel"], true);
    assert_eq!(resp.data["message"], true);
}

pub async fn assert_protocol_filter_retains(host: &AbilityHost) {
    let resp = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            iid("probe.surface"),
            mid("filter"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.data["shared_local"], true);
    assert_eq!(resp.data["same_instance"], true);
}

pub async fn assert_register_aspect_fails(host: &AbilityHost) {
    let err = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            iid("probe.surface"),
            mid("aspect"),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Failed);
    assert!(
        err.message().contains("Guest 不支持 register_aspect"),
        "{}",
        err.message()
    );
}

pub async fn assert_share_chain_status(host: &AbilityHost) {
    let gw = host.clone().into_gateway();
    let resp = gw
        .invoke(InvokeRequest::new(iid("probe.ctx"), mid("run"), json!({})))
        .await
        .unwrap();
    assert_eq!(resp.data["has_shared"], true);
    assert!(resp.data["chain"].is_string());
}

async fn wait_hub_status(
    gw: &dyn frame_core::Gateway,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let mut last = json!({});
    for _ in 0..80 {
        last = gw
            .invoke(InvokeRequest::new(
                iid("probe.hub"),
                mid("status"),
                json!({}),
            ))
            .await
            .unwrap()
            .data;
        if pred(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    last
}

pub async fn assert_hub_guest_surface(host: &AbilityHost) {
    host.install(sid("probe.echo"), echo_provider()).await;
    host.install(sid("probe.wrap"), wrap_provider()).await;
    host.install(sid("probe.note"), note_provider()).await;
    host.load(LoadSpec::new(sid("probe.echo"), iid("probe.echo")))
        .await
        .unwrap();

    let mut ready = host
        .events()
        .listen(EventFilter {
            class: Some(EventClass::Ability),
            name: Some("ready".into()),
            instance: None,
        })
        .await;
    let mut caught = host
        .events()
        .listen(EventFilter {
            class: Some(EventClass::Ability),
            name: Some("caught".into()),
            instance: None,
        })
        .await;

    host.load(
        LoadSpec::new(sid("probe.hub"), iid("probe.hub")).with_config(json!({ "tag": "hub-1" })),
    )
    .await
    .unwrap();
    assert_eq!(ready.next().await.expect("ready").name(), "ready");

    host.load(LoadSpec::new(sid("probe.wrap"), iid("probe.wrap")))
        .await
        .unwrap();

    let gw = host.clone().into_gateway();
    gw.invoke(InvokeRequest::new(
        iid("probe.wrap"),
        mid("run"),
        json!({ "n": 1 }),
    ))
    .await
    .unwrap();
    assert_eq!(caught.next().await.expect("caught").name(), "caught");

    let st = wait_hub_status(&*gw, |st| {
        st["heard"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("done")))
            && !st["injected"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true)
    })
    .await;
    assert_eq!(st["tag"], "hub-1");
    assert_eq!(st["echo"], "probe.echo");

    gw.invoke(InvokeRequest::new(iid("probe.hub"), mid("boot"), json!({})))
        .await
        .unwrap();
    assert!(
        host.catalog()
            .iter()
            .any(|e| e.instance.as_str() == "probe.note")
    );

    gw.invoke(InvokeRequest::new(
        iid("probe.note"),
        mid("write"),
        json!({ "k": 1 }),
    ))
    .await
    .unwrap();
    assert_eq!(caught.next().await.expect("noted→caught").name(), "caught");

    let st = wait_hub_status(&*gw, |st| {
        st["heard"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("noted")))
    })
    .await;
    assert!(
        st["catalog"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("probe.note"))
    );

    gw.invoke(InvokeRequest::new(
        iid("probe.hub"),
        mid("unboot"),
        json!({}),
    ))
    .await
    .unwrap();
    assert!(
        !host
            .catalog()
            .iter()
            .any(|e| e.instance.as_str() == "probe.note")
    );

    let ping = gw
        .invoke(InvokeRequest::new(
            iid("probe.hub"),
            mid("ping"),
            json!({ "text": "hub" }),
        ))
        .await
        .unwrap();
    assert_eq!(ping.data["via"]["text"], "hub");
}
