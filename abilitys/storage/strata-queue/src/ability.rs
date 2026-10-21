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
        Ok(Arc::new(StrataQueueAbility {
            manifest: manifest::manifest(),
            strata,
        }) as Arc<dyn Ability>)
    })
}

struct StrataQueueAbility {
    manifest: AbilityManifest,
    strata: Arc<Strata>,
}

#[async_trait]
impl Ability for StrataQueueAbility {
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
            "enqueue" => crate::logic::enqueue::run(&self.strata, &args).await,
            "lease" => crate::logic::lease::run(&self.strata, &args).await,
            "extend" => crate::logic::extend::run(&self.strata, &args).await,
            "nack" => crate::logic::nack::run(&self.strata, &args).await,
            "lease_id" => crate::logic::lease_id::run(&self.strata, &args).await,
            "release_leases" => crate::logic::release_leases::run(&self.strata, &args).await,
            "active" => crate::logic::active::run(&self.strata, &args).await,
            "lease_ready" => crate::logic::lease_ready::run(&self.strata, &args).await,
            "lease_follows" => crate::logic::lease_follows::run(&self.strata, &args).await,
            "leased_count" => crate::logic::leased_count::run(&self.strata, &args).await,
            "apply" => crate::logic::apply::run(&self.strata, &args).await,
            "accepted_cursor" => crate::logic::accepted_cursor::run(&self.strata, &args).await,
            "visible_cursor" => crate::logic::visible_cursor::run(&self.strata, &args).await,
            "hot_load" => crate::logic::hot::load(&self.strata, &args).await,
            "hot_store" => crate::logic::hot::store(&self.strata, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
