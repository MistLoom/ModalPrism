use std::sync::Arc;

use ability_plugin_host::{
    PluginHostConfig, PluginHostKind, PluginHostRuntime, wire_turn_complete,
};
use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, EventReg, Lifecycle, Result,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::doctor;
use super::invoke;
use super::lifecycle::store_runtime;

struct ClaudeModAbility {
    config: PluginHostConfig,
    runtime: Arc<Mutex<PluginHostRuntime>>,
    /// 持有事件登记；丢弃即卸订阅。
    wires: Mutex<Vec<EventReg>>,
}

#[async_trait]
impl Ability for ClaudeModAbility {
    fn manifest(&self) -> &AbilityManifest {
        static M: std::sync::OnceLock<AbilityManifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            AbilitySpec::must("claude_mod")
                .stateful()
                .on(
                    "doctor",
                    "检查 Mod 与 Node",
                    json!({ "type": "object" }),
                    |_c, _a| ability_support::box_fut(async { Ok(Value::Null) }),
                )
                .on(
                    "invoke",
                    "调用 Mod 钩子",
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
        if phase == Lifecycle::Stopped {
            self.wires.lock().await.clear();
            return Ok(());
        }
        if phase != Lifecycle::Started {
            return Ok(());
        }
        let regs = {
            let mut rt = self.runtime.lock().await;
            rt.start(ctx, &self.config).await?;
            let hooks = rt.hook_names_async().await;
            wire_turn_complete(ctx, &hooks).await?
        };
        *self.wires.lock().await = regs;
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
    AbilitySpec::must("claude_mod")
        .stateful()
        .on(
            "doctor",
            "检查 Mod 与 Node",
            json!({ "type": "object" }),
            |_c, _a| ability_support::box_fut(async { Ok(Value::Null) }),
        )
        .on(
            "invoke",
            "调用 Mod 钩子",
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
            Ok(Arc::new(ClaudeModAbility {
                config: cfg,
                runtime: Arc::new(Mutex::new(PluginHostRuntime::new(
                    PluginHostKind::ClaudeMod,
                ))),
                wires: Mutex::new(Vec::new()),
            }) as Arc<dyn Ability>)
        })
}
