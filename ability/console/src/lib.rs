//! 控制台 Ability：经 Gateway 暴露，内部调用 `console` 领域逻辑。

use std::sync::Arc;

use ability_support::{AbilityProvider, AbilitySpec};

pub fn provider(services: Arc<assembly::Services>) -> Arc<dyn AbilityProvider> {
    let mut spec = AbilitySpec::must("console").stateful();
    spec = spec.on(
        "dispatch",
        "执行控制台读写的统一入口",
        serde_json::json!({
            "type": "object",
            "properties": {
                "op": { "type": "string" },
                "args": { "type": "object" }
            },
            "required": ["op"]
        }),
        {
            let services = Arc::clone(&services);
            move |_ctx, args| {
                let services = Arc::clone(&services);
                ability_support::box_fut(async move {
                    let op = args
                        .get("op")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| ability_support::Error::invalid_args("需要 op"))?;
                    let payload = args.get("args").cloned().unwrap_or(serde_json::json!({}));
                    let ctx = console::ConsoleContext {
                        services,
                        http_addr: String::new(),
                    };
                    console::dispatch(&ctx, op, payload)
                        .await
                        .map_err(map_err)
                })
            }
        },
    );
    spec.into_provider()
}

fn map_err(error: console::ConsoleError) -> ability_support::Error {
    if error.status == 400 {
        ability_support::Error::invalid_args(error.message)
    } else if error.status == 404 {
        ability_support::Error::not_found(error.message)
    } else if error.status == 403 {
        ability_support::Error::invalid_args(error.message)
    } else {
        ability_support::Error::failed(error.message)
    }
}
