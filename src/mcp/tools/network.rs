//! Network and wallet-listing MCP tools.

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
        "network".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Show the active network and which networks are configured. \
             Returns JSON with 'active' and 'available' fields."
                .into(),
        )
    }

    fn input_schema() -> Option<std::sync::Arc<rmcp::model::JsonObject>> {
        None
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
        let active = network_display_name(ctx.network).to_owned();

        let config = crate::config::Config::load_from(&ctx.data_dir)
            .map_err(|e| McpToolError::Internal(format!("config load: {e}")))?;
        let available = collect_available(&config)
            .into_iter()
            .map(|s| s.to_owned())
            .collect();

        Ok(NetworkOutput { active, available })
    }
}

// ---------------------------------------------------------------------------
// ListWalletsTool
// ---------------------------------------------------------------------------

/// List wallet names currently loaded in the application.
pub struct ListWalletsTool;

#[derive(Serialize, schemars::JsonSchema)]
pub struct WalletEntry {
    seed_hash: String,
    alias: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ListWalletsOutput {
    wallets: Vec<WalletEntry>,
}

impl ToolBase for ListWalletsTool {
    type Parameter = EmptyParams;
    type Output = ListWalletsOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "list_wallets".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("List wallet names currently loaded in the application".into())
    }

    fn input_schema() -> Option<std::sync::Arc<rmcp::model::JsonObject>> {
        None
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default().read_only(true).open_world(false))
    }
}

impl AsyncTool<DashMcpService> for ListWalletsTool {
    async fn invoke(
        service: &DashMcpService,
        _param: EmptyParams,
    ) -> Result<ListWalletsOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
        let entries: Vec<WalletEntry> = wallets
            .iter()
            .map(|(hash, wallet_arc)| {
                let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
                WalletEntry {
                    seed_hash: hex::encode(hash),
                    alias: wallet.alias.clone(),
                }
            })
            .collect();
        Ok(ListWalletsOutput { wallets: entries })
    }
}
