//! gRPC 端口：catalog / invoke 通用信封。

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use frame_core::{DefaultGrpcController, Gateway, Port, ProtocolController, resolve_target};
use tonic::{Request, Response, Status, transport::Server};
use tracing::info;

pub mod proto {
    tonic::include_proto!("leaf");
}

use proto::leaf_gateway_server::{LeafGateway, LeafGatewayServer};
use proto::{
    CatalogRequest, CatalogResponse, InvokeRequest as GrpcInvoke, InvokeResponse as GrpcResponse,
};

pub struct GrpcPort {
    addr: SocketAddr,
    controller: Arc<dyn ProtocolController>,
}

impl GrpcPort {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            controller: Arc::new(DefaultGrpcController),
        }
    }

    pub fn with_controller(addr: SocketAddr, controller: Arc<dyn ProtocolController>) -> Self {
        Self { addr, controller }
    }
}

struct GrpcService {
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
}

#[async_trait]
impl LeafGateway for GrpcService {
    async fn catalog(
        &self,
        _request: Request<CatalogRequest>,
    ) -> Result<Response<CatalogResponse>, Status> {
        let entries = self
            .controller
            .catalog(self.gateway.as_ref())
            .into_iter()
            .map(|e| proto::CatalogEntry {
                instance: e.instance.to_string(),
                method: e.method.to_string(),
                description: e.description.clone(),
            })
            .collect();
        Ok(Response::new(CatalogResponse { entries }))
    }

    async fn invoke(&self, request: Request<GrpcInvoke>) -> Result<Response<GrpcResponse>, Status> {
        let req = request.into_inner();
        let args: serde_json::Value =
            serde_json::from_str(&req.args_json).unwrap_or(serde_json::Value::Null);
        let catalog = self.controller.catalog(self.gateway.as_ref());
        let target = if req.method.is_empty() {
            req.instance.clone()
        } else {
            format!("{}.{}", req.instance, req.method)
        };
        let (instance, method) = resolve_target(&target, &catalog)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        match self
            .controller
            .invoke(self.gateway.as_ref(), instance, method, args, None, None)
            .await
        {
            Ok(resp) => {
                let data_json = serde_json::to_string(&resp.data).unwrap_or_else(|_| "null".into());
                Ok(Response::new(GrpcResponse {
                    data_json,
                    ok: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(GrpcResponse {
                data_json: "null".into(),
                ok: false,
                error: e.to_string(),
            })),
        }
    }
}

#[async_trait]
impl Port for GrpcPort {
    fn name(&self) -> &'static str {
        "grpc"
    }

    async fn serve(self: Box<Self>, gateway: Arc<dyn Gateway>) -> frame_core::Result<()> {
        let svc = GrpcService {
            gateway,
            controller: self.controller,
        };
        info!(%self.addr, "starting grpc port");
        Server::builder()
            .add_service(LeafGatewayServer::new(svc))
            .serve(self.addr)
            .await
            .map_err(|e| frame_core::Error::io(e.to_string()))?;
        Ok(())
    }
}
