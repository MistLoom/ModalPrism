//! 监测 app 集成测试：异步记录、查询面、fun 入口。

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, FnAbility, InstanceId, LoadSpec, MethodId, SourceId, StaticProvider,
};
use frame_core::InvokeRequest;
use leaf_monitor::{MonitorConfig, attach, fun_call};
use serde_json::json;

/// 监视器 sink 是进程级单例：并行跑测试会互相顶掉注册，事件落到别的队列。
/// 这些用例都依赖"本用例的 app 收到全部事件"，因此统一串行。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn provider(id: &str) -> Arc<dyn Ability> {
    let m = crate_manifest(id);
    // 停留片刻，保证主线程能观察到在途链路。
    Arc::new(FnAbility::new(m, |_a| async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Ok(_a)
    }))
}

fn crate_manifest(id: &str) -> ability_support::AbilityManifest {
    ability_support::AbilityManifest {
        id: ability_support::AbilityId::new(id).unwrap(),
        depends: vec![],
        stateful: false,
        config_schema: json!({}),
        methods: vec![ability_support::MethodMeta::new(
            "run",
            "",
            json!({"type":"object"}),
        )],
        events: vec![],
    }
}

#[tokio::test]
async fn recorder_and_query_surfaces() {
    let _serial = SERIAL.lock().await;
    let log = std::env::temp_dir().join(format!("leaf-monitor-test-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let config = MonitorConfig {
        log_path: log.to_string_lossy().into(),
        ..MonitorConfig::default()
    };

    let host = AbilityHost::new();
    // attach 先于 load：生命周期 / host 事件也要被记录。
    let app = attach(&host, config);
    host.install_str("echo", Arc::new(StaticProvider::new(provider("echo"))))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("echo").unwrap(),
        InstanceId::new("echo").unwrap(),
    ))
    .await
    .unwrap();

    // 在途链路查询。
    let gw = host.clone().into_gateway();
    let root = {
        let gw = Arc::clone(&gw);
        tokio::spawn(async move {
            gw.invoke(InvokeRequest::new(
                InstanceId::new("echo").unwrap(),
                MethodId::new("run").unwrap(),
                json!({"x": 1}),
            ))
            .await
        })
    };
    for _ in 0..200 {
        if !app.chains().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(!app.chains().is_empty(), "监测 app 能查到在途链路");
    root.await.unwrap().unwrap();

    // 内存事件里有 invoke / lifecycle / host 记录。
    for _ in 0..100 {
        if !app.recent_events().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let kinds: Vec<String> = app.recent_events().iter().map(|e| e.kind.clone()).collect();
    assert!(
        kinds.iter().any(|k| k == "invoke"),
        "应有 invoke 事件: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "lifecycle"),
        "应有 lifecycle 事件: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "host"),
        "应有 host 事件: {kinds:?}"
    );

    // fun 入口。
    let chains = fun_call(&app, "monitor.chains", json!({})).await.unwrap();
    assert!(chains.get("chains").is_some());
    let evs = fun_call(&app, "monitor.events", json!({"tail": 10}))
        .await
        .unwrap();
    assert!(evs.get("events").is_some());

    // 异步落盘：等 worker 冲刷后尾读。
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let lines = app.tail_log(1000).unwrap();
    assert!(!lines.is_empty(), "日志应已落盘");
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert!(first.get("kind").is_some(), "JSONL 行应为 MonitorEvent");

    app.shutdown();
    let _ = std::fs::remove_file(&log);
}

#[tokio::test]
async fn queue_drops_oldest_without_blocking() {
    let _serial = SERIAL.lock().await;
    let config = MonitorConfig {
        log_path: String::new(), // 不落盘
        queue_capacity: 32,
        recent_capacity: 32,
        ..MonitorConfig::default()
    };
    let host = AbilityHost::new();
    let app = attach(&host, config);

    // 直接压入远超容量的事件（构造 sink 场景）。
    for i in 0..1000u64 {
        app.recent_events(); // 触碰接口
        ability_support::monitor::emit(ability_support::monitor::MonitorEvent::new("invoke", i));
    }
    assert_eq!(app.recent_events().len(), 32);
    assert!(app.stats().dropped > 0, "dropped={:?}", app.stats());
    app.shutdown();
}
