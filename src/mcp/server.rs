//! MCP service definition and tool implementations.

use crate::backend_task::core::CoreTask;
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::mcp::dispatch::{dispatch_task, resolve_wallet, task_error_to_mcp};
use arc_swap::ArcSwap;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData as McpError, ServerHandler, schemars, tool, tool_handler, tool_router};
use std::sync::Arc;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct WalletIdParam {
    /// Wallet alias or 64-char hex seed hash
    wallet_id: String,
}

/// MCP service backed by the app's shared context.
///
/// Holds an `ArcSwap<AppContext>` so the MCP server always sees the
/// currently active network -- updated when the user switches networks in the GUI.
#[derive(Clone)]
pub struct DashMcpService {
    app_context: Arc<ArcSwap<AppContext>>,
    tool_router: ToolRouter<DashMcpService>,
}

impl DashMcpService {
    pub fn new(app_context: Arc<ArcSwap<AppContext>>) -> Self {
        Self {
            app_context,
            tool_router: Self::tool_router(),
        }
    }

    /// Load the current `AppContext` (follows network switches).
    fn ctx(&self) -> arc_swap::Guard<Arc<AppContext>> {
        self.app_context.load()
    }
}

#[tool_router]
impl DashMcpService {
    #[tool(description = "List wallet names currently loaded in the application")]
    async fn list_wallets(&self) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx();
        let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
        let list: Vec<serde_json::Value> = wallets
            .iter()
            .map(|(hash, wallet_arc)| {
                let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
                serde_json::json!({
                    "seed_hash": hex::encode(hash),
                    "alias": wallet.alias,
                })
            })
            .collect();
        let json = serde_json::to_string_pretty(&list)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Generate a new receive address for a wallet. Pass wallet alias or hex seed hash."
    )]
    async fn generate_receive_address(
        &self,
        Parameters(params): Parameters<WalletIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx();
        let seed_hash = resolve_wallet(&ctx, &params.wallet_id)?;
        let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
        let result = dispatch_task(&ctx, task).await.map_err(task_error_to_mcp)?;
        match result {
            BackendTaskSuccessResult::GeneratedReceiveAddress { address, .. } => {
                Ok(CallToolResult::success(vec![Content::text(address)]))
            }
            other => Ok(CallToolResult::success(vec![Content::text(format!(
                "{:?}",
                other
            ))])),
        }
    }

    #[tool(description = "List wallet names currently loaded in Dash Core")]
    async fn list_core_wallets(&self) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx();
        let task = BackendTask::CoreTask(CoreTask::ListCoreWallets);
        let result = dispatch_task(&ctx, task).await.map_err(task_error_to_mcp)?;
        match result {
            BackendTaskSuccessResult::CoreWalletsList(wallets) => {
                let json = serde_json::to_string_pretty(&wallets)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            other => Ok(CallToolResult::success(vec![Content::text(format!(
                "{:?}",
                other
            ))])),
        }
    }
}

#[tool_handler]
impl ServerHandler for DashMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Dash Evo Tool MCP server. Provides wallet and core operations for the Dash blockchain.".to_string(),
            )
    }
}
