use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ability_support::{AbilityCtx, Error, MethodId, Result};
use async_trait::async_trait;
use collector_bind::BindConfig;
use collector_gateway::{call_policy_from, policy_from, EGRESS_POLICY, REQUEST_DEADLINE};
use collector_ports::{CtxPersist, CtxQueue};
use collector_runtime::execute::{self, FetchEnv, FollowWork};
use collector_runtime::plans::plans_of;
use collector_runtime::stage::DiskStaging;
use egress::Policy;
use planner::{Handler, Planner, Receipts, TaskSuccess};
use serde::Deserialize;
use serde_json::{Value, json};
use strata_model::{
    AccessTask, ErrorClass, FacetKind, FacetRef, Locator, TaskReason, Timestamp, VersionSel,
};
use strata_shared::parse;

use crate::access::AbilityAccess;
use crate::env::RunnerEnv;

pub async fn run_task(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let env = crate::runtime::env()?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        task: AccessTask,
    }
    let args: Args = parse(&args)?;
    let ctx = ctx.retain();
    let snapshot = collector_ports::applied(ctx.as_ref()).await?;
    let plans = plans_of(&snapshot);
    let Some(mut plan) = plans
        .into_iter()
        .find(|plan| plan.source_id == args.task.source_id)
    else {
        let nack = MethodId::new("nack").expect("method id");
        ctx.call(
            "strata_queue",
            &nack,
            json!({
                "taskId": args.task.task_id.to_string(),
                "retryAt": Timestamp::now().to_string(),
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
    let planner = Planner::new(
        CtxQueue::runner(crate::ctx_call::wrap(ctx.clone())),
        CtxPersist::new(crate::ctx_call::wrap(ctx.clone()), "planner"),
    );
    let handler = TaskHandler {
        ctx: ctx.clone(),
        env: Arc::clone(&env),
        snapshot,
        timeout: plan.timeout,
    };
    let receipts = CtxReceipts { ctx: ctx.clone() };
    let follows = planner
        .run(&plan, args.task, &handler, &receipts, now, deadline)
        .await
        .map_err(|e| Error::failed(e.to_string()))?;
    let follows = serde_json::to_value(&follows).unwrap_or(Value::Array(Vec::new()));
    Ok(json!({ "ok": true, "follows": follows }))
}

struct TaskHandler {
    ctx: Arc<dyn AbilityCtx>,
    env: Arc<crate::env::RunnerEnv>,
    snapshot: Arc<config::Snapshot>,
    timeout: Duration,
}

#[async_trait]
impl Handler for TaskHandler {
    async fn execute(
        &self,
        task: &AccessTask,
        cursor: Option<Value>,
    ) -> std::result::Result<TaskSuccess, planner::Classified> {
        let resource = if task.reason == TaskReason::Follow {
            let key = match &task.locator {
                Locator::Url { url } => format!("url:{url}"),
                _ => String::new(),
            };
            if key.is_empty() {
                None
            } else {
                let method = MethodId::new("resolve").expect("method id");
                let value = self
                    .ctx
                    .call("strata_read", &method, json!({ "identity": key }))
                    .await
                    .map_err(|e| classified(ErrorClass::Transient, e.to_string()))?;
                value
                    .as_str()
                    .and_then(|text| strata_model::Rid::parse(text).ok())
            }
        } else {
            None
        };
        let fetch_env = FetchEnv {
            snapshot: Arc::clone(&self.snapshot),
            staging_dir: self.env.bind.data_dir.join("staging"),
            proxy: self.env.bind.proxy_url.clone().unwrap_or_default(),
            browser: self
                .env
                .bind
                .browser_program
                .clone()
                .unwrap_or_else(|| PathBuf::from("moli")),
            timeout: self.timeout,
        };
        let module = fetch_env
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
        let implementation = fetch_env
            .snapshot
            .modules
            .get(accessor_id)
            .and_then(|item| item.get("implementation"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let source_policy = module
            .get("policy")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let accessor_policy = fetch_env
            .snapshot
            .modules
            .get(accessor_id)
            .and_then(|item| item.get("policy"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let egress_policy = policy_from(&fetch_env.snapshot.system, &source_policy);
        let paid_policy = call_policy_from(&fetch_env.snapshot.system, &accessor_policy);
        let access = AbilityAccess::build(
            self.ctx.clone(),
            egress_policy.clone(),
            source_policy,
            paid_policy,
            self.env.bind.data_dir.join("staging"),
        );
        let accessor = crate::ctx_accessor::CtxAccessor::open(self.ctx.clone(), implementation)
            .map_err(|error| classified(ErrorClass::Config, error.message))?;
        let policy = egress_policy.clone();
        let fetched = REQUEST_DEADLINE
            .scope(
                std::time::Instant::now() + self.timeout,
                EGRESS_POLICY.scope(policy.clone(), async move {
                    execute::fetch(
                        &fetch_env,
                        Box::new(accessor),
                        &access,
                        policy,
                        task,
                        cursor,
                        resource,
                    )
                    .await
                }),
            )
            .await?;
        let mut success = fetched.success;
        success.follows =
            open_follows(self.ctx.clone(), fetched.follows, fetched.max_fetches).await;
        Ok(success)
    }
}

struct CtxReceipts {
    ctx: Arc<dyn AbilityCtx>,
}

#[async_trait]
impl Receipts for CtxReceipts {
    async fn complete(&self, receipt_id: &str) -> std::result::Result<(), planner::PlannerError> {
        let method = MethodId::new("complete").expect("method id");
        self.ctx
            .call(
                "paid_gateway",
                &method,
                json!({ "receiptId": receipt_id }),
            )
            .await
            .map_err(|e| planner::PlannerError::new(e.to_string()))?;
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
    ctx: Arc<dyn AbilityCtx>,
    works: Vec<FollowWork>,
    max_fetches: usize,
) -> Vec<AccessTask> {
    let mut out = Vec::new();
    let resolve = MethodId::new("resolve").expect("method id");
    let get = MethodId::new("get").expect("method id");
    for work in works {
        if out.len() >= max_fetches {
            break;
        }
        let rid = ctx
            .call("strata_read", &resolve, json!({ "identity": work.identity }))
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string));
        if let Some(rid) = rid {
            let reference = FacetRef::new(
                match strata_model::Rid::parse(&rid) {
                    Ok(r) => r,
                    Err(_) => continue,
                },
                FacetKind::Struct,
                Some("page".into()),
                VersionSel::Latest,
            );
            if let Ok(reference) = reference {
                let facet = ctx
                    .call(
                        "strata_read",
                        &get,
                        json!({ "facet": reference.to_string(), "fresh": true }),
                    )
                    .await
                    .ok();
                if let Some(value) = facet {
                    if needs_met(value.get("struct"), &work.needs) {
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
