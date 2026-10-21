use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ability_support::{AbilityCtx, Error, Result as AbilityResult};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{
    BoundEgress, BoundPaid, EgressGw, PaidGw, call_policy_from, policy_from,
};
use collector_runtime::execute::{self, FetchEnv, FollowWork};
use collector_runtime::plans::plans_of;
use collector_runtime::stage::DiskStaging;
use planner::{Handler, Planner, Receipts};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_client::Strata;
use strata_model::{
    AccessTask, ErrorClass, FacetKind, FacetRef, Locator, TaskReason, Timestamp, VersionSel,
};
use strata_shared::parse;
use strata_shared::persist::{Hot, StrataQueue};

use crate::logic::snapshot::from_ctx;

pub struct RunEnv {
    pub bind: BindConfig,
    pub egress: Arc<EgressGw>,
    pub paid: Arc<PaidGw>,
    pub strata: Arc<Strata>,
}

pub async fn run(
    ctx: &dyn AbilityCtx,
    planner: &Planner<StrataQueue, Hot>,
    env: &RunEnv,
    args: &Value,
) -> AbilityResult<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        task: AccessTask,
    }
    let args: Args = parse(args)?;
    let snapshot = from_ctx(ctx).await?;
    let plans = plans_of(&snapshot);
    let Some(mut plan) = plans
        .into_iter()
        .find(|plan| plan.source_id == args.task.source_id)
    else {
        let retry_at = Timestamp::now();
        let nack = ability_support::MethodId::new("nack").expect("method id");
        ctx.call(
            "strata_queue",
            &nack,
            json!({
                "taskId": args.task.task_id.to_string(),
                "retryAt": retry_at.to_string(),
                "error": "config",
            }),
        )
        .await?;
        return Ok(json!({ "nacked": "config" }));
    };
    plan.generation = args.task.generation;
    let now = Timestamp::now();
    let deadline = now
        .checked_add(plan.timeout)
        .map_err(|source| Error::failed(source.to_string()))?;
    let handler = TaskHandler {
        snapshot,
        data_dir: env.bind.data_dir.clone(),
        proxy: env
            .bind
            .proxy_url
            .clone()
            .unwrap_or_default(),
        browser: env
            .bind
            .browser_program
            .clone()
            .unwrap_or_else(|| PathBuf::from("moli")),
        egress: Arc::clone(&env.egress),
        paid: Arc::clone(&env.paid),
        strata: Arc::clone(&env.strata),
        timeout: plan.timeout,
    };
    let receipts = CtxReceipts { ctx };
    let follows = planner
        .run(&plan, args.task, &handler, &receipts, now, deadline)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    let follows = serde_json::to_value(&follows).unwrap_or(Value::Array(Vec::new()));
    Ok(json!({ "ok": true, "follows": follows }))
}

struct TaskHandler {
    snapshot: Arc<config::Snapshot>,
    data_dir: PathBuf,
    proxy: String,
    browser: PathBuf,
    egress: Arc<EgressGw>,
    paid: Arc<PaidGw>,
    strata: Arc<Strata>,
    timeout: Duration,
}

#[async_trait]
impl Handler for TaskHandler {
    async fn execute(
        &self,
        task: &AccessTask,
        cursor: Option<Value>,
    ) -> std::result::Result<planner::TaskSuccess, planner::Classified> {
        let resource = if task.reason == TaskReason::Follow {
            let key = match &task.locator {
                Locator::Url { url } => format!("url:{url}"),
                _ => String::new(),
            };
            if key.is_empty() {
                None
            } else {
                self.strata.resolve(&key).await.ok().flatten()
            }
        } else {
            None
        };
        let env = FetchEnv {
            snapshot: Arc::clone(&self.snapshot),
            staging_dir: self.data_dir.join("staging"),
            proxy: self.proxy.clone(),
            browser: self.browser.clone(),
            timeout: self.timeout,
        };
        let module = env
            .snapshot
            .modules
            .get(task.source_id.as_str())
            .cloned()
            .ok_or_else(|| classified(ErrorClass::Config, "config"))?;
        let accessor_id = if task.reason == TaskReason::Follow {
            task.accessor.as_str()
        } else {
            module
                .pointer("/access/accessor")
                .and_then(Value::as_str)
                .unwrap_or(task.accessor.as_str())
        };
        let implementation = env
            .snapshot
            .modules
            .get(accessor_id)
            .and_then(|item| item.get("implementation"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let accessor = execute::open_accessor(implementation, &env.browser, &env.proxy)
            .map_err(|error| classified(ErrorClass::Config, error.message))?;
        let source_policy = module
            .get("policy")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let accessor_policy = env
            .snapshot
            .modules
            .get(accessor_id)
            .and_then(|item| item.get("policy"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let egress_policy = policy_from(&env.snapshot.system, &source_policy);
        let paid_policy = call_policy_from(&env.snapshot.system, &accessor_policy);
        let access = accessors::AccessContext {
            egress: Arc::new(BoundEgress {
                inner: Arc::clone(&self.egress),
                policy: egress_policy.clone(),
            }),
            paid: Arc::new(BoundPaid {
                inner: Arc::clone(&self.paid),
                policy: paid_policy,
            }),
            sessions: Arc::new(execute::NoSession),
            staging: Arc::new(DiskStaging {
                dir: self.data_dir.join("staging"),
            }),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let fetched = execute::fetch(
            &env,
            &self.paid,
            accessor,
            &access,
            egress_policy,
            task,
            cursor,
            resource,
        )
        .await?;
        let mut success = fetched.success;
        success.follows = open_follows(&self.strata, fetched.follows, fetched.max_fetches).await;
        Ok(success)
    }
}

struct CtxReceipts<'a> {
    ctx: &'a dyn AbilityCtx,
}

#[async_trait]
impl Receipts for CtxReceipts<'_> {
    async fn complete(&self, receipt_id: &str) -> std::result::Result<(), planner::PlannerError> {
        let method = ability_support::MethodId::new("complete").expect("method id");
        self.ctx
            .call(
                "paid_gateway",
                &method,
                json!({ "receiptId": receipt_id }),
            )
            .await
            .map_err(|error| planner::PlannerError::new(error.to_string()))?;
        Ok(())
    }
}

fn classified(class: ErrorClass, message: impl Into<String>) -> planner::Classified {
    planner::Classified {
        class,
        message: message.into(),
        retry_after: None,
        retry_at: None,
    }
}

async fn open_follows(
    strata: &Strata,
    works: Vec<FollowWork>,
    max_fetches: usize,
) -> Vec<AccessTask> {
    let mut out = Vec::new();
    for work in works {
        if out.len() >= max_fetches {
            break;
        }
        if let Some(rid) = strata.resolve(&work.identity).await.ok().flatten() {
            let reference = FacetRef::new(
                rid,
                FacetKind::Struct,
                Some("page".into()),
                VersionSel::Latest,
            );
            if let Ok(reference) = reference {
                if let Ok(Some(facet)) = strata.get(&reference, true).await {
                    if needs_met(facet.value.as_ref(), &work.needs) {
                        continue;
                    }
                }
            }
        }
        out.push(work.task);
    }
    out
}

fn needs_met(value: Option<&Value>, needs: &[String]) -> bool {
    let Some(value) = value else {
        return false;
    };
    needs.iter().all(|need| {
        value.get(need).is_some_and(|item| match item {
            Value::String(text) => !text.is_empty(),
            Value::Null => false,
            _ => true,
        })
    })
}
