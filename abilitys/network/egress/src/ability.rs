use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{HttpTransport, SystemResolver, policy_of};
use egress::TcpDial;
use crate::hot::Hot;
use egress::Resolver;

pub(crate) type EgressGw = egress::Gateway<Hot, HttpTransport, SystemResolver, TcpDial>;
use serde_json::{Value, json};
use strata_shared::open_strata;

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        let strata = open_strata(&bind.strata_config()).await?;
        let opening = policy_of(&bind.system);
        let gateway = Arc::new(egress::Gateway::new(
            Hot::new(strata, "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        Ok(Arc::new(EgressAbility {
            manifest: manifest::manifest(),
            gateway,
            proxy_url: bind.proxy_url.unwrap_or_default(),
            system: bind.system,
            opening,
        }) as Arc<dyn Ability>)
    })
}

struct EgressAbility {
    manifest: AbilityManifest,
    gateway: Arc<EgressGw>,
    proxy_url: String,
    system: Value,
    opening: egress::Policy,
}

#[async_trait]
impl Ability for EgressAbility {
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
            "check" => {
                let raw = args
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::invalid_args("spec 无效"))?;
                let url = url::Url::parse(raw).map_err(|_| Error::failed("gateway"))?;
                let host = url.host_str().ok_or_else(|| Error::failed("gateway"))?;
                let addresses = SystemResolver
                    .lookup(host)
                    .await
                    .map_err(|e| Error::failed(e.message))?;
                let policy = policy_of(&self.system);
                let blocked = if policy.private_networks == egress::PrivateNetworks::Allow {
                    false
                } else {
                    addresses.is_empty() || addresses.iter().all(|ip| egress::is_blocked(*ip))
                };
                let rendered: Vec<String> = addresses.iter().map(ToString::to_string).collect();
                Ok(json!({ "blocked": blocked, "addresses": rendered }))
            }
            "status" => Ok(json!({ "proxy": self.proxy_url })),
            "http_request" => {
                crate::logic::http_request::run(&self.gateway, self.opening.clone(), &args).await
            }
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
