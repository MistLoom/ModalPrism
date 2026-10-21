use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use config::ConfigStore;
use serde_json::{Value, json};

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        let store = ConfigStore::open(&bind.config_dir, bind.load_options())?;
        Ok(Arc::new(ConfigAbility {
            manifest: manifest::manifest(),
            store: Arc::new(store),
        }) as Arc<dyn Ability>)
    })
}

struct ConfigAbility {
    manifest: AbilityManifest,
    store: Arc<ConfigStore>,
}

#[async_trait]
impl Ability for ConfigAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        _ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        match method.as_str() {
            "load" => {
                self.store.reload()?;
                Ok(json!({ "generation": self.store.snapshot().generation }))
            }
            "generation" => Ok(json!({ "generation": self.store.snapshot().generation })),
            "applied" => {
                let _ = args;
                let snap = self.store.applied();
                Ok(json!({
                    "generation": snap.generation,
                    "system": snap.system,
                    "modules": snap.modules,
                }))
            }
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
