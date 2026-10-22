//! MCP 端口：catalog 投影为 MCP Tools（传输发现词）。

use std::sync::Arc;

use async_trait::async_trait;
use frame_core::{
    CatalogEntry, DefaultMcpController, Error, Gateway, Port, ProtocolController, resolve_target,
};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, RoleServer, ServiceExt};
use serde_json::Value;
use tracing::info;

#[derive(Clone)]
pub struct McpPort {
    server_name: String,
    server_version: String,
    controller: Arc<dyn ProtocolController>,
}

impl McpPort {
    pub fn new(server_name: impl Into<String>, server_version: impl Into<String>) -> Self {
        Self::with_controller(server_name, server_version, Arc::new(DefaultMcpController))
    }

    pub fn with_controller(
        server_name: impl Into<String>,
        server_version: impl Into<String>,
        controller: Arc<dyn ProtocolController>,
    ) -> Self {
        Self {
            server_name: server_name.into(),
            server_version: server_version.into(),
            controller,
        }
    }
}

#[async_trait]
impl Port for McpPort {
    fn name(&self) -> &'static str {
        "mcp"
    }

    async fn serve(self: Box<Self>, gateway: Arc<dyn Gateway>) -> Result<(), Error> {
        let handler = McpGatewayHandler {
            gateway,
            controller: self.controller,
            server_name: self.server_name,
            server_version: self.server_version,
        };
        info!("mcp port starting on stdio");
        let service = handler
            .serve(stdio())
            .await
            .map_err(|e| Error::failed(e.to_string()))?;
        service
            .waiting()
            .await
            .map_err(|e| Error::failed(e.to_string()))?;
        Ok(())
    }
}

#[derive(Clone)]
struct McpGatewayHandler {
    gateway: Arc<dyn Gateway>,
    controller: Arc<dyn ProtocolController>,
    server_name: String,
    server_version: String,
}

impl ServerHandler for McpGatewayHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                self.server_name.clone(),
                self.server_version.clone(),
            ))
            .with_instructions("LEAF ability gateway over MCP")
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self
            .controller
            .catalog(self.gateway.as_ref())
            .into_iter()
            .map(entry_to_mcp_tool)
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let cat = self.gateway.catalog();
        let (inst, method) = match resolve_target(&request.name, &cat) {
            Ok(v) => v,
            Err(e) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into());
            }
        };

        if self.gateway.get_entry(&inst, &method).is_none() {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        }

        let args = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Default::default()));

        match self
            .controller
            .invoke(self.gateway.as_ref(), inst, method, args, None, None)
            .await
        {
            Ok(resp) => {
                let text = serde_json::to_string_pretty(&resp.data)
                    .unwrap_or_else(|_| resp.data.to_string());
                let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
                if let Some(sid) = resp.locals_scope {
                    result.structured_content = Some(serde_json::json!({
                        "data": resp.data,
                        "session_id": sid.to_string(),
                    }));
                } else {
                    result.structured_content = Some(resp.data);
                }
                Ok(result.into())
            }
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())]).into()),
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        let cat = self.gateway.catalog();
        let (inst, method) = resolve_target(name, &cat).ok()?;
        self.gateway
            .get_entry(&inst, &method)
            .map(entry_to_mcp_tool)
    }
}

fn entry_to_mcp_tool(e: CatalogEntry) -> Tool {
    let schema = json_object(e.input_schema);
    let mut tool = Tool::new(e.instance.as_str().to_owned(), e.description, schema)
        .with_title(e.method.as_str().to_owned())
        .annotate(ToolAnnotations::new().read_only(!e.stateful));
    if let Some(out) = e.output_schema {
        tool = tool.with_raw_output_schema(Arc::new(json_object(out)));
    }
    tool
}

fn json_object(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("raw".into(), other);
            map
        }
    }
}
