use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{HttpTransport, SystemResolver, call_policy_from, policy_of};
use crate::bridge::{EgressGw, PaidBridge, PaidGw};
use egress::{Resolver, TcpDial};
use serde_json::Value;
use strata_shared::attach_strata;

use crate::hot::Hot;

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        let strata = attach_strata(&bind.strata_config()).await?;
        let opening = policy_of(&bind.system);
        let egress = Arc::new(egress::Gateway::new(
            Hot::new(Arc::clone(&strata), "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let gateway = Arc::new(paid::Gateway::new(
            Hot::new(strata, "paid"),
            PaidBridge {
                egress,
                fallback: opening,
            },
            call_policy_from(&bind.system, &Value::Null),
        ));
        Ok(Arc::new(PaidGatewayAbility {
            manifest: manifest::manifest(),
            gateway,
            system: bind.system,
        }) as Arc<dyn Ability>)
    })
}

struct PaidGatewayAbility {
    manifest: AbilityManifest,
    gateway: Arc<PaidGw>,
    system: Value,
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
        match method.as_str() {
            "reserve" => Err(Error::failed("reserve")),
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
                    .unwrap_or_else(|| self.system.clone());
                crate::logic::call::run(&self.gateway, &system, &args).await
            }
            "complete" => crate::logic::complete::run(&self.gateway, &args).await,
            "status" => crate::logic::status::run(&self.gateway, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
