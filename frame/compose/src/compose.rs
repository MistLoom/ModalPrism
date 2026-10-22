//! 读配置 → Catalog → install Provider → 拓扑 load + Expose。

use std::collections::BTreeSet;
use std::sync::Arc;

use ability_core::{AbilityGrant, AbilityHost};
use ability_support::{InstanceId, LoadSpec, SourceId};
use frame_core::{AbilityExpose, AccessKind, Gateway, ProtocolController, ProtocolKind};

use crate::catalog::{WorkspaceCatalog, register_protocol_support};
use crate::config::ConfigCatalog;
use crate::expose::ToolHost;
use crate::load_order::topo_sort;
use crate::protocol_registry::{ProtocolRegistry, any_supports_gate, sync_protocol_support};
use crate::registry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComposeArtifact {
    #[default]
    Runtime,
    ToolBin,
}

#[derive(Clone)]
pub struct ComposeOptions {
    pub config: Option<Arc<ConfigCatalog>>,
    pub config_id: Option<String>,
    pub host: ToolHost,
    pub access_gate: Option<AccessKind>,
    pub artifact: ComposeArtifact,
    pub registry: ProtocolRegistry,
    pub catalog: WorkspaceCatalog,
    /// 本进程的入口应用。
    pub app: Option<String>,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        let catalog = WorkspaceCatalog::discover();
        let mut registry = ProtocolRegistry::new();
        registry.scan_catalog(&catalog);
        Self {
            config: None,
            config_id: None,
            host: ToolHost::default(),
            access_gate: None,
            artifact: ComposeArtifact::default(),
            registry,
            catalog,
            app: None,
        }
    }
}

impl ComposeOptions {
    pub fn expose(&self) -> &AbilityExpose {
        self.host.expose()
    }

    pub fn with_protocols(mut self, protocols: &[ProtocolKind]) -> Self {
        self.host = self.host.with_protocols(protocols);
        self
    }

    pub fn override_controller(mut self, ctrl: Arc<dyn ProtocolController>) -> Self {
        self.host = self.host.override_controller(ctrl);
        self
    }

    pub fn app(mut self, app: impl Into<String>) -> Self {
        self.app = Some(app.into());
        self
    }
}

/// 把 Catalog 中的归属写入 Host，并记录入口应用。
pub fn bind_app_scope(host: &AbilityHost, catalog: &WorkspaceCatalog, app: Option<&str>) {
    if let Some(app) = app {
        host.set_entry_app(app);
    }
    for source in catalog.sources() {
        host.set_ability_grant(
            &source.id,
            AbilityGrant {
                app: source.app.clone(),
                open: source.open,
            },
        );
    }
}

pub async fn build_gateway() -> Arc<dyn Gateway> {
    build_gateway_with(ComposeOptions::default()).await
}

pub async fn build_gateway_with(opts: ComposeOptions) -> Arc<dyn Gateway> {
    let host = AbilityHost::new();
    let expose = opts.host.expose().clone();
    host.set_expose(expose.clone());
    if let Some(gate) = opts.access_gate {
        host.set_access_gate(Some(gate));
    }

    sync_protocol_support(&host, &opts.catalog);
    bind_app_scope(&host, &opts.catalog, opts.app.as_deref());
    registry::install_all(&host).await;
    load_exposed(&host, &opts.catalog, &expose, LoadOpts::default()).await;

    host.into_gateway()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOpts {
    pub skip_loaded: bool,
}

pub async fn install_providers(host: &AbilityHost) {
    registry::install_all(host).await;
}

pub async fn load_exposed(
    host: &AbilityHost,
    catalog: &WorkspaceCatalog,
    expose: &AbilityExpose,
    opts: LoadOpts,
) {
    let mut enabled = BTreeSet::new();
    for entry in catalog.linked_sources() {
        if expose.resolve(&entry.id).enabled {
            enabled.insert(entry.id.clone());
        }
    }
    let order = topo_sort(catalog, &enabled);
    for id in order {
        let Some(entry) = catalog.get(&id) else {
            continue;
        };
        if !expose.resolve(&id).enabled {
            continue;
        }
        if opts.skip_loaded
            && host
                .catalog()
                .iter()
                .any(|e| e.instance.as_str() == id.as_str())
        {
            continue;
        }
        register_protocol_support(host, &id, &entry.root);
        let Ok(source) = SourceId::new(&id) else {
            continue;
        };
        let Ok(as_name) = InstanceId::new(&id) else {
            continue;
        };
        let config = expose.resolve(&id).config;
        if let Err(e) = host
            .load(LoadSpec::new(source, as_name).with_config(config))
            .await
        {
            tracing::warn!(id = %id, error = %e, "load ability 失败");
        }
    }
}

pub async fn apply_expose_delta(
    host: &AbilityHost,
    catalog: &WorkspaceCatalog,
    old: &AbilityExpose,
    new: &AbilityExpose,
) -> Result<(), ability_support::Error> {
    for entry in catalog.linked_sources() {
        let id = &entry.id;
        let was = old.resolve(id);
        let now = new.resolve(id);
        let iid =
            InstanceId::new(id).map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;

        if was.enabled && !now.enabled {
            if host.catalog().iter().any(|e| e.instance.as_str() == id) {
                host.unload(&iid).await?;
            }
            continue;
        }

        if !was.enabled && now.enabled {
            register_protocol_support(host, id, &entry.root);
            let source = SourceId::new(id)
                .map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
            host.load(LoadSpec::new(source, iid).with_config(now.config))
                .await?;
        }
    }
    Ok(())
}

pub async fn reload_one(
    host: &AbilityHost,
    catalog: &WorkspaceCatalog,
    expose: &AbilityExpose,
    id: &str,
) -> Result<(), ability_support::Error> {
    let iid =
        InstanceId::new(id).map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
    if host.catalog().iter().any(|e| e.instance.as_str() == id) {
        host.unload(&iid).await?;
    }
    let resolved = expose.resolve(id);
    if !resolved.enabled {
        return Ok(());
    }
    let Some(entry) = catalog.get(id) else {
        return Err(ability_support::Error::not_found(id));
    };
    register_protocol_support(host, id, &entry.root);
    let source =
        SourceId::new(id).map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
    host.load(LoadSpec::new(source, iid).with_config(resolved.config))
        .await?;
    Ok(())
}

pub fn access_allowed(opts: &ComposeOptions, kind: AccessKind, instance_ids: &[&str]) -> bool {
    opts.host.is_protocol_enabled(kind)
        && any_supports_gate(&opts.registry, &opts.catalog, kind, instance_ids)
}

#[cfg(feature = "port-fun")]
pub async fn build_fun_port() -> frame_port_fun::FunPort {
    frame_port_fun::FunPort::new(build_gateway().await)
}

#[cfg(feature = "port-fun")]
pub async fn build_fun_port_with(opts: ComposeOptions) -> frame_port_fun::FunPort {
    let mut opts = opts;
    if opts.access_gate.is_none() {
        opts.access_gate = Some(AccessKind::Fun);
    }
    let ctrl = opts
        .host
        .controller(AccessKind::Fun)
        .unwrap_or_else(|| Arc::new(frame_core::DefaultFunController));
    frame_port_fun::FunPort::with_controller(build_gateway_with(opts).await, ctrl)
}
