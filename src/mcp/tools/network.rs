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
// NetworkRefreshEndpoints
// ---------------------------------------------------------------------------

/// Fetch fresh DAPI node addresses from the DCG discovery service and save
/// them to the network configuration. Equivalent to the "Refresh DAPI
/// endpoints" button in Network Settings.
pub struct NetworkRefreshEndpoints;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct RefreshEndpointsParams {
    /// Target network. Required — must match the server's active network.
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RefreshEndpointsOutput {
    /// Number of node addresses discovered.
    count: usize,
    /// Comma-separated list of DAPI node URLs saved to config.
    addresses_csv: String,
}

impl ToolBase for NetworkRefreshEndpoints {
    type Parameter = RefreshEndpointsParams;
    type Output = RefreshEndpointsOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "network_refresh_endpoints".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Fetch fresh DAPI node addresses from the Dash Core Group discovery service \
             and save them to the network configuration. The SDK is reinitialized with \
             the new addresses."
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

impl AsyncTool<DashMcpService> for NetworkRefreshEndpoints {
    async fn invoke(
        service: &DashMcpService,
        param: RefreshEndpointsParams,
    ) -> Result<RefreshEndpointsOutput, McpToolError> {
        let ctx = service
            .ctx()
            .await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;
        resolve::require_network(&ctx, Some(&param.network))?;

        let network = ctx.network();
        let task = BackendTask::DiscoverDapiNodes { network };
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        match result {
            BackendTaskSuccessResult::DapiNodesDiscovered {
                count,
                addresses_csv,
                ..
            } => {
                // Save to config and reinit SDK
                let data_dir = ctx.data_dir();
                if let Ok(mut config) = crate::config::Config::load_from(data_dir)
                    && let Some(mut network_cfg) = config.config_for_network(network).clone()
                {
                    network_cfg.dapi_addresses = Some(addresses_csv.clone());
                    config.update_config_for_network(network, network_cfg.clone());
                    let _ = config.save(data_dir);

                    if let Ok(mut cfg_lock) = ctx.config.write() {
                        *cfg_lock = network_cfg;
                    }
                    if let Err(e) = std::sync::Arc::clone(&ctx).reinit_core_client_and_sdk() {
                        tracing::warn!("SDK reinit after endpoint refresh failed: {e}");
                    }
                }

                Ok(RefreshEndpointsOutput {
                    count,
                    addresses_csv,
                })
            }
            other => Err(McpToolError::Internal(format!(
                "Unexpected task result: {other:?}"
            ))),
        }
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
