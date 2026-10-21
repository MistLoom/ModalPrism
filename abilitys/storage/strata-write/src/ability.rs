//! 工厂与调度

use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::{normalize_args, open_strata};

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let strata = open_strata(&config).await?;
        Ok(Arc::new(StrataWriteAbility {
            manifest: manifest::manifest(),
            strata,
        }) as Arc<dyn Ability>)
    })
}

struct StrataWriteAbility {
    manifest: AbilityManifest,
    strata: Arc<Strata>,
}

#[async_trait]
impl Ability for StrataWriteAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        _ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        let args = normalize_args(args)?;
        match method.as_str() {
            "apply" => crate::logic::apply::run(&self.strata, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
