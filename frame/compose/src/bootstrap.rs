//! 可选 Bootstrap：统一装配 Host + Gateway。

use std::path::Path;
use std::sync::{Arc, RwLock};

use ability_core::AbilityHost;
use ability_support::{
    BusEvent, EventClass, EventFilter, EventHandler, EventOpts, EventSink, host_event,
};
use frame_core::{AbilityExpose, Gateway};
use serde_json::{Value, json};

use crate::app_control::AppControl;
use crate::catalog::WorkspaceCatalog;
use crate::compose::{
    ComposeOptions, LoadOpts, apply_expose_delta, bind_app_scope, install_providers, load_exposed,
    reload_one,
};
use crate::config::ConfigCatalog;
use crate::expose::ToolHost;
use crate::host_lifecycle::{HostRegistration, apply_registrations, on_host_message};
use crate::load_order::topo_sort;
use crate::protocol_registry::ProtocolRegistry;

/// 装配完成后的句柄。
pub struct AppHandle {
    pub host: AbilityHost,
    pub gateway: Arc<dyn Gateway>,
    pub expose: AbilityExpose,
    pub catalog: WorkspaceCatalog,
    pub config_path: Option<std::path::PathBuf>,
    pub invoke_log: Arc<RwLock<Vec<Value>>>,
    /// 保持 Host 事件登记存活。
    pub(crate) _host_event_regs: Vec<ability_support::EventReg>,
}

pub struct AppBuilder {
    catalog: WorkspaceCatalog,
    expose: AbilityExpose,
    config_path: Option<std::path::PathBuf>,
    registry: ProtocolRegistry,
    host_registrations: Vec<HostRegistration>,
    invoke_log: Option<Arc<RwLock<Vec<Value>>>>,
    app: Option<String>,
}

impl Default for AppBuilder {
    fn default() -> Self {
        let catalog = WorkspaceCatalog::discover();
        let mut registry = ProtocolRegistry::new();
        registry.scan_catalog(&catalog);
        Self {
            catalog,
            expose: ToolHost::default().into_expose(),
            config_path: None,
            registry,
            host_registrations: Vec::new(),
            invoke_log: None,
            app: None,
        }
    }
}

impl AppBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn catalog(mut self, catalog: WorkspaceCatalog) -> Self {
        self.registry = ProtocolRegistry::new();
        self.registry.scan_catalog(&catalog);
        self.catalog = catalog;
        self
    }

    pub fn expose(mut self, expose: AbilityExpose) -> Self {
        self.expose = expose;
        self
    }

    /// 本进程的入口应用。
    pub fn app(mut self, app: impl Into<String>) -> Self {
        self.app = Some(app.into());
        self
    }

    pub fn expose_from_config(mut self, path: impl AsRef<Path>) -> Result<Self, frame_core::Error> {
        let path = path.as_ref();
        self.config_path = Some(path.to_path_buf());
        let catalog = ConfigCatalog::new();
        let (_, value) = catalog.load_id(None, path)?;
        let expose = if value.get("tool_expose").is_some()
            || value.get("ability_expose").is_some()
            || value.get("capability_expose").is_some()
            || value.get("expose").is_some()
            || value.get("tools").is_some()
            || value.get("abilities").is_some()
            || value.get("capabilities").is_some()
        {
            AbilityExpose::from_value(&value)?
        } else {
            AbilityExpose::open_all()
        };
        self.expose = expose;
        Ok(self)
    }

    /// 登记 Host 系统事件处理（middleware / pipeline / parallel / message）。
    pub fn on_host(mut self, filter: EventFilter, handler: EventHandler, opts: EventOpts) -> Self {
        self.host_registrations
            .push(crate::host_lifecycle::on_host(filter, handler, opts));
        self
    }

    /// `message` 模式快捷登记。
    pub fn on_host_message(mut self, filter: EventFilter, sink: Arc<dyn EventSink>) -> Self {
        self.host_registrations.push(on_host_message(filter, sink));
        self
    }

    pub async fn bootstrap(self) -> AppHandle {
        let host = AbilityHost::new();
        host.set_expose(self.expose.clone());
        crate::protocol_registry::sync_protocol_support(&host, &self.catalog);
        bind_app_scope(&host, &self.catalog, self.app.as_deref());
        install_providers(&host).await;

        let invoke_log = self
            .invoke_log
            .unwrap_or_else(|| Arc::new(RwLock::new(Vec::new())));
        let mut regs = self.host_registrations;
        regs.push(on_host_message(
            EventFilter {
                class: Some(EventClass::Host),
                ..Default::default()
            },
            Arc::new(InvokeLogSink(Arc::clone(&invoke_log))),
        ));
        let host_event_regs = apply_registrations(&host, &regs);

        load_exposed(
            &host,
            &self.catalog,
            &self.expose,
            LoadOpts { skip_loaded: false },
        )
        .await;

        AppHandle {
            gateway: host.clone().into_gateway(),
            host,
            expose: self.expose,
            catalog: self.catalog,
            config_path: self.config_path,
            invoke_log,
            _host_event_regs: host_event_regs,
        }
    }

    pub fn into_compose_options(self) -> ComposeOptions {
        ComposeOptions {
            config: None,
            config_id: None,
            host: ToolHost::new(self.expose),
            access_gate: None,
            artifact: crate::compose::ComposeArtifact::Runtime,
            registry: self.registry,
            catalog: self.catalog,
            app: self.app,
        }
    }
}

struct InvokeLogSink(Arc<RwLock<Vec<Value>>>);

#[async_trait::async_trait]
impl EventSink for InvokeLogSink {
    async fn on_event(&self, event: &BusEvent) {
        let name = event.name();
        if name != host_event::INVOKE_BEFORE
            && name != host_event::INVOKE_AFTER
            && name != host_event::INVOKE_ERROR
        {
            return;
        }
        let entry = json!({
            "name": name,
            "instance": event.instance().map(|i| i.to_string()),
            "detail": event.payload().clone(),
        });
        let mut g = self.0.write().expect("invoke_log");
        g.push(entry);
        if g.len() > 200 {
            g.remove(0);
        }
    }
}

impl AppHandle {
    /// 入口应用的事件控制面。
    pub fn control(&self) -> AppControl {
        let app = self.host.entry_app().unwrap_or_default();
        AppControl::from_catalog(&app, &self.catalog, self.host.clone())
    }

    pub async fn apply_expose_delta(
        &self,
        old: &AbilityExpose,
        new: &AbilityExpose,
    ) -> Result<(), ability_support::Error> {
        apply_expose_delta(&self.host, &self.catalog, old, new).await
    }

    pub async fn reload_all(
        &self,
        expose: &AbilityExpose,
    ) -> Result<Vec<String>, ability_support::Error> {
        use std::collections::BTreeSet;
        let loaded: BTreeSet<String> = self
            .host
            .catalog()
            .iter()
            .map(|e| e.instance.to_string())
            .collect();
        let enabled: BTreeSet<String> = self
            .catalog
            .linked_sources()
            .filter(|e| expose.resolve(&e.id).enabled)
            .map(|e| e.id.clone())
            .collect();

        let mut unload_order = topo_sort(&self.catalog, &loaded);
        unload_order.reverse();
        for id in &unload_order {
            let iid = ability_support::InstanceId::new(id)
                .map_err(|e| ability_support::Error::invalid_args(e.to_string()))?;
            if self
                .host
                .catalog()
                .iter()
                .any(|e| e.instance.as_str() == id.as_str())
            {
                self.host.unload(&iid).await?;
            }
        }

        let load_order = topo_sort(&self.catalog, &enabled);
        let mut reloaded = Vec::new();
        for id in load_order {
            reload_one(&self.host, &self.catalog, expose, &id).await?;
            reloaded.push(id);
        }
        Ok(reloaded)
    }
}
