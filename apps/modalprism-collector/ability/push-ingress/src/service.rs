use std::sync::Arc;
use std::time::Duration;

use ability_support::AbilityCtx;
use async_trait::async_trait;
use collector_gateway::{call_policy_from, policy_of};
use collector_runtime::{PushCommit, execute, serve_push};
use serde_json::json;

use collector_accessor_shared::AbilityAccess;

pub struct CtxPush {
    pub ctx: Arc<dyn AbilityCtx>,
    pub bind: collector_bind::BindConfig,
    pub push: accessors_builtin::PushAccessor,
    pub snapshot: Arc<config::Snapshot>,
}

#[async_trait]
impl PushCommit for CtxPush {
    fn applied_snapshot(&self) -> Arc<config::Snapshot> {
        Arc::clone(&self.snapshot)
    }

    fn access_context(&self) -> accessors::AccessContext {
        let policy = policy_of(&self.bind.system);
        let paid = call_policy_from(&self.bind.system, &serde_json::Value::Null);
        AbilityAccess::build(
            Arc::clone(&self.ctx),
            policy,
            config::overlay_system_policy(&self.bind.system, &serde_json::Value::Null),
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

pub async fn serve(bind: collector_bind::BindConfig, ctx: Arc<dyn AbilityCtx>) -> Result<(), ability_support::Error> {
    let Some(addr) = bind.push_listen.as_deref() else {
        return Ok(());
    };
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| ability_support::Error::failed(e.to_string()))?;
    let snapshot = collector_ports::applied(ctx.as_ref()).await?;
    let services = Arc::new(CtxPush {
        ctx,
        bind,
        push: accessors_builtin::PushAccessor::new(),
        snapshot,
    });
    let task = tokio::spawn(async move {
        serve_push(services, listener).await;
    });
    crate::runtime::set_listener(task).await;
    Ok(())
}
