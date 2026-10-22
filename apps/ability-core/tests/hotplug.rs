mod common;

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, AbilityCtx, AbilityId, AbilityManifest, AbilityProvider, CtxFnAbility, Dependency,
    Effect, Error, ErrorKind, FnAbility, InstanceId, LoadSpec, MethodId, Result as CoreResult,
    SourceId, StaticProvider, Undo,
};
use common::echo_provider;
use frame_core::{Gateway, InvokeRequest};
use serde_json::json;

#[tokio::test]
async fn hotplug_load_invoke_unload() {
    let host = AbilityHost::new();
    host.install_str("echo", echo_provider("echo"))
        .await
        .unwrap();
    let id = host
        .load(LoadSpec::new(
            SourceId::new("echo").unwrap(),
            InstanceId::new("echo").unwrap(),
        ))
        .await
        .unwrap();

    let gw: Arc<dyn Gateway> = host.clone().into_gateway();
    let resp = gw
        .invoke(InvokeRequest::new(
            id.clone(),
            MethodId::new("run").unwrap(),
            json!({ "x": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.data["x"], 1);

    host.unload(&id).await.unwrap();
    let err = gw
        .invoke(InvokeRequest::new(
            id,
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn refuse_implicit_multi_instance_dep() {
    let host = AbilityHost::new();
    host.install_str("echo", echo_provider("echo"))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("a").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("b").unwrap(),
    ))
    .await
    .unwrap();

    let mut m = common::manifest("user", &["run"]);
    m.depends = vec![Dependency::required(AbilityId::new("echo").unwrap())];
    let user: Arc<dyn Ability> = Arc::new(FnAbility::new(m, |a| async move { Ok(a) }));
    host.install_str("user", Arc::new(StaticProvider::new(user)))
        .await
        .unwrap();
    let err = host
        .load(LoadSpec::new(
            SourceId::new("user").unwrap(),
            InstanceId::new("user").unwrap(),
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("多个实例"));
}

struct FlagEffect {
    flag: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl Effect for FlagEffect {
    async fn apply(&self, _ctx: &dyn AbilityCtx) -> CoreResult<Undo> {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let flag = Arc::clone(&self.flag);
        Ok(Undo::new(move || {
            let flag = Arc::clone(&flag);
            async move {
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }))
    }
}

#[tokio::test]
async fn undo_runs_on_failure() {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = Arc::clone(&flag);
    let m = common::manifest("boom", &["run"]);
    let ability: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(m, move |ctx, _args| {
        let f = Arc::clone(&f);
        Box::pin(async move {
            ctx.push_effect(Box::new(FlagEffect { flag: f })).await?;
            Err(Error::failed("boom"))
        })
    }));
    let host = AbilityHost::new();
    host.install_str("boom", Arc::new(StaticProvider::new(ability)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("boom").unwrap(),
        InstanceId::new("boom").unwrap(),
    ))
    .await
    .unwrap();
    let err = host
        .clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("boom").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("boom"));
    assert!(
        !flag.load(std::sync::atomic::Ordering::SeqCst),
        "Undo 应将 flag 置回 false"
    );
}

#[tokio::test]
async fn unload_drains_in_flight_and_rejects_new() {
    use std::time::Duration;

    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let entered_c = Arc::clone(&entered);
    let release_c = Arc::clone(&release);
    let m = common::manifest("slow", &["run"]);
    let ability: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(m, move |_, _| {
        let entered = Arc::clone(&entered_c);
        let release = Arc::clone(&release_c);
        Box::pin(async move {
            entered.notify_one();
            release.notified().await;
            Ok(json!({ "ok": true }))
        })
    }));
    let host = AbilityHost::new();
    host.install_str("slow", Arc::new(StaticProvider::new(ability)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("slow").unwrap(),
        InstanceId::new("slow").unwrap(),
    ))
    .await
    .unwrap();

    let gw: Arc<dyn Gateway> = host.clone().into_gateway();
    let gw_invoke = Arc::clone(&gw);
    let first = tokio::spawn(async move {
        gw_invoke
            .invoke(InvokeRequest::new(
                InstanceId::new("slow").unwrap(),
                MethodId::new("run").unwrap(),
                json!({}),
            ))
            .await
    });
    entered.notified().await;

    let host_u = host.clone();
    let unloading =
        tokio::spawn(async move { host_u.unload(&InstanceId::new("slow").unwrap()).await });
    tokio::time::sleep(Duration::from_millis(30)).await;

    let err = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("slow").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);

    release.notify_one();
    let first = first.await.unwrap().unwrap();
    assert_eq!(first.data["ok"], true);
    unloading.await.unwrap().unwrap();

    let err = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("slow").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn stateful_ability_single_instance() {
    let host = AbilityHost::new();
    let p = ability_support::AbilitySpec::must("hold")
        .stateful()
        .on(
            "run",
            "n",
            json!({ "type": "object" }),
            ability_support::handler!(|_ctx, args| Ok(args)),
        )
        .into_provider();
    host.install_str("hold", p).await.unwrap();
    host.load(LoadSpec::new(
        SourceId::new("hold").unwrap(),
        InstanceId::new("a").unwrap(),
    ))
    .await
    .unwrap();
    let err = host
        .load(LoadSpec::new(
            SourceId::new("hold").unwrap(),
            InstanceId::new("b").unwrap(),
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("单实例"));
}

#[tokio::test]
async fn locals_roundtrip_on_stateless() {
    let host = AbilityHost::new();
    host.install_str("echo", echo_provider("echo"))
        .await
        .unwrap();
    let id = host
        .load(LoadSpec::new(
            SourceId::new("echo").unwrap(),
            InstanceId::new("echo").unwrap(),
        ))
        .await
        .unwrap();
    let gw: Arc<dyn Gateway> = host.clone().into_gateway();
    let scope = ability_support::LocalsScope::new();
    let resp = gw
        .invoke(
            InvokeRequest::new(id, MethodId::new("run").unwrap(), json!({ "x": 1 }))
                .with_locals(scope.clone()),
        )
        .await
        .unwrap();
    assert_eq!(resp.locals_scope.as_ref(), Some(&scope));
}
