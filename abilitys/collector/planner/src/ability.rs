use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Error, MethodId, Result,
};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{
    HttpTransport, PaidBridge, PaidGw, EgressGw, SystemResolver, call_policy_from, policy_of,
};
use egress::{Resolver, TcpDial};
use planner::Planner;
use serde_json::Value;
use strata_shared::{normalize_args, open_strata, persist::{Hot, StrataQueue}};

use crate::generated::manifest;
use crate::logic::run_task::RunEnv;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest()).into_factory(|config| async move {
        let bind = BindConfig::parse(&config)?;
        let strata = open_strata(&bind.strata_config()).await?;
        let opening = policy_of(&bind.system);
        let egress = Arc::new(egress::Gateway::new(
            Hot::new(Arc::clone(&strata), "egress"),
            HttpTransport,
            SystemResolver,
            TcpDial,
            opening.clone(),
        ));
        let paid = Arc::new(paid::Gateway::new(
            Hot::new(Arc::clone(&strata), "paid"),
            PaidBridge {
                egress: Arc::clone(&egress),
                fallback: opening,
            },
            call_policy_from(&bind.system, &Value::Null),
        ));
        let planner = Planner::new(
            StrataQueue(Arc::clone(&strata)),
            Hot::new(Arc::clone(&strata), "planner"),
        );
        let run_env = RunEnv {
            bind: bind.clone(),
            egress,
            paid,
            strata,
        };
        Ok(Arc::new(PlannerAbility {
            manifest: manifest::manifest(),
            planner,
            run_env,
        }) as Arc<dyn Ability>)
    })
}

struct PlannerAbility {
    manifest: AbilityManifest,
    planner: Planner<StrataQueue, Hot>,
    run_env: RunEnv,
}

#[async_trait]
impl Ability for PlannerAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn invoke(
        &self,
        method: &MethodId,
        ctx: &dyn AbilityCtx,
        args: Value,
    ) -> Result<Value> {
        let args = normalize_args(args)?;
        match method.as_str() {
            "tick" => crate::logic::tick::run(ctx, &self.planner).await,
            "trigger" => crate::logic::trigger::run(ctx, &self.planner, &args).await,
            "next" => crate::logic::next::run(&self.planner, &args).await,
            "run_task" => crate::logic::run_task::run(ctx, &self.planner, &self.run_env, &args).await,
            _ => Err(Error::unavailable("方法不存在")),
        }
    }
}
