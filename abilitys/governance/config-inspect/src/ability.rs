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
        let load_options = bind.load_options();
        let config_dir = bind.config_dir.clone();
        let store = ConfigStore::open(&bind.config_dir, load_options.clone())
            .map_err(|e| Error::failed(e.to_string()))?;
        Ok(Arc::new(InspectAbility {
            manifest: manifest::manifest(),
            config_dir,
            load_options,
            store: Arc::new(store),
        }) as Arc<dyn Ability>)
    })
}

struct InspectAbility {
    manifest: AbilityManifest,
    config_dir: std::path::PathBuf,
    load_options: config::LoadOptions,
    store: Arc<ConfigStore>,
}

#[async_trait]
impl Ability for InspectAbility {
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
            "validate" => {
                config::validate(&self.config_dir, &self.load_options)
                    .map_err(|e| Error::failed(e.to_string()))?;
                Ok(json!({ "ok": true }))
            }
            "diff" => {
                let fresh = config::validate(&self.config_dir, &self.load_options)
                    .map_err(|e| Error::failed(e.to_string()))?;
                let changes = config::diff(&self.store.snapshot(), &fresh);
                let items = changes
                    .into_iter()
                    .map(|change| {
                        json!({
                            "path": change.path,
                            "before": change.before,
                            "after": change.after,
                        })
                    })
                    .collect();
                Ok(Value::Array(items))
            }
            "explain" => {
                let module = args
                    .get("module")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::invalid_args("spec 无效"))?;
                let fields = config::explain(&self.store.snapshot(), module)
                    .map_err(|e| Error::failed(e.to_string()))?;
                let items = fields
                    .into_iter()
                    .map(|field| {
                        json!({
                            "path": field.path,
                            "value": field.value,
                            "file": field.file,
                            "line": field.line,
                            "presetId": field.preset_id,
                            "viaPreset": field.via_preset,
                            "systemScope": field.system_scope,
                        })
                    })
                    .collect();
                Ok(Value::Array(items))
            }
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
