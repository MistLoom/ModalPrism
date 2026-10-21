//! 工厂与调度

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
        Ok(Arc::new(StrataReadAbility {
            manifest: manifest::manifest(),
            strata,
        }) as Arc<dyn Ability>)
    })
}

struct StrataReadAbility {
    manifest: AbilityManifest,
    strata: Arc<Strata>,
}

#[async_trait]
impl Ability for StrataReadAbility {
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
            "kinds" => crate::logic::kinds::run(&self.strata, &args).await,
            "has" => crate::logic::has::run(&self.strata, &args).await,
            "get" => crate::logic::get::run(&self.strata, &args).await,
            "slice" => crate::logic::slice::run(&self.strata, &args).await,
            "resolve" => crate::logic::resolve::run(&self.strata, &args).await,
            "history" => crate::logic::history::run(&self.strata, &args).await,
            "lineage" => crate::logic::lineage::run(&self.strata, &args).await,
            "related" => crate::logic::related::run(&self.strata, &args).await,
            "query" => crate::logic::query::run(&self.strata, &args).await,
            "changes" => crate::logic::changes::run(&self.strata, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
