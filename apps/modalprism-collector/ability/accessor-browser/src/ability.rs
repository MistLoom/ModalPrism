use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use serde_json::Value;

use crate::generated::manifest;

const BUILTIN: &str = "browser";

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        Ok(Arc::new(AccessorAbility {
            manifest: manifest::manifest(),
            bind,
        }) as Arc<dyn Ability>)
    })
}

struct AccessorAbility {
    manifest: AbilityManifest,
    bind: BindConfig,
}

#[async_trait]
impl Ability for AccessorAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        match method.as_str() {
            "validate" => collector_accessor_shared::validate(&self.bind, BUILTIN, args).await,
            "access" => collector_accessor_shared::access(ctx, &self.bind, BUILTIN, args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
