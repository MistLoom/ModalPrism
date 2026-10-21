use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use serde_json::Value;
use strata_shared::normalize_args;

use crate::env::RunnerEnv;
use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        Ok(Arc::new(RunnerAbility {
            manifest: manifest::manifest(),
            env: Arc::new(RunnerEnv { bind }),
        }) as Arc<dyn Ability>)
    })
}

struct RunnerAbility {
    manifest: AbilityManifest,
    env: Arc<RunnerEnv>,
}

#[async_trait]
impl Ability for RunnerAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        let args = normalize_args(args)?;
        match method.as_str() {
            "run_task" => crate::logic::run_task::run_task(ctx, &self.env, args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
