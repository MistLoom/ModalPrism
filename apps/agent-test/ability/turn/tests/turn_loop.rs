//! turn 回合循环集成测试（mock chat + echo）。

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, AbilityManifest, FnAbility, InstanceId, LoadSpec, MethodId, MethodMeta, SourceId,
    StaticProvider,
};
use ai_protocol::ToolCallKind;
use frame_core::InvokeRequest;
use serde_json::json;

fn mock_chat_provider() -> Arc<dyn Ability> {
    let manifest = AbilityManifest {
        id: ability_support::AbilityId::new("chat").unwrap(),
        depends: vec![],
        stateful: false,
        config_schema: json!(null),
        methods: vec![MethodMeta {
            id: MethodId::new("completion").unwrap(),
            description: "mock".into(),
            input_schema: json!({ "type": "object" }),
            output_schema: None,
        }],
        events: vec![],
    };
    let state = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Arc::new(FnAbility::new(manifest, move |args| {
        let state = Arc::clone(&state);
        async move {
            let n = state.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n == 0 {
                let tools = args.get("tools").and_then(|t| t.as_array()).cloned();
                let has_tools = tools.as_ref().is_some_and(|t| !t.is_empty());
                if has_tools {
                    let name = tools.unwrap()[0]["function"]["name"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    return Ok(json!({
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": null,
                                "tool_calls": [{
                                    "id": "c1",
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": "{}"
                                    }
                                }]
                            }
                        }]
                    }));
                }
            }
            Ok(json!({
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "done"
                    }
                }]
            }))
        }
    }))
}

#[tokio::test]
async fn turn_loop_calls_tool_then_stops() {
    let echo_meta = AbilityManifest {
        id: ability_support::AbilityId::new("algo.echo").unwrap(),
        depends: vec![],
        stateful: false,
        config_schema: json!(null),
        methods: vec![MethodMeta {
            id: MethodId::new("run").unwrap(),
            description: "echo".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } }
            }),
            output_schema: None,
        }],
        events: vec![],
    };
    let echo: Arc<dyn Ability> = Arc::new(FnAbility::new(echo_meta, |args| async move {
        Ok(json!({ "text": args.get("text").and_then(|v| v.as_str()).unwrap_or("") }))
    }));

    let host = AbilityHost::new();
    host.install_str("session", ai_session::provider())
        .await
        .unwrap();
    host.install_str("dispatch", ai_dispatch::provider())
        .await
        .unwrap();
    host.install_str("chat", Arc::new(StaticProvider::new(mock_chat_provider())))
        .await
        .unwrap();
    host.install_str("algo.echo", Arc::new(StaticProvider::new(echo)))
        .await
        .unwrap();
    host.install_str("turn", ai_turn::provider()).await.unwrap();

    for id in ["session", "dispatch", "chat", "algo.echo"] {
        host.load(LoadSpec::new(
            SourceId::new(id).unwrap(),
            InstanceId::new(id).unwrap(),
        ))
        .await
        .unwrap();
    }
    host.set_protocol_support(
        "algo.echo",
        vec![
            frame_core::AccessKind::Tools,
            frame_core::AccessKind::Http,
            frame_core::AccessKind::Fun,
        ],
    );
    host.load(
        LoadSpec::new(
            SourceId::new("turn").unwrap(),
            InstanceId::new("turn").unwrap(),
        )
        .with_config(json!({ "max_rounds": 8 })),
    )
    .await
    .unwrap();

    let gw = host.into_gateway();
    let out = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("turn").unwrap(),
            MethodId::new("run").unwrap(),
            json!({
                "text": "please echo",
                "create_session": true
            }),
        ))
        .await
        .unwrap();

    assert_eq!(out.data["reason"], "stopped");
    assert_eq!(out.data["text"], "done");
    assert!(out.data["rounds"].as_u64().unwrap() >= 2);

    let sid = out.data["session_id"].as_str().unwrap();
    let msgs = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("session").unwrap(),
            MethodId::new("protocol_messages").unwrap(),
            json!({ "id": sid }),
        ))
        .await
        .unwrap();
    let arr = msgs.data.as_array().unwrap();
    assert!(arr.len() >= 4, "{arr:?}");
}
