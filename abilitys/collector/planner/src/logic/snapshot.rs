use std::sync::Arc;

use ability_support::{AbilityCtx, Error, Result};

pub async fn from_ctx(ctx: &dyn AbilityCtx) -> Result<Arc<config::Snapshot>> {
    let call = crate::ctx_call::wrap(ctx.retain());
    collector_ports::applied(call.as_ref())
        .await
        .map_err(|e| Error::failed(e))
}
