mod common;

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, CtxFnAbility, InstanceId, InvokeMask, LoadSpec, MethodId, SourceId, StaticProvider,
};
use frame_core::InvokeRequest;
use serde_json::json;

#[tokio::test]
async fn invoke_mask_filters_catalog_and_call() {
    let host = AbilityHost::new();

    let echo_m = common::manifest("echo", &["run"]);
    let echo: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(echo_m, |ctx, args| {
        Box::pin(async move {
            let cat = ctx.catalog();
            let ids: Vec<_> = cat.iter().map(|d| d.instance.to_string()).collect();
            Ok(json!({ "catalog": ids, "args": args }))
        })
    }));
    host.install_str("echo", Arc::new(StaticProvider::new(echo)))
        .await
        .unwrap();

    let gate_m = common::manifest("gate", &["run"]);
    let gate: Arc<dyn Ability> = Arc::new(CtxFnAbility::new(gate_m, |ctx, _| {
        Box::pin(async move {
            let out = ctx
                .call(
                    "write_file",
                    &MethodId::new("run").unwrap(),
                    json!({ "path": "x" }),
                )
                .await;
            match out {
                Ok(v) => Ok(json!({ "called": "write_file", "data": v })),
                Err(e) => Ok(json!({ "denied": e.to_string() })),
            }
        })
    }));
    host.install_str("gate", Arc::new(StaticProvider::new(gate)))
        .await
        .unwrap();

    host.install_str("write_file", common::echo_provider("write_file"))
        .await
        .unwrap();

    for id in ["echo", "gate", "write_file"] {
        host.load(LoadSpec::new(
            SourceId::new(id).unwrap(),
            InstanceId::new(id).unwrap(),
        ))
        .await
        .unwrap();
    }

    let mask = InvokeMask::allow_only(["echo", "gate"]);
    let gw = host.clone().into_gateway();
    let out = gw
        .invoke(
            InvokeRequest::new(
                InstanceId::new("gate").unwrap(),
                MethodId::new("run").unwrap(),
                json!({}),
            )
            .with_invoke_mask(mask),
        )
        .await
        .unwrap();

    let denied = out
        .data
        .get("denied")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        denied.contains("调用链策略拒绝"),
        "expected mask denial, got {out:?}"
    );

    let echo_out = gw
        .invoke(
            InvokeRequest::new(
                InstanceId::new("echo").unwrap(),
                MethodId::new("run").unwrap(),
                json!({}),
            )
            .with_invoke_mask(InvokeMask::allow_only(["echo"])),
        )
        .await
        .unwrap();
    let cat = echo_out
        .data
        .get("catalog")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(cat.len(), 1);
    assert_eq!(cat[0].as_str(), Some("echo"));
}
