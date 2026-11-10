use anyhow::{Context, Result};
use mcplink_core::runtime::Client;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use std::path::PathBuf;
mod cli;

#[derive(Clone)]
struct Bridge {
    client: Client,
    id: String,
    info: ServerConfig,
}

impl Bridge {
    async fn forward<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, McpError> {
        let client = Client::connect(self.client.data.clone())
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let value = client
            .call(
                "mcp.request",
                json!({"id":self.id,"request":{"method":method,"params":params}}),
            )
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        if let Some(error) = value.get("rpcError") {
            return Err(serde_json::from_value(error.clone())
                .unwrap_or_else(|_| McpError::internal_error("上游请求失败", None)));
        }
        serde_json::from_value(value)
            .map_err(|_| McpError::internal_error("上游响应与能力声明不一致", None))
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
    }
    async fn list_tools(
        &self,
        p: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        self.forward("tools/list", p.unwrap_or_default()).await
    }
    async fn call_tool(
        &self,
        p: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        self.forward::<_, CallToolResult>("tools/call", p)
            .await
            .map(Into::into)
    }
    async fn list_resources(
        &self,
        p: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        self.forward("resources/list", p.unwrap_or_default()).await
    }
    async fn list_resource_templates(
        &self,
        p: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        self.forward("resources/templates/list", p.unwrap_or_default())
            .await
    }
    async fn read_resource(
        &self,
        p: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        self.forward::<_, ReadResourceResult>("resources/read", p)
            .await
            .map(Into::into)
    }
    async fn list_prompts(
        &self,
        p: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        self.forward("prompts/list", p.unwrap_or_default()).await
    }
    async fn get_prompt(
        &self,
        p: GetPromptRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        self.forward::<_, GetPromptResult>("prompts/get", p)
            .await
            .map(Into::into)
    }
}

#[tokio::main]
async fn main() {
    std::process::exit(cli::run().await);
}

async fn bridge(data: PathBuf, connection: String) -> Result<()> {
    let client = Client::connect(data).await?;
    let raw = client
        .call("mcp.describe", json!({"id":connection}))
        .await?;
    let mut info: ServerConfig = serde_json::from_value(raw).context("上游服务信息无效")?;
    let mut capabilities = json!({});
    if info.capabilities.tools.is_some() {
        capabilities["tools"] = json!({});
    }
    if info.capabilities.resources.is_some() {
        capabilities["resources"] = json!({});
    }
    if info.capabilities.prompts.is_some() {
        capabilities["prompts"] = json!({});
    }
    info.capabilities = serde_json::from_value(capabilities)?;
    info.protocol_version = ProtocolVersion::V_2025_11_25;
    let bridge = Bridge {
        client,
        id: connection,
        info,
    };
    let service = bridge
        .serve(rmcp::transport::stdio())
        .await
        .context("MCP 客户端初始化未完成")?;
    service.waiting().await.context("MCP 会话已结束")?;
    Ok(())
}
