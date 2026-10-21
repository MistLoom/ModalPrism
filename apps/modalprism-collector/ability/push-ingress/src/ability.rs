use std::sync::Arc;
use std::time::Duration;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, Lifecycle, MethodId,
    Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{call_policy_from, policy_of};
use collector_runtime::{PushCommit, execute, serve_push};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use ability_collector_runner::access::AbilityAccess;

use crate::generated::manifest;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        Ok(Arc::new(PushIngressAbility {
            manifest: manifest::manifest(),
            bind,
            listener: Mutex::new(None),
            ctx: Mutex::new(None),
        }) as Arc<dyn Ability>)
    })
}

struct PushIngressAbility {
    manifest: AbilityManifest,
    bind: BindConfig,
    listener: Mutex<Option<JoinHandle<()>>>,
    ctx: Mutex<Option<Arc<dyn AbilityCtx>>>,
}

struct CtxPush {
    ctx: Arc<dyn AbilityCtx>,
    bind: BindConfig,
    push: accessors_builtin::PushAccessor,
    snapshot: Arc<config::Snapshot>,
}

#[async_trait]
impl PushCommit for CtxPush {
    fn applied_snapshot(&self) -> Arc<config::Snapshot> {
        Arc::clone(&self.snapshot)
    }

    fn access_context(&self) -> accessors::AccessContext {
        let policy = policy_of(&self.bind.system);
        let paid = call_policy_from(&self.bind.system, &Value::Null);
        AbilityAccess::build(
            Arc::clone(&self.ctx),
            policy,
            paid,
            self.bind.data_dir.join("staging"),
        )
    }

    fn push_accessor(&self) -> &accessors_builtin::PushAccessor {
        &self.push
    }

    async fn commit_inbound(
        &self,
        source_id: &str,
        envelopes: &[strata_model::RawEnvelope],
    ) -> std::result::Result<(), collector_runtime::Error> {
        let applied = collector_ports::applied(self.ctx.as_ref())
            .await
            .map_err(|e| collector_runtime::Error::new(e.to_string()))?;
        let module = applied
            .modules
            .get(source_id)
            .cloned()
            .ok_or_else(|| collector_runtime::Error::new("config"))?;
        let env = execute::FetchEnv {
            snapshot: applied,
            staging_dir: self.bind.data_dir.join("staging"),
            proxy: self.bind.proxy_url.clone().unwrap_or_default(),
            browser: self
                .bind
                .browser_program
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from("moli")),
            timeout: Duration::from_secs(30),
        };
        let ops = execute::inbound_ops(&env, source_id, &module, envelopes)
            .await
            .map_err(|e| collector_runtime::Error::new(e.message))?;
        if ops.is_empty() {
            return Ok(());
        }
        let method = ability_support::MethodId::new("apply").expect("method");
        self.ctx
            .call(
                "strata_write",
                &method,
                json!({
                    "changeset": {
                        "producer": {
                            "module": "push",
                            "version": "1",
                            "configGeneration": env.snapshot.generation,
                        },
                        "ops": ops,
                    }
                }),
            )
            .await
            .map_err(|e| collector_runtime::Error::new(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl Ability for PushIngressAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        match phase {
            Lifecycle::Started => {
                *self.ctx.lock().await = Some(ctx.retain());
                let Some(addr) = self.bind.push_listen.as_deref() else {
                    return Ok(());
                };
                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .map_err(|e| Error::failed(e.to_string()))?;
                let ctx = self.ctx.lock().await.clone().expect("ctx");
                let snapshot = collector_ports::applied(ctx.as_ref()).await?;
                let bind = self.bind.clone();
                let services = Arc::new(CtxPush {
                    ctx,
                    bind,
                    push: accessors_builtin::PushAccessor::new(),
                    snapshot,
                });
                let task = tokio::spawn(async move {
                    serve_push(services, listener).await;
                });
                *self.listener.lock().await = Some(task);
            }
            Lifecycle::Stopped => {
                if let Some(task) = self.listener.lock().await.take() {
                    task.abort();
                }
                *self.ctx.lock().await = None;
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
        let _ = args;
        match method.as_str() {
            "status" => Ok(json!({ "listen": self.bind.push_listen })),
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
