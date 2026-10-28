//! catalog + tool_calls Ability ??

use ability_core::AbilityHost;
use ability_support::{
    Ability, AbilityId, AbilityManifest, FnAbility, InstanceId, LoadSpec, MethodId, MethodMeta,
    SourceId, StaticProvider,
};
use ai_protocol::{FunctionCall, ToolCall, ToolCallKind};
use serde_json::json;

#[tokio::test]
async fn catalog_and_dispatch() {
    let add_meta = AbilityManifest {
        id: AbilityId::new("algo.add").unwrap(),
        depends: vec![],
        stateful: false,
        config_schema: json!(null),
        methods: vec![MethodMeta {
            id: MethodId::new("run").unwrap(),
            description: "a + b".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                },
                "required": ["a", "b"]
            }),
            output_schema: None,
        }],
        events: vec![],
    };
    let add: std::sync::Arc<dyn Ability> =
        std::sync::Arc::new(FnAbility::new(add_meta, |args| async move {
            let a = args["a"].as_f64().unwrap_or(0.0);
            let b = args["b"].as_f64().unwrap_or(0.0);
            Ok(json!({ "sum": a + b }))
        }));

    let host = AbilityHost::new();
    host.install_str("algo.add", std::sync::Arc::new(StaticProvider::new(add)))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("algo.add").unwrap(),
        InstanceId::new("algo.add").unwrap(),
    ))
    .await
    .unwrap();
    host.set_protocol_support(
        "algo.add",
        vec![
            frame_core::AccessKind::Tools,
            frame_core::AccessKind::Http,
            frame_core::AccessKind::Fun,
        ],
    );
    host.install_str("dispatch", ai_dispatch::provider())
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("dispatch").unwrap(),
        InstanceId::new("dispatch").unwrap(),
    ))
    .await
    .unwrap();

    let gw = host.into_gateway();
    let defs = gw
        .invoke(frame_core::InvokeRequest::new(
            InstanceId::new("dispatch").unwrap(),
            MethodId::new("defs").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    let tools = defs.data["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["function"]["name"] == "algo_add"));

    let calls = vec![ToolCall {
        id: "call_1".into(),
        kind: ToolCallKind::Function,
        function: FunctionCall {
            name: "algo_add".into(),
            arguments: r#"{"a":1,"b":2}"#.into(),
        },
    }];
    let run = gw
        .invoke(frame_core::InvokeRequest::new(
            InstanceId::new("dispatch").unwrap(),
            MethodId::new("run").unwrap(),
            json!({ "calls": calls }),
        ))
        .await
        .unwrap();
    let msgs = run.data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["tool_call_id"], "call_1");
    let body = msgs[0]["content"].as_str().unwrap();
    assert!(body.contains("3"), "{body}");
}
