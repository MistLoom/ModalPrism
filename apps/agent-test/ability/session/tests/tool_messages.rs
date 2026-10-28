//! session tool 消息 append 测试。

use ability_core::AbilityHost;
use ability_support::{InstanceId, LoadSpec, MethodId, SourceId};
use ai_protocol::{FunctionCall, ToolCall, ToolCallKind};
use frame_core::InvokeRequest;
use serde_json::json;

#[tokio::test]
async fn append_assistant_with_tool_calls() {
    let host = AbilityHost::new();
    host.install_str("session", ai_session::provider())
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("session").unwrap(),
        InstanceId::new("session").unwrap(),
    ))
    .await
    .unwrap();

    let gw = host.into_gateway();
    let created = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("session").unwrap(),
            MethodId::new("create").unwrap(),
            json!({}),
        ))
        .await
        .unwrap();
    let sid = created.data["id"].as_str().unwrap();

    gw.invoke(InvokeRequest::new(
        InstanceId::new("session").unwrap(),
        MethodId::new("append_assistant").unwrap(),
        json!({
            "id": sid,
            "text": "calling tool",
            "tool_calls": [ToolCall {
                id: "t1".into(),
                kind: ToolCallKind::Function,
                function: FunctionCall {
                    name: "read_file_run".into(),
                    arguments: "{}".into(),
                },
            }],
        }),
    ))
    .await
    .unwrap();

    gw.invoke(InvokeRequest::new(
        InstanceId::new("session").unwrap(),
        MethodId::new("append_tool_results").unwrap(),
        json!({
            "id": sid,
            "messages": [{
                "role": "tool",
                "tool_call_id": "t1",
                "content": "ok"
            }],
        }),
    ))
    .await
    .unwrap();

    let msgs = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("session").unwrap(),
            MethodId::new("protocol_messages").unwrap(),
            json!({ "id": sid }),
        ))
        .await
        .unwrap();
    let arr = msgs.data.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr[0]["tool_calls"].is_array());
    assert_eq!(arr[1]["role"], "tool");
}
