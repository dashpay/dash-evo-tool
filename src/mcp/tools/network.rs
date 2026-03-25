//! Network MCP tools.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::Serialize;

use crate::mcp::error::McpToolError;
use crate::mcp::server::{DashMcpService, collect_available, network_display_name};
use crate::mcp::tools::EmptyParams;

// ---------------------------------------------------------------------------
// NetworkTool
// ---------------------------------------------------------------------------

/// Show the active network and which networks are configured.
pub struct NetworkTool;

#[derive(Serialize, schemars::JsonSchema)]
pub struct NetworkOutput {
    active: String,
    available: Vec<String>,
}

impl ToolBase for NetworkTool {
    type Parameter = EmptyParams;
    type Output = NetworkOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "network_info".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Show the active network and which networks are configured. \
             Returns JSON with 'active' and 'available' fields."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(true))
    }
}

impl AsyncTool<DashMcpService> for NetworkTool {
    async fn invoke(
        service: &DashMcpService,
        _param: EmptyParams,
    ) -> Result<NetworkOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        let active = network_display_name(ctx.network()).to_owned();

        let config = crate::config::Config::load_from(ctx.data_dir())
            .map_err(|e| McpToolError::Internal(format!("config load: {e}")))?;
        let available = collect_available(&config)
            .into_iter()
            .map(|s| s.to_owned())
            .collect();

        Ok(NetworkOutput { active, available })
    }
}
