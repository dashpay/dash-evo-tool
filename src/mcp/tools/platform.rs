//! Platform query tools: withdrawal status, epoch info, etc.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;

use crate::backend_task::platform_info::{
    PlatformInfoTaskRequestType, PlatformInfoTaskResult, WithdrawalRecord,
};
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

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct QueryWithdrawalsParams {
    /// Which withdrawals to query: "queued" (default) or "completed".
    #[serde(default = "default_queued")]
    pub status: String,
    /// Maximum number of withdrawals to return (1–100). Defaults to 50.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Continuation cursor: the `document_id` from a previous response's
    /// `next_cursor`. Returns the page of withdrawals that follow it.
    #[serde(default)]
    pub start_after: Option<String>,
    /// Expected network (e.g. "mainnet", "testnet"). If provided, the request fails when it
    /// doesn't match the server's active network.
    #[serde(default)]
    pub network: Option<String>,
}

fn default_queued() -> String {
    "queued".to_string()
}

/// One withdrawal in the structured response. Amounts are in credits (atomic
/// units, 1 Dash = 100_000_000_000 credits); timestamps are Unix milliseconds.
#[derive(Serialize, schemars::JsonSchema)]
pub struct WithdrawalEntry {
    /// Withdrawal document id (base58). Also the page cursor for `start_after`.
    pub document_id: String,
    /// Identity that requested the withdrawal (base58).
    pub owner_id: String,
    /// Amount in credits (atomic units).
    pub amount_credits: u64,
    /// Status: "queued", "pooled", "broadcasted", "complete", or "expired".
    pub status: String,
    /// Destination Dash address, or null when the output script is non-standard.
    pub address: Option<String>,
    /// Sequential on-chain transaction index, when present.
    pub transaction_index: Option<u64>,
    /// Document creation time (Unix ms), when present.
    pub created_at_ms: Option<u64>,
    /// Document last-update time (Unix ms), when present.
    pub updated_at_ms: Option<u64>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct QueryWithdrawalsOutput {
    /// One entry per matching withdrawal document.
    pub withdrawals: Vec<WithdrawalEntry>,
    /// Sum of every returned entry's `amount_credits`.
    pub total_amount_credits: u64,
    /// Pass as the next request's `start_after` to fetch the following page.
    /// Null when no further results are available.
    pub next_cursor: Option<String>,
}

impl From<WithdrawalRecord> for WithdrawalEntry {
    fn from(r: WithdrawalRecord) -> Self {
        WithdrawalEntry {
            document_id: r.document_id.to_string(Encoding::Base58),
            owner_id: r.owner_id.to_string(Encoding::Base58),
            amount_credits: r.amount_credits,
            status: r.status,
            address: r.address,
            transaction_index: r.transaction_index,
            created_at_ms: r.created_at_ms,
            updated_at_ms: r.updated_at_ms,
        }
    }
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
             Pass status=\"queued\" (default) or status=\"completed\". \
             Use limit and start_after for pagination."
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

        let completed = match params.status.as_str() {
            "completed" | "complete" => true,
            "queued" | "" => false,
            other => {
                return Err(McpToolError::InvalidParam {
                    message: format!(
                        "Unknown status \"{other}\". Use \"queued\" or \"completed\"."
                    ),
                });
            }
        };

        let start_after = params
            .start_after
            .as_deref()
            .map(|cursor| {
                Identifier::from_string(cursor, Encoding::Base58).map_err(|_| {
                    McpToolError::InvalidParam {
                        message: format!("Invalid start_after cursor: {cursor}"),
                    }
                })
            })
            .transpose()?;

        let request = PlatformInfoTaskRequestType::Withdrawals {
            completed,
            limit: params.limit,
            start_after,
        };

        let task = BackendTask::PlatformInfo(request);
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::Withdrawals {
                records,
                total_amount_credits,
                next_cursor,
            }) => Ok(QueryWithdrawalsOutput {
                withdrawals: records.into_iter().map(WithdrawalEntry::from).collect(),
                total_amount_credits,
                next_cursor: next_cursor.map(|id| id.to_string(Encoding::Base58)),
            }),
            other => Err(McpToolError::Internal(format!(
                "Unexpected result variant: {other:?}"
            ))),
        }
    }
}
