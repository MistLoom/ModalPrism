//! 装配织入的切面：计数，并写入当前 scope locals。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ability_sdk::{AbilityCtx, Aspect, Next, Result};
use async_trait::async_trait;
use serde_json::json;

pub struct TraceAspect {
    hits: AtomicU64,
}

impl TraceAspect {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            hits: AtomicU64::new(0),
        })
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Aspect for TraceAspect {
    async fn around(
        &self,
        ctx: &dyn AbilityCtx,
        _args: &serde_json::Value,
        next: Next<'_>,
    ) -> Result<serde_json::Value> {
        let n = self.hits.fetch_add(1, Ordering::SeqCst) + 1;
        ctx.local_set("aspect.hits", json!(n)).await?;
        next.run().await
    }
}
