mod common;

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    Ability, AbilityManifest, AbilityProvider, CtxFnAbility, InstanceId, LoadSpec, MethodId,
    SourceId, StaticProvider,
};
use frame_core::{Gateway, InvokeRequest};
use serde_json::json;

struct EntryProvider;

#[async_trait::async_trait]
impl AbilityProvider for EntryProvider {
    fn manifest(&self) -> AbilityManifest {
        common::manifest("entry", &["boot"])
    }

    async fn create(
        &self,
        _config: serde_json::Value,
    ) -> ability_support::Result<Arc<dyn Ability>> {
        let m = common::manifest("entry", &["boot"]);
        Ok(Arc::new(CtxFnAbility::new(m, |ctx, _| {
            Box::pin(async move {
                ctx.load(LoadSpec::new(
                    SourceId::new("echo").unwrap(),
                    InstanceId::new("echo").unwrap(),
                ))
                .await?;
                Ok(json!({ "booted": true }))
            })
        })))
    }
}

#[tokio::test]
async fn empty_host_entry_loads_other() {
    let host = AbilityHost::new();
    host.install_str("entry", Arc::new(EntryProvider))
        .await
        .unwrap();
    host.install_str("echo", common::echo_provider("echo"))
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("entry").unwrap(),
        InstanceId::new("entry").unwrap(),
    ))
    .await
    .unwrap();

    let gw = host.clone().into_gateway();
    gw.invoke(InvokeRequest::new(
        InstanceId::new("entry").unwrap(),
        MethodId::new("boot").unwrap(),
        json!({}),
    ))
    .await
    .unwrap();

    assert!(host.catalog().iter().any(|e| e.instance.as_str() == "echo"));
}
