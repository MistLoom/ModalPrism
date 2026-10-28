//! C1 嵌套链语义：子调用返回不拆父链，根返回才结束。

mod common;

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, AbilityId, CtxFnAbility, Dependency, InstanceId, LoadSpec, MethodId, SourceId,
    StaticProvider,
};
use frame_core::InvokeRequest;
use serde_json::json;

#[tokio::test]
async fn nested_call_keeps_root_chain_until_root_returns() {
    let host = AbilityHost::new();

    // child: run 回显。
    let child_m = common::manifest("child", &["run"]);
    let child: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(child_m, |_ctx, args| {
        Box::pin(async move {
            // 子调用在途时同链可见（查到的是根记录，span 保持 0）。
            let st = _ctx.chain_status(None).expect("child sees chain");
            assert_eq!(st.span, 0, "链记录保持根 span");
            // 停留一会儿，保证测试主线程能观察到在途链。
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            Ok(args)
        })
    }));
    host.install_str("child", Arc::new(StaticProvider::new(child)))
        .await
        .unwrap();

    // parent: run 内部 ctx.call child.run。
    let mut parent_m = common::manifest("parent", &["run"]);
    parent_m.depends = vec![Dependency::required(AbilityId::new("child").unwrap())];
    let parent: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(parent_m, |ctx, _| {
        Box::pin(async move {
            let st = ctx.chain_status(None).expect("parent sees chain");
            assert_eq!(st.span, 0);
            ctx.call(
                "child",
                &MethodId::new("run").unwrap(),
                json!({ "from": "parent" }),
            )
            .await
        })
    }));
    host.install_str("parent", Arc::new(StaticProvider::new(parent)))
        .await
        .unwrap();

    host.load(LoadSpec::new(
        SourceId::new("child").unwrap(),
        InstanceId::new("child").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("parent").unwrap(),
        InstanceId::new("parent").unwrap(),
    ))
    .await
    .unwrap();

    // 根调用进行中：链路在途。
    let gw = host.clone().into_gateway();
    let root = {
        let gw = Arc::clone(&gw);
        tokio::spawn(async move {
            gw.invoke(InvokeRequest::new(
                InstanceId::new("parent").unwrap(),
                MethodId::new("run").unwrap(),
                json!({}),
            ))
            .await
        })
    };
    // 等根链登记。
    for _ in 0..200 {
        if !host.chains_snapshot().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let snap = host.chains_snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].span, 0, "链记录保持根 span");
    assert!(snap[0].running);

    root.await.unwrap().unwrap();
    // 根返回后链路结束。
    assert!(
        host.chains_snapshot().is_empty(),
        "根返回才拆链，且子返回未拆"
    );
}

#[cfg(feature = "monitor")]
#[tokio::test]
async fn nested_invoke_emits_parent_span_ids() {
    use ability_support::monitor::{MonitorEvent, MonitorSink};
    use std::sync::Mutex;

    struct Collect(Mutex<Vec<MonitorEvent>>);
    impl MonitorSink for Collect {
        fn record(&self, event: MonitorEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    // Global sink is process-wide; clear first and prefer --test-threads=1 with other monitor tests.
    ability_support::monitor::clear_monitor_sink();
    let sink = Arc::new(Collect(Mutex::new(Vec::new())));
    ability_support::monitor::set_monitor_sink(sink.clone() as Arc<dyn MonitorSink>);

    let host = AbilityHost::new();
    let child_m = common::manifest("child", &["run"]);
    let child: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(child_m, |_ctx, args| {
        Box::pin(async move { Ok(args) })
    }));
    host.install_str("child", Arc::new(StaticProvider::new(child)))
        .await
        .unwrap();

    let mut parent_m = common::manifest("parent", &["run"]);
    parent_m.depends = vec![Dependency::required(AbilityId::new("child").unwrap())];
    let parent: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(parent_m, |ctx, _| {
        Box::pin(async move {
            ctx.call(
                "child",
                &MethodId::new("run").unwrap(),
                json!({ "from": "parent" }),
            )
            .await
        })
    }));
    host.install_str("parent", Arc::new(StaticProvider::new(parent)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("child").unwrap(),
        InstanceId::new("child").unwrap(),
    ))
    .await
    .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("parent").unwrap(),
        InstanceId::new("parent").unwrap(),
    ))
    .await
    .unwrap();

    host.clone()
        .into_gateway()
        .invoke(InvokeRequest::new(
            InstanceId::new("parent").unwrap(),
            MethodId::new("run").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();

    let evs = sink.0.lock().unwrap().clone();
    ability_support::monitor::clear_monitor_sink();

    // Pair parent/child on the same chain_id (ignore noise from concurrent tests).
    let parent_before = evs
        .iter()
        .rev()
        .find(|e| {
            e.kind == "invoke"
                && e.instance.as_deref() == Some("parent")
                && e.phase.as_deref() == Some("before")
                && e.parent_span_id.is_none()
        })
        .expect("parent before")
        .clone();
    let chain = parent_before.chain_id.clone().expect("parent chain_id");
    let child_before = evs
        .iter()
        .rev()
        .find(|e| {
            e.kind == "invoke"
                && e.instance.as_deref() == Some("child")
                && e.phase.as_deref() == Some("before")
                && e.chain_id.as_deref() == Some(chain.as_str())
        })
        .expect("child before");
    assert!(parent_before.span_id.is_some());
    assert!(parent_before.parent_span_id.is_none());
    assert_eq!(child_before.parent_span_id, parent_before.span_id);
    assert_eq!(child_before.span, Some(1));
    let parent_after = evs
        .iter()
        .rev()
        .find(|e| {
            e.kind == "invoke"
                && e.instance.as_deref() == Some("parent")
                && e.phase.as_deref() == Some("after")
                && e.chain_id.as_deref() == Some(chain.as_str())
        })
        .expect("parent after");
    assert!(parent_after.ended_at.is_some());
    assert_eq!(parent_after.span_id, parent_before.span_id);
}
