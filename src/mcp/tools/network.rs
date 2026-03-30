//! Network MCP tools.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::{DashMcpService, collect_available, network_display_name};
use crate::mcp::tools::EmptyParams;
use dash_sdk::dpp::dashcore::Network;

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

// ---------------------------------------------------------------------------
// NetworkReinitSdk
// ---------------------------------------------------------------------------

/// Rebuild the Core RPC client and Platform SDK using the current network
/// configuration. Use this after changing RPC credentials or DAPI addresses
/// to apply the new settings without restarting the app.
pub struct NetworkReinitSdk;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ReinitSdkParams {
    /// Target network. Required — must match the server's active network.
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ReinitSdkOutput {
    success: bool,
}

impl ToolBase for NetworkReinitSdk {
    type Parameter = ReinitSdkParams;
    type Output = ReinitSdkOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "network_reinit_sdk".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Rebuild the Core RPC client and Platform SDK using the current network \
             configuration. Use after changing RPC credentials or DAPI addresses."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for NetworkReinitSdk {
    async fn invoke(
        service: &DashMcpService,
        param: ReinitSdkParams,
    ) -> Result<ReinitSdkOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;

        let task = BackendTask::ReinitCoreClientAndSdk;
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::CoreClientReinitialized => {
                Ok(ReinitSdkOutput { success: true })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// NetworkSwitch
// ---------------------------------------------------------------------------

/// Switch the active network. Creates a new context for the target network
/// if needed, then swaps the MCP server to use it.
pub struct NetworkSwitch;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct NetworkSwitchParams {
    /// Target network (e.g. "mainnet", "testnet", "devnet", "local").
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct NetworkSwitchOutput {
    /// The network that is now active.
    active: String,
}

impl ToolBase for NetworkSwitch {
    type Parameter = NetworkSwitchParams;
    type Output = NetworkSwitchOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "network_switch".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Switch the active network. Creates the context if needed (may take \
             a few seconds). Requires that the target network has DAPI addresses \
             configured."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(true),
        )
    }
}

/// Parse a network name string into a `Network` enum value.
fn parse_network(name: &str) -> Result<Network, McpToolError> {
    match name.to_lowercase().as_str() {
        "mainnet" | "main" => Ok(Network::Mainnet),
        "testnet" | "test" => Ok(Network::Testnet),
        "devnet" | "dev" => Ok(Network::Devnet),
        "regtest" | "local" => Ok(Network::Regtest),
        other => Err(McpToolError::InvalidParam {
            message: format!("Unknown network '{other}'. Use mainnet, testnet, devnet, or local."),
        }),
    }
}

impl AsyncTool<DashMcpService> for NetworkSwitch {
    async fn invoke(
        service: &DashMcpService,
        param: NetworkSwitchParams,
    ) -> Result<NetworkSwitchOutput, McpToolError> {
        let target = parse_network(&param.network)?;

        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        // Already on the target network — no-op.
        if ctx.network() == target {
            return Ok(NetworkSwitchOutput {
                active: network_display_name(target).to_owned(),
            });
        }

        // Dispatch SwitchNetwork through the standard backend task system.
        let task = BackendTask::SwitchNetwork(target);
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::NetworkContextCreated { context, .. } => {
                // Persist the network choice so the next CLI invocation
                // starts on the same network.
                context
                    .update_settings(crate::ui::RootScreenType::RootScreenNetworkChooser)
                    .ok();
                service.swap_context(context);
                Ok(NetworkSwitchOutput {
                    active: network_display_name(target).to_owned(),
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
    }
}
