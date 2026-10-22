//! `stamp`：create 时读取 config.stamp，reload 后仍由新实例读配置。

use std::sync::Arc;

use ability_sdk::prelude::*;
use ability_sdk::{Ability, AbilityManifest, MethodId};
use async_trait::async_trait;

struct Stamp {
    manifest: AbilityManifest,
    stamp: String,
}

#[async_trait]
impl Ability for Stamp {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        _ctx: &dyn AbilityCtx,
        _args: Value,
    ) -> Result<Value> {
        if self.manifest.method(method).is_none() {
            return Err(Error::not_found(method));
        }
        Ok(json!({ "stamp": self.stamp }))
    }
}

pub fn stamp_provider() -> Arc<dyn AbilityProvider> {
    let spec =
        AbilitySpec::must("stamp")
            .stateful()
            .method("run", "印记", json!({ "type": "object" }));
    let manifest = spec.manifest();
    spec.into_factory(move |config| {
        let manifest = manifest.clone();
        let stamp = config
            .get("stamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        async move { Ok(Arc::new(Stamp { manifest, stamp }) as Arc<dyn Ability>) }
    })
}
