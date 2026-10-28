//! HTTP 端口：将 REST 请求映射为统一调用信封后进入管道。

mod error;
mod remote;
mod routes;

pub use remote::{RemoteHttpGateway, remote_error_from_body};

use std::net::SocketAddr;
use std::sync::Arc;

use ability_support::RateLimiter;
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
    /// 可选路由扩展：在默认 REST 面之上合并应用自有路由（厚领域 REST 留应用）。
    route_extension: Option<Arc<dyn Fn(Router) -> Router + Send + Sync>>,
    /// 可选 Bearer 令牌：设置后所有路由（`/health` 除外）要求匹配凭证。
    auth_token: Option<Arc<String>>,
    /// 可选限流器：作用于 invoke 准入，按目标实例计桶。
    rate_limiter: Option<Arc<dyn RateLimiter>>,
}

impl HttpPort {
    pub fn new(bind: SocketAddr) -> Self {
        Self::with_controller(bind, Arc::new(DefaultHttpController))
    }

    pub fn with_controller(bind: SocketAddr, controller: Arc<dyn ProtocolController>) -> Self {
        Self {
            bind,
            controller,
            route_extension: None,
            auth_token: None,
            rate_limiter: None,
        }
    }

    /// 注册路由扩展：`extend` 收到含默认 REST 面的 Router，返回补充了应用路由的 Router。
    /// 用于应用层自有领域 REST（复杂领域接口），内部能力调用仍走统一 Gateway 面。
    pub fn with_route_extension<F>(mut self, extend: F) -> Self
    where
        F: Fn(Router) -> Router + Send + Sync + 'static,
    {
        self.route_extension = Some(Arc::new(extend));
        self
    }

    /// 启用 Bearer 鉴权：所有路由（`/health` 除外）要求 `Authorization: Bearer <token>`。
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(Arc::new(token.into()));
        self
    }

    /// 启用限流：invoke 准入按目标实例计令牌桶。
    pub fn with_rate_limit(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    fn apply_extension(&self, router: Router) -> Router {
        match &self.route_extension {
            Some(extend) => extend(router),
            None => router,
        }
    }

    pub fn router(gateway: Arc<dyn Gateway>) -> Router {
        Self::router_with(gateway, Arc::new(DefaultHttpController))
    }

    pub fn router_with(
        gateway: Arc<dyn Gateway>,
        controller: Arc<dyn ProtocolController>,
    ) -> Router {
        routes::build_router_with(gateway, controller, None, None)
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
        let controller = self.controller.clone();
        let app = self
            .apply_extension(routes::build_router_with(
                gateway,
                controller,
                self.rate_limiter.clone(),
                self.auth_token.clone(),
            ))
            .layer(CorsLayer::permissive())
            .layer(TraceLayer::new_for_http());
        let listener = TcpListener::bind(self.bind).await.map_err(Error::from)?;
        info!(%self.bind, "http port listening");
        axum::serve(listener, app).await.map_err(Error::from)?;
        Ok(())
    }
}
