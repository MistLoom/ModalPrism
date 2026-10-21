use std::collections::BTreeMap;
use std::sync::Arc;

use ability_core::{AbilityGrant, AbilityHost};
use ability_support::{InstanceId, LoadSpec, SourceId};
use collector_runtime::Error;
use frame_core::AccessKind;
use serde_json::Value;

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

/// 只读进程：存储读取、配置检查、健康查询与类型投影。
pub async fn install_read_agent(bind: Value, strata_config: Value) -> Result<Arc<AbilityHost>, Error> {
    let host = Arc::new(AbilityHost::new());
    host.set_entry_app("modalprism");
    host.set_expose(crate::expose::read_agent_expose());
    let grant = AbilityGrant {
        app: "modalprism".into(),
        open: false,
    };

    let providers: Vec<(
        &str,
        Arc<dyn ability_support::AbilityProvider>,
        Vec<AccessKind>,
        Option<Value>,
    )> = vec![
        (
            "config",
            ability_config::provider(),
            vec![AccessKind::Fun],
            Some(bind.clone()),
        ),
        (
            "committer",
            ability_committer::provider(),
            vec![AccessKind::Fun],
            Some(strata_config.clone()),
        ),
        (
            "strata_read",
            ability_strata_read::provider(),
            vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
            Some(strata_config.clone()),
        ),
        (
            "config_inspect",
            ability_config_inspect::provider(),
            vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
            Some(bind.clone()),
        ),
        (
            "control_state",
            ability_control_state::provider(),
            vec![AccessKind::Fun],
            Some(strata_config.clone()),
        ),
        (
            "strata_queue",
            ability_strata_queue::provider(),
            vec![AccessKind::Fun],
            Some(strata_config.clone()),
        ),
        (
            "planner",
            ability_collector_planner::provider(),
            vec![AccessKind::Fun],
            Some(bind.clone()),
        ),
        (
            "health",
            ability_health::provider(),
            vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
            Some(bind.clone()),
        ),
        (
            "type_article",
            type_article::provider(),
            vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
            Some(bind.clone()),
        ),
        (
            "type_hotlist",
            type_hotlist::provider(),
            vec![AccessKind::Fun, AccessKind::Mcp, AccessKind::Tools],
            Some(bind.clone()),
        ),
    ];

    for (id, provider, protocols, bind_config) in providers {
        host.set_ability_grant(id, grant.clone());
        host.install_str(id, provider)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
        host.set_protocol_support(id, protocols);
        let source = SourceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let instance = InstanceId::new(id).map_err(|error| Error::new(error.to_string()))?;
        let spec = LoadSpec::new(source, instance);
        let spec = match bind_config {
            Some(config) => spec.with_config(config),
            None => spec,
        };
        let spec = match id {
            "planner" => with_bindings(spec, &["config", "strata_queue", "control_state"])?,
            "health" => with_bindings(spec, &["config", "planner"])?,
            "committer" => with_bindings(spec, &["config"])?,
            _ => spec,
        };
        host.load(spec)
            .await
            .map_err(|error| Error::new(error.to_string()))?;
    }
    Ok(host)
}
