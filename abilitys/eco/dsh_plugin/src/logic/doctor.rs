use ability_plugin_host::{
    PluginHostConfig, PluginHostKind, diagnose_config_async, report_to_value,
};
use ability_support::{AbilityCtx, Result};
use serde_json::Value;

pub async fn doctor(ctx: &dyn AbilityCtx, args: Value) -> Result<Value> {
    let cfg: PluginHostConfig = if args.is_null() {
        PluginHostConfig::default()
    } else {
        serde_json::from_value(args)
            .map_err(|e| ability_support::Error::invalid_args(e.to_string()))?
    };
    let report = diagnose_config_async(ctx, &cfg, PluginHostKind::Dsh).await;
    Ok(report_to_value(&report))
}
