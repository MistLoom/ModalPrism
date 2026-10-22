//! 应用对自身 Ability 的事件控制。
//!
//! 监听与拦截只通过这一层登记。拦截限定属主应用；监听还可观察其他应用的 `open` Ability。

use std::collections::BTreeSet;
use std::sync::Arc;

use ability_core::{AbilityHost, ChannelListener};
use ability_support::{
    BusEvent, Error, EventFilter, EventHandler, EventMiddleware, EventNext, EventOpts, EventReg,
    Result,
};
use async_trait::async_trait;

use crate::catalog::WorkspaceCatalog;

/// 某个应用的事件控制面。
#[derive(Clone)]
pub struct AppControl {
    app: String,
    host: AbilityHost,
    owned: BTreeSet<String>,
    open_foreign: BTreeSet<String>,
}

impl AppControl {
    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn from_catalog(app: &str, catalog: &WorkspaceCatalog, host: AbilityHost) -> Self {
        let mut owned = BTreeSet::new();
        let mut open_foreign = BTreeSet::new();
        for source in catalog.sources() {
            if source.app == app || source.app == "common" {
                owned.insert(source.id.clone());
            } else if source.open {
                open_foreign.insert(source.id.clone());
            }
        }
        Self {
            app: app.to_string(),
            host,
            owned,
            open_foreign,
        }
    }

    /// 监听本应用能力事件，以及他应用已开放能力的事件。
    pub async fn listen(&self, filter: EventFilter) -> Result<ChannelListener> {
        self.check(&filter, false)?;
        Ok(self
            .host
            .events()
            .listen_where(filter, self.allow(false))
            .await)
    }

    /// 拦截本应用能力事件。他应用事件会原样交给后续处理者。
    pub fn intercept(
        &self,
        filter: EventFilter,
        middleware: Arc<dyn EventMiddleware>,
    ) -> Result<EventReg> {
        self.check(&filter, true)?;
        let gate = Arc::new(GateMw {
            allow: self.allow(true),
            inner: middleware,
        });
        Ok(self
            .host
            .events()
            .handle(filter, EventHandler::Middleware(gate), EventOpts::default()))
    }

    fn check(&self, filter: &EventFilter, intercept: bool) -> Result<()> {
        let Some(inst) = filter.instance.as_ref() else {
            return Ok(());
        };
        let id = inst.as_str();
        if self.owned.contains(id) {
            return Ok(());
        }
        if !intercept && self.open_foreign.contains(id) {
            return Ok(());
        }
        let verb = if intercept { "拦截" } else { "监听" };
        Err(Error::unavailable(format!(
            "应用 {} 不能{verb}能力 {id}",
            self.app
        )))
    }

    fn allow(&self, intercept: bool) -> Arc<dyn Fn(&BusEvent) -> bool + Send + Sync> {
        let owned = self.owned.clone();
        let open_foreign = self.open_foreign.clone();
        Arc::new(move |event: &BusEvent| {
            let Some(inst) = event.instance() else {
                return false;
            };
            let id = inst.as_str();
            if owned.contains(id) {
                return true;
            }
            !intercept && open_foreign.contains(id)
        })
    }
}

struct GateMw {
    allow: Arc<dyn Fn(&BusEvent) -> bool + Send + Sync>,
    inner: Arc<dyn EventMiddleware>,
}

#[async_trait]
impl EventMiddleware for GateMw {
    async fn around(&self, event: BusEvent, next: EventNext) -> Result<BusEvent> {
        if !(self.allow)(&event) {
            return next.run(event).await;
        }
        self.inner.around(event, next).await
    }
}
