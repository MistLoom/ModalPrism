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
        prime_before_load(id, bind_config.as_ref(), &bind, &strata_config)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
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

async fn prime_before_load(
    id: &str,
    bind_config: Option<&Value>,
    bind: &Value,
    strata_config: &Value,
) -> ability_support::Result<()> {
    match id {
        "config" => ability_config::install::prime(bind.clone()).await?,
        "config_inspect" => ability_config_inspect::install::prime(bind.clone()).await?,
        "committer" => ability_committer::install::prime(strata_config.clone()).await?,
        "strata_read" => ability_strata_read::install::prime(strata_config.clone()).await?,
        "strata_write" => ability_strata_write::install::prime(strata_config.clone()).await?,
        "strata_queue" => ability_strata_queue::install::prime(strata_config.clone()).await?,
        "control_state" => ability_control_state::install::prime(strata_config.clone()).await?,
        "type_hotlist" => type_hotlist::install::prime(strata_config.clone()).await?,
        "egress" => ability_egress::install::prime(bind.clone()).await?,
        "paid_gateway" => ability_paid_gateway::install::prime(bind.clone()).await?,
        "runner" => ability_collector_runner::install::prime(bind.clone()).await?,
        "push_ingress" => ability_push_ingress::install::prime(bind.clone()).await?,
        "accessor_http" => ability_accessor_http::install::prime(bind.clone()).await?,
        "accessor_rss" => ability_accessor_rss::install::prime(bind.clone()).await?,
        "accessor_api" => ability_accessor_api::install::prime(bind.clone()).await?,
        "accessor_paid_api" => ability_accessor_paid_api::install::prime(bind.clone()).await?,
        "accessor_search" => ability_accessor_search::install::prime(bind.clone()).await?,
        "accessor_push" => ability_accessor_push::install::prime(bind.clone()).await?,
        "accessor_file" => ability_accessor_file::install::prime(bind.clone()).await?,
        "accessor_browser" => ability_accessor_browser::install::prime(bind.clone()).await?,
        "accessor_bundle_list" => {
            ability_accessor_bundle_list::install::prime(bind.clone()).await?
        }
        _ => {
            if let Some(config) = bind_config {
                let _ = config;
            }
        }
    }
    Ok(())
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
