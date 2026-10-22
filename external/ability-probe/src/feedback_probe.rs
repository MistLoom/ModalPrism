//! `probe.feedback`：invoke 内提交反馈，返回前宿主已收下。

use std::sync::Arc;

use ability_sdk::prelude::*;

pub fn feedback_provider() -> Arc<dyn AbilityProvider> {
    AbilitySpec::must("probe.feedback")
        .on(
            "ping",
            "提交一条 feedback",
            json!({ "type": "object" }),
            handler!(|_ctx, _args| {
                feedback!(["guest"], "probe-feedback-rust");
                Ok(json!({ "ok": true }))
            }),
        )
        .into_provider()
}
