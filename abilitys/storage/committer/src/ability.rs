//! 工厂与调度

use std::path::PathBuf;
use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use serde_json::Value;
use strata_client::Strata;
use strata_shared::{normalize_args, open_strata, OpenConfig};

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let cfg: OpenConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::invalid_args(e.to_string()))?;
        let strata = open_strata(&config).await?;
        Ok(Arc::new(CommitterAbility {
            manifest: manifest::manifest(),
            data_dir: cfg.data_dir,
            strata,
        }) as Arc<dyn Ability>)
    })
}

struct CommitterAbility {
    manifest: AbilityManifest,
    data_dir: PathBuf,
    strata: Arc<Strata>,
}

#[async_trait]
impl Ability for CommitterAbility {
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
            "flush" => crate::logic::flush::run(&self.strata, &args).await,
            "maintain" => crate::logic::maintain::run(&self.data_dir, &self.strata, &args).await,
            "recover" => crate::logic::recover::run(&self.strata, &args).await,
            "status" => crate::logic::status::run(&self.data_dir, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
