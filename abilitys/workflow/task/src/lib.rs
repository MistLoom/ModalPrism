//! Ability `task`：图拓扑任务多方法

mod generated;
mod methods;
mod support;

use std::sync::Arc;

use ability_support::{
    Ability, AbilityCtx, AbilityManifest, AbilityProvider, AbilitySpec, Lifecycle, MethodId, Result,
};
use async_trait::async_trait;
use serde_json::Value;

use crate::generated::manifest;
use crate::methods::{GetTool, NextTool, ProjectSetTool, StatusTool, SubmitTool, UpdateTool};
use crate::support::TaskHub;

pub fn provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::from_manifest(manifest::manifest())
        .into_factory(|_config| async move { Ok(Arc::new(TaskAbility::new()) as Arc<dyn Ability>) })
}

struct TaskAbility {
    manifest: AbilityManifest,
    project_set: ProjectSetTool,
    submit: SubmitTool,
    status: StatusTool,
    next: NextTool,
    get: GetTool,
    update: UpdateTool,
    hub: Arc<TaskHub>,
}

impl TaskAbility {
    fn new() -> Self {
        let hub = Arc::new(TaskHub::new());
        Self {
            manifest: manifest::manifest(),
            project_set: ProjectSetTool { hub: hub.clone() },
            submit: SubmitTool { hub: hub.clone() },
            status: StatusTool { hub: hub.clone() },
            next: NextTool { hub: hub.clone() },
            get: GetTool { hub: hub.clone() },
            update: UpdateTool { hub: hub.clone() },
            hub,
        }
    }
}

#[async_trait]
impl Ability for TaskAbility {
    fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    async fn on_lifecycle(&self, phase: Lifecycle, ctx: &dyn AbilityCtx) -> Result<()> {
        if phase == Lifecycle::Started {
            self.hub.bind_ctx(ctx.retain()).await;
        }
        Ok(())
    }

    async fn invoke(&self, method: &MethodId, _ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
        match method.as_str() {
            "project_set" => self.project_set.invoke(args).await,
            "submit" => self.submit.invoke(args).await,
            "status" => self.status.invoke(args).await,
            "next" => self.next.invoke(args).await,
            "get" => self.get.invoke(args).await,
            "update" => self.update.invoke(args).await,
            other => Err(ability_support::Error::not_found(other)),
        }
    }
}
