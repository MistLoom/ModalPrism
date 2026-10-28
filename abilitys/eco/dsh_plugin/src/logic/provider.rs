use std::sync::Arc;

use ability_plugin_host::{PluginHostConfig, PluginHostKind, PluginHostRuntime};
use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Lifecycle, Result,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::doctor;
use super::invoke;
use super::lifecycle::store_runtime;

struct DshPluginAbility {
    config: PluginHostConfig,
    runtime: Arc<Mutex<PluginHostRuntime>>,
}

#[async_trait]
impl Ability for DshPluginAbility {
    fn manifest(&self) -> &AbilityManifest {
        static M: std::sync::OnceLock<AbilityManifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            AbilitySpec::must("dsh_plugin")
                .stateful()
                .on(
                    "doctor",
                    "检查 DSH 与 Node",
                    json!({ "type": "object" }),
                    |_c, _a| ability_support::box_fut(async { Ok(Value::Null) }),
                )
                .on(
                    "invoke",
                    "调用 DSH 钩子",
                    json!({
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string" },
                            "args": { "type": "object" }
                        },
                        "required": ["tool"]
                    }),
                    |_c, _a| ability_support::box_fut(async { Ok(Value::Null) }),
                )
                .manifest()
        })
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        if phase != Lifecycle::Started {
            return Ok(());
        }
        {
            let mut rt = self.runtime.lock().await;
            rt.start(ctx, &self.config).await?;
        }
        store_runtime(ctx.instance(), Arc::clone(&self.runtime))?;
        Ok(())
    }

    async fn invoke(
        &self,
        method: &ability_support::MethodId,
        ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        match method.as_str() {
            "doctor" => doctor::doctor(ctx, args).await,
            "invoke" => invoke::invoke(ctx, args).await,
            _ => Err(ability_support::Error::not_found(method.as_str())),
        }
    }
}

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("dsh_plugin")
        .stateful()
        .on(
            "doctor",
            "检查 DSH 与 Node",
            json!({ "type": "object" }),
            |_c, _a| ability_support::box_fut(async { Ok(Value::Null) }),
        )
        .on(
            "invoke",
            "调用 DSH 钩子",
            json!({
                "type": "object",
                "properties": {
                    "tool": { "type": "string" },
                    "args": { "type": "object" }
                },
                "required": ["tool"]
            }),
            |_c, _a| ability_support::box_fut(async { Ok(Value::Null) }),
        )
        .into_factory(|config| async move {
            let cfg: PluginHostConfig = serde_json::from_value(config)
                .map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
            Ok(Arc::new(DshPluginAbility {
                config: cfg,
                runtime: Arc::new(Mutex::new(PluginHostRuntime::new(PluginHostKind::Dsh))),
            }) as Arc<dyn Ability>)
        })
}
