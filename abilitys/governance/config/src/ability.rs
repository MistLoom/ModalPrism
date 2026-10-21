use std::sync::Arc;
use std::time::Duration;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, Lifecycle, MethodId,
    Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use config::{ConfigStore, watch};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        let store = ConfigStore::open(&bind.config_dir, bind.load_options())
            .map_err(|e| Error::failed(e.to_string()))?;
        Ok(Arc::new(ConfigAbility {
            manifest: manifest::manifest(),
            store: Arc::new(store),
            bind,
            watch: Mutex::new(None),
        }) as Arc<dyn Ability>)
    })
}

struct ConfigAbility {
    manifest: AbilityManifest,
    store: Arc<ConfigStore>,
    bind: BindConfig,
    watch: Mutex<Option<JoinHandle<()>>>,
}

#[async_trait]
impl Ability for ConfigAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, _ctx: &dyn AbilityCtx) -> Result<()> {
        match phase {
            Lifecycle::Started => {
                let store = Arc::clone(&self.store);
                let handle = tokio::spawn(async move {
                    let _ = watch(store, Duration::from_secs(2));
                });
                *self.watch.lock().await = Some(handle);
            }
            Lifecycle::Stopped => {
                if let Some(task) = self.watch.lock().await.take() {
                    task.abort();
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn invoke(
        &self,
        method: &MethodId,
        _ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        match method.as_str() {
            "load" => {
                self.store
                    .reload()
                    .map_err(|e| Error::failed(e.to_string()))?;
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
            "layout" => Ok(json!({
                "dataDir": self.bind.data_dir,
                "configDir": self.bind.config_dir,
            })),
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
