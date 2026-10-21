use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{HttpTransport, SystemResolver, call_policy_from, policy_of};
use crate::bridge::{PaidBridge, PaidGw};
use egress::{Resolver, TcpDial};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::hot::Hot;

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        Ok(Arc::new(PaidGatewayAbility {
            manifest: manifest::manifest(),
            bind,
            gateway: Mutex::new(None),
        }) as Arc<dyn Ability>)
    })
}

struct PaidGatewayAbility {
    manifest: AbilityManifest,
    bind: BindConfig,
    gateway: Mutex<Option<Arc<PaidGw>>>,
}

impl PaidGatewayAbility {
    async fn gateway(&self, ctx: &dyn AbilityCtx) -> Result<Arc<PaidGw>> {
        {
            let guard = self.gateway.lock().await;
            if let Some(gateway) = guard.as_ref() {
                return Ok(Arc::clone(gateway));
            }
        }
        let retained = ctx.retain();
        let opening = policy_of(&self.bind.system);
        let egress = Arc::new(egress::Gateway::new(
            Hot::new(Arc::clone(&retained), "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let gateway = Arc::new(paid::Gateway::new(
            Hot::new(retained, "paid"),
            PaidBridge {
                egress,
                fallback: opening,
            },
            call_policy_from(&self.bind.system, &Value::Null),
        ));
        *self.gateway.lock().await = Some(Arc::clone(&gateway));
        Ok(gateway)
    }
}

#[async_trait]
impl Ability for PaidGatewayAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        let gateway = self.gateway(ctx).await?;
        match method.as_str() {
            "reserve" => crate::logic::reserve::reserve(ctx, args).await,
            "call" => {
                let applied = ctx
                    .call(
                        "config",
                        &MethodId::new("applied").expect("method id"),
                        serde_json::json!({}),
                    )
                    .await?;
                let system = applied
                    .get("system")
                    .cloned()
                    .unwrap_or_else(|| self.bind.system.clone());
                crate::logic::call::run(&gateway, &system, &args).await
            }
            "complete" => crate::logic::complete::run(&gateway, &args).await,
            "status" => crate::logic::status::run(&gateway, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
