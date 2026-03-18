//! MCP service definition and tool implementations.

use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::connection_status::OverallConnectionState;
use crate::mcp::dispatch::{dispatch_task, resolve_wallet, task_error_to_mcp};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData as McpError, ServerHandler, schemars, tool, tool_handler, tool_router};
use std::sync::Arc;

/// Poll interval for waiting on SPV connection — matches ConnectionStatus throttle.
const SPV_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
/// Initial SPV sync (headers, masternodes, filters, blocks) can take several minutes.
const SPV_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct WalletIdParam {
    /// Wallet alias or 64-char hex seed hash
    wallet_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SendFundsParams {
    /// Wallet alias or 64-char hex seed hash (sender)
    wallet_id: String,
    /// Recipient address (Dash address string)
    address: String,
    /// Amount to send in duffs (1 DASH = 100,000,000 duffs)
    amount_duffs: u64,
}

/// Abstracts how the MCP service obtains its AppContext.
#[derive(Clone)]
enum ContextProvider {
    /// HTTP mode: context provided by the GUI app, follows network switches.
    #[cfg(feature = "mcp")]
    Shared(Arc<arc_swap::ArcSwap<AppContext>>),
    /// Stdio/CLI mode: lazily initialized on first use.
    #[cfg(feature = "cli")]
    Lazy(Arc<tokio::sync::OnceCell<Arc<AppContext>>>),
}

/// MCP service backed by the app's context.
///
/// Works with both transports: HTTP (shared ArcSwap context from the GUI app)
/// and stdio (lazily initialized standalone context).
#[derive(Clone)]
pub struct DashMcpService {
    ctx_provider: ContextProvider,
    tool_router: ToolRouter<DashMcpService>,
}

impl std::fmt::Debug for DashMcpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashMcpService").finish_non_exhaustive()
    }
}

impl DashMcpService {
    /// For HTTP mode: wrap an existing shared context.
    #[cfg(feature = "mcp")]
    pub fn new_shared(app_context: Arc<arc_swap::ArcSwap<AppContext>>) -> Self {
        Self {
            ctx_provider: ContextProvider::Shared(app_context),
            tool_router: Self::tool_router(),
        }
    }

    /// For stdio/CLI mode: lazy init on first tool call.
    #[cfg(feature = "cli")]
    pub fn new_lazy() -> Self {
        Self {
            ctx_provider: ContextProvider::Lazy(Arc::new(tokio::sync::OnceCell::new())),
            tool_router: Self::tool_router(),
        }
    }

    /// Get the current AppContext. In HTTP mode, loads from ArcSwap.
    /// In stdio mode, initializes on first call.
    async fn ctx(&self) -> Result<Arc<AppContext>, McpError> {
        match &self.ctx_provider {
            #[cfg(feature = "mcp")]
            ContextProvider::Shared(swap) => Ok(swap.load_full()),
            #[cfg(feature = "cli")]
            ContextProvider::Lazy(cell) => cell
                .get_or_try_init(|| async { init_app_context().await })
                .await
                .cloned(),
        }
    }
}

/// Initialize an AppContext for standalone/CLI mode.
/// Uses the last network selected in the GUI (from database), defaults to mainnet.
/// Starts the SPV client so wallet operations work, but does not block waiting
/// for wallets to load — individual tools wait for their specific wallet.
#[cfg(feature = "cli")]
async fn init_app_context() -> Result<Arc<AppContext>, McpError> {
    use crate::app_dir::{
        app_user_data_dir_path, data_file_path, ensure_data_dir_exists, ensure_env_file,
    };
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::Database;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;

    let data_dir = app_user_data_dir_path()
        .map_err(|e| McpError::internal_error(format!("data dir: {e}"), None))?;
    ensure_data_dir_exists(&data_dir)
        .map_err(|e| McpError::internal_error(format!("ensure data dir: {e}"), None))?;
    ensure_env_file(&data_dir);

    let env_path = data_dir.join(".env");
    if env_path.exists() {
        let _ = dotenvy::from_path(&env_path);
    }

    let db_file_path = data_file_path(&data_dir, "data.db")
        .map_err(|e| McpError::internal_error(format!("db path: {e}"), None))?;
    let db = Arc::new(
        Database::new(&db_file_path)
            .map_err(|e| McpError::internal_error(format!("db open: {e}"), None))?,
    );
    db.initialize(&db_file_path)
        .map_err(|e| McpError::internal_error(format!("db init: {e}"), None))?;

    let network = db
        .get_settings()
        .ok()
        .flatten()
        .map(|(network, ..)| network)
        .unwrap_or(Network::Dash);

    let subtasks = Arc::new(TaskManager::new());
    let connection_status = Arc::new(ConnectionStatus::new());

    // Pre-validate: check that the selected network has a config before attempting
    // AppContext::new(), which returns an opaque None on failure.
    let config = crate::config::Config::load_from(&data_dir)
        .map_err(|e| McpError::internal_error(format!("config load: {e}"), None))?;
    if config.config_for_network(network).is_none() {
        let available = available_network_names(&config);
        return Err(McpError::internal_error(
            format!(
                "no configuration found for network '{network:?}'. \
                 Available: {available}. Check your .env file.",
            ),
            None,
        ));
    }

    let app_context = AppContext::new(
        data_dir,
        network,
        db,
        None, // no wallet passwords in MCP server
        subtasks,
        connection_status,
        egui::Context::default(),
    )
    .ok_or_else(|| {
        McpError::internal_error(
            "failed to create AppContext — check logs for details".to_string(),
            None,
        )
    })?;

    if let Err(e) = app_context.start_spv() {
        tracing::warn!("SPV start failed (wallet tools may not work): {e}");
    } else {
        tracing::info!("SPV client started, wallets loading in background");
    }

    Ok(app_context)
}

/// Collect configured network names from a Config.
fn collect_available(config: &crate::config::Config) -> Vec<&'static str> {
    let mut names = Vec::new();
    if config.mainnet_config.is_some() {
        names.push("mainnet");
    }
    if config.testnet_config.is_some() {
        names.push("testnet");
    }
    if config.devnet_config.is_some() {
        names.push("devnet");
    }
    if config.local_config.is_some() {
        names.push("local");
    }
    names
}

/// Human-readable network name for JSON output.
fn network_display_name(network: dash_sdk::dpp::dashcore::Network) -> &'static str {
    use dash_sdk::dpp::dashcore::Network;
    match network {
        Network::Dash => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "local",
        _ => "unknown",
    }
}

/// Wait for SPV to reach fully-synced (green) state.
async fn wait_for_spv_sync(ctx: &AppContext) -> Result<(), McpError> {
    let deadline = tokio::time::Instant::now() + SPV_WAIT_TIMEOUT;
    loop {
        let _ = ctx.connection_status.trigger_refresh(ctx);
        let state = ctx.connection_status.overall_state();
        if state == OverallConnectionState::Synced {
            return Ok(());
        }
        if state == OverallConnectionState::Error {
            return Err(McpError::internal_error(
                "SPV connection failed. Check your network configuration.".to_string(),
                None,
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(McpError::internal_error(
                format!(
                    "SPV sync timed out after {} seconds (state: {state:?}). \
                     Check your network.",
                    SPV_WAIT_TIMEOUT.as_secs()
                ),
                None,
            ));
        }
        tokio::time::sleep(SPV_WAIT_POLL_INTERVAL).await;
    }
}

/// Return a comma-separated list of configured network names.
fn available_network_names(config: &crate::config::Config) -> String {
    let mut names = Vec::new();
    if config.mainnet_config.is_some() {
        names.push("mainnet");
    }
    if config.testnet_config.is_some() {
        names.push("testnet");
    }
    if config.devnet_config.is_some() {
        names.push("devnet");
    }
    if config.local_config.is_some() {
        names.push("local");
    }
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

#[tool_router]
impl DashMcpService {
    #[tool(
        description = "Show the active network and which networks are configured. Returns JSON with 'active' and 'available' fields."
    )]
    async fn network(&self) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx().await?;
        let active = network_display_name(ctx.network);

        let config = crate::config::Config::load_from(&ctx.data_dir)
            .map_err(|e| McpError::internal_error(format!("config load: {e}"), None))?;
        let available = collect_available(&config);

        let json = serde_json::json!({
            "active": active,
            "available": available,
        });
        let text = serde_json::to_string_pretty(&json)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "List wallet names currently loaded in the application")]
    async fn list_wallets(&self) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx().await?;
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
        let ctx = self.ctx().await?;
        let seed_hash = resolve_wallet(&ctx, &params.wallet_id)?;

        wait_for_spv_sync(&ctx).await?;

        // Verify the specific wallet is loaded into SPV's det_wallets map.
        if ctx.spv_manager.wallet_id_for_seed(seed_hash).is_none() {
            return Err(McpError::internal_error(
                "Wallet is not loaded into SPV. Please retry in a moment.".to_string(),
                None,
            ));
        }

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

    #[tool(
        description = "Show wallet balances (total, confirmed, unconfirmed) in duffs. Pass wallet alias or hex seed hash."
    )]
    async fn wallet_balances(
        &self,
        Parameters(params): Parameters<WalletIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx().await?;
        let seed_hash = resolve_wallet(&ctx, &params.wallet_id)?;

        let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
        let wallet_arc = wallets
            .get(&seed_hash)
            .ok_or_else(|| McpError::invalid_params("Wallet not found".to_string(), None))?;
        let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());

        let json = serde_json::json!({
            "alias": wallet.alias,
            "total_duffs": wallet.total_balance_duffs(),
            "confirmed_duffs": wallet.confirmed_balance_duffs(),
            "unconfirmed_duffs": wallet.unconfirmed_balance_duffs(),
        });
        let text = serde_json::to_string_pretty(&json)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Send DASH from a wallet to an address. Amount is in duffs (1 DASH = 100,000,000 duffs)."
    )]
    async fn send_core_funds(
        &self,
        Parameters(params): Parameters<SendFundsParams>,
    ) -> Result<CallToolResult, McpError> {
        let ctx = self.ctx().await?;
        let seed_hash = resolve_wallet(&ctx, &params.wallet_id)?;

        wait_for_spv_sync(&ctx).await?;

        let wallet_arc = {
            let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| McpError::invalid_params("Wallet not found".to_string(), None))?
        };

        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: params.address,
                amount_duffs: params.amount_duffs,
            }],
            subtract_fee_from_amount: false,
            memo: None,
            override_fee: None,
        };

        let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: wallet_arc,
            request,
        });

        let result = dispatch_task(&ctx, task).await.map_err(task_error_to_mcp)?;
        match result {
            BackendTaskSuccessResult::WalletPayment {
                txid,
                recipients,
                total_amount,
            } => {
                let json = serde_json::json!({
                    "txid": txid,
                    "recipients": recipients.iter().map(|(addr, amt)| {
                        serde_json::json!({"address": addr, "amount_duffs": amt})
                    }).collect::<Vec<_>>(),
                    "total_amount_duffs": total_amount,
                });
                let text = serde_json::to_string_pretty(&json)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(text)]))
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
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME").to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(
                "Dash Evo Tool MCP server. Provides wallet and core operations for the Dash blockchain.".to_string(),
            )
    }
}
