//! HTTP 端口：将 REST 请求映射为统一调用信封后进入管道。

mod error;
mod routes;

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use frame_core::{DefaultHttpController, Error, Gateway, Port, ProtocolController};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

pub use routes::InvokeBody;

/// HTTP 服务端口配置。
#[derive(Clone)]
pub struct HttpPort {
    pub bind: SocketAddr,
    controller: Arc<dyn ProtocolController>,
}

impl HttpPort {
    pub fn new(bind: SocketAddr) -> Self {
        Self::with_controller(bind, Arc::new(DefaultHttpController))
    }

    pub fn with_controller(bind: SocketAddr, controller: Arc<dyn ProtocolController>) -> Self {
        Self { bind, controller }
    }

    pub fn router(gateway: Arc<dyn Gateway>) -> Router {
        Self::router_with(gateway, Arc::new(DefaultHttpController))
    }

    pub fn router_with(
        gateway: Arc<dyn Gateway>,
        controller: Arc<dyn ProtocolController>,
    ) -> Router {
        routes::build_router_with(gateway, controller)
            .layer(CorsLayer::permissive())
            .layer(TraceLayer::new_for_http())
    }
}

#[async_trait]
impl Port for HttpPort {
    fn name(&self) -> &'static str {
        "http"
    }

    async fn serve(self: Box<Self>, gateway: Arc<dyn Gateway>) -> Result<(), Error> {
        let app = Self::router_with(gateway, self.controller);
        let listener = TcpListener::bind(self.bind).await.map_err(Error::from)?;
        info!(%self.bind, "http port listening");
        axum::serve(listener, app).await.map_err(Error::from)?;
        Ok(())
    }
}
