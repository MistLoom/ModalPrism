use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, Lifecycle, MethodId,
    Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{HttpTransport, SystemResolver, policy_of};
use egress::{Resolver, TcpDial};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::hot::Hot;
use serde_json::{Value, json};
use strata_shared::{attach_strata, text_field};

use crate::generated::manifest;

pub(crate) type EgressGw = egress::Gateway<Hot, HttpTransport, SystemResolver, TcpDial>;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        let strata = attach_strata(&bind.strata_config()).await?;
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
            proxy_listen: bind.proxy_listen,
            listener: Mutex::new(None),
        }) as Arc<dyn Ability>)
    })
}

struct EgressAbility {
    manifest: AbilityManifest,
    gateway: Arc<EgressGw>,
    proxy_url: String,
    system: Value,
    opening: egress::Policy,
    proxy_listen: Option<String>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

#[async_trait]
impl Ability for EgressAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, _ctx: &dyn AbilityCtx) -> Result<()> {
        match phase {
            Lifecycle::Started => {
                let Some(addr) = self.proxy_listen.as_deref() else {
                    return Ok(());
                };
                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .map_err(|e| Error::failed(e.to_string()))?;
                let gateway = Arc::clone(&self.gateway);
                let system = self.system.clone();
                let task = tokio::spawn(async move {
                    egress::serve_each(&gateway, listener, move || policy_of(&system)).await;
                });
                *self.listener.lock().await = Some(task);
            }
            Lifecycle::Stopped => {
                if let Some(task) = self.listener.lock().await.take() {
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
        ctx: &dyn AbilityCtx,
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
            "governance_domains" => {
                crate::logic::governance::domains(&self.gateway, &self.proxy_url).await
            }
            "reset_circuit" => {
                let host = text_field(&args, "host")?;
                let confirm = args
                    .get("confirm")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !confirm {
                    return Err(Error::invalid_args("需要 confirm: true"));
                }
                crate::logic::governance::reset_circuit(&self.gateway, &host).await
            }
            "http_request" => {
                let applied = ctx
                    .call(
                        "config",
                        &MethodId::new("applied").expect("method id"),
                        json!({}),
                    )
                    .await?;
                let system = applied
                    .get("system")
                    .cloned()
                    .unwrap_or_else(|| self.system.clone());
                crate::logic::http_request::run(
                    &self.gateway,
                    &system,
                    policy_of(&system),
                    &args,
                )
                .await
            }
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
