//! Platform query tools: withdrawal status, epoch info, etc.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::platform_info::{PlatformInfoTaskRequestType, PlatformInfoTaskResult};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::DashMcpService;

// ---------------------------------------------------------------------------
// QueryWithdrawals
// ---------------------------------------------------------------------------

/// Query withdrawal documents from Platform.
pub struct QueryWithdrawals;

// TODO: Add pagination support (limit/start_after params) once
// PlatformInfoTaskRequestType variants accept query parameters.
// Currently the backend hardcodes limit=50 in DocumentQuery.
#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct QueryWithdrawalsParams {
    /// Which withdrawals to query: "queued" (default) or "completed"
    #[serde(default = "default_queued")]
    pub status: String,
    /// Expected network (e.g. "mainnet", "testnet"). If provided, the request fails when it
    /// doesn't match the server's active network.
    #[serde(default)]
    pub network: Option<String>,
}

fn default_queued() -> String {
    "queued".to_string()
}

// TODO: Return structured withdrawal data (amount, status, address, timestamps)
// instead of a pre-formatted text string. Requires a new backend result variant
// that returns Vec<WithdrawalDocument> rather than TextResult(String).
#[derive(Serialize, schemars::JsonSchema)]
pub struct QueryWithdrawalsOutput {
    /// Human-readable withdrawal information
    pub info: String,
}

impl ToolBase for QueryWithdrawals {
    type Parameter = QueryWithdrawalsParams;
    type Output = QueryWithdrawalsOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "platform_withdrawals_get".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Query withdrawal documents from Platform. \
             Pass status=\"queued\" (default) or status=\"completed\"."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(true))
    }
}

impl AsyncTool<DashMcpService> for QueryWithdrawals {
    async fn invoke(
        service: &DashMcpService,
        params: QueryWithdrawalsParams,
    ) -> Result<QueryWithdrawalsOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::verify_network(&ctx, params.network.as_deref())?;

        let request = match params.status.as_str() {
            "completed" | "complete" => PlatformInfoTaskRequestType::RecentlyCompletedWithdrawals,
            "queued" | "" => PlatformInfoTaskRequestType::CurrentWithdrawalsInQueue,
            other => {
                return Err(McpToolError::InvalidParam {
                    message: format!(
                        "Unknown status \"{other}\". Use \"queued\" or \"completed\"."
                    ),
                });
            }
        };

        let task = BackendTask::PlatformInfo(request);
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::TextResult(text)) => {
                Ok(QueryWithdrawalsOutput { info: text })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected result variant: {other:?}"
            ))),
        }
    }
}
