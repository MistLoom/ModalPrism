use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::{attach_strata, normalize_args};

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let strata = attach_strata(&config).await?;
        Ok(Arc::new(ControlStateAbility {
            manifest: manifest::manifest(),
            strata,
        }) as Arc<dyn Ability>)
    })
}

struct ControlStateAbility {
    manifest: AbilityManifest,
    strata: Arc<Strata>,
}

#[async_trait]
impl Ability for ControlStateAbility {
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
            "load" => crate::logic::load::run(&self.strata, &args).await,
            "store" => crate::logic::store::run(&self.strata, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
