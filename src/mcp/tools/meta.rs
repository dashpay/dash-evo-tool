//! Meta MCP tool: describe_tool.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::Serialize;

use crate::mcp::error::McpToolError;
use crate::mcp::server::DashMcpService;
use crate::mcp::tools::ToolNameParams;

/// Return the full tool definition (schema, annotations, description) for a tool.
pub struct DescribeTool;

/// Wrapper for serializing an rmcp `Tool` as JSON output.
///
/// We use `serde_json::Value` because rmcp's `Tool` does not implement `JsonSchema`.
#[derive(Serialize, schemars::JsonSchema)]
pub struct DescribeToolOutput {
    tool: serde_json::Value,
}

impl ToolBase for DescribeTool {
    type Parameter = ToolNameParams;
    type Output = DescribeToolOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "describe_tool".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Return the full MCP tool definition (schema, annotations, description) \
             for a given tool name."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true))
    }
}

impl AsyncTool<DashMcpService> for DescribeTool {
    async fn invoke(
        service: &DashMcpService,
        param: ToolNameParams,
    ) -> Result<DescribeToolOutput, McpToolError> {
        let tool_def = service.tool_router.get(&param.name).ok_or_else(|| {
            McpToolError::InvalidParams(format!("Tool '{}' not found", param.name))
        })?;
        let value = serde_json::to_value(tool_def)
            .map_err(|e| McpToolError::Internal(format!("serialization: {e}")))?;
        Ok(DescribeToolOutput { tool: value })
    }
}
