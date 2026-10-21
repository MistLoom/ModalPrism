use std::collections::BTreeMap;
use std::sync::Arc;

use ability_core::{AbilityGrant, AbilityHost};
use ability_support::{InstanceId, LoadSpec, SourceId};
use collector_runtime::Error;
use frame_core::AccessKind;
use serde_json::Value;

pub async fn install_collector(bind: Value, strata_config: Value) -> Result<Arc<AbilityHost>, Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    let grant = AbilityGrant {
        app: "modalprism".into(),
        open: false,
    };

    let providers: Vec<(
        &str,
        Arc<dyn ability_support::AbilityProvider>,
        bool,
        Option<Value>,
    )> = vec![
        (
            "config",
            ability_config::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "committer",
            ability_committer::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "strata_read",
            ability_strata_read::provider(),
            true,
            Some(strata_config.clone()),
        ),
        (
            "strata_write",
            ability_strata_write::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "strata_queue",
            ability_strata_queue::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "control_state",
            ability_control_state::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "config_inspect",
            ability_config_inspect::provider(),
            true,
            Some(bind.clone()),
        ),
        (
            "type_hotlist",
            type_hotlist::provider(),
            false,
            Some(strata_config.clone()),
        ),
        (
            "egress",
            ability_egress::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "paid_gateway",
            ability_paid_gateway::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "planner",
            ability_collector_planner::provider(),
            true,
            Some(bind.clone()),
        ),
        (
            "runner",
            ability_collector_runner::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "health",
            ability_health::provider(),
            true,
            Some(bind.clone()),
        ),
        (
            "push_ingress",
            ability_push_ingress::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_http",
            ability_accessor_http::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_rss",
            ability_accessor_rss::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_api",
            ability_accessor_api::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_paid_api",
            ability_accessor_paid_api::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_search",
            ability_accessor_search::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_push",
            ability_accessor_push::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_file",
            ability_accessor_file::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_browser",
            ability_accessor_browser::provider(),
            false,
            Some(bind.clone()),
        ),
        (
            "accessor_bundle_list",
            ability_accessor_bundle_list::provider(),
            false,
            Some(bind.clone()),
        ),
    ];

    for (id, provider, http, bind_config) in providers {
        host.set_ability_grant(id, grant.clone());
        host.install_str(id, provider)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
        let kinds = if http {
            vec![AccessKind::Fun, AccessKind::Http]
        } else {
            vec![AccessKind::Fun]
        };
        host.set_protocol_support(id, kinds);
        let source = SourceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let instance = InstanceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let spec = LoadSpec::new(source, instance);
        let spec = match bind_config {
            Some(config) => spec.with_config(config),
            None => spec,
        };
        let spec = match id {
            "planner" => with_bindings(
                spec,
                &["config", "strata_queue", "control_state"],
            )?,
            "runner" => with_bindings(
                spec,
                &[
                    "config",
                    "strata_queue",
                    "strata_write",
                    "strata_read",
                    "egress",
                    "paid_gateway",
                    "control_state",
                    "accessor_http",
                    "accessor_rss",
                    "accessor_api",
                    "accessor_paid_api",
                    "accessor_search",
                    "accessor_push",
                    "accessor_file",
                    "accessor_browser",
                    "accessor_bundle_list",
                ],
            )?,
            "health" => with_bindings(spec, &["config", "planner", "strata_write"])?,
            "push_ingress" => with_bindings(
                spec,
                &["config", "egress", "paid_gateway", "strata_write"],
            )?,
            "committer" => with_bindings(spec, &["config", "type_hotlist", "strata_write"])?,
            "type_hotlist" => with_bindings(spec, &["config"])?,
            "egress" | "paid_gateway" => with_bindings(spec, &["config"])?,
            _ => spec,
        };
        host.load(spec)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
    }
    Ok(host)
}

fn with_bindings(spec: LoadSpec, deps: &[&str]) -> Result<LoadSpec, Error> {
    let mut bindings = BTreeMap::new();
    for dep in deps {
        bindings.insert(
            (*dep).to_string(),
            InstanceId::new(*dep).map_err(|error| Error::new(error.to_string()))?,
        );
    }
    Ok(LoadSpec {
        bindings: Some(bindings),
        ..spec
    })
}
