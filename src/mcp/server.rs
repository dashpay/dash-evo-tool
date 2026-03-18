//! MCP service definition and tool implementations.

use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::mcp::dispatch::{dispatch_task, resolve_wallet, task_error_to_mcp};
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

/// Abstracts how the MCP service obtains its AppContext.
#[derive(Clone)]
enum ContextProvider {
    /// HTTP mode: context provided by the GUI app, follows network switches.
    #[cfg(feature = "mcp")]
    Shared(Arc<arc_swap::ArcSwap<AppContext>>),
    /// Stdio/CLI mode: lazily initialized on first use.
    #[cfg(feature = "cli")]
    Lazy {
        cell: Arc<tokio::sync::OnceCell<Arc<AppContext>>>,
        /// Optional network override from --network flag.
        network_override: Option<String>,
    },
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
    /// `network_override` takes precedence over the database default.
    #[cfg(feature = "cli")]
    pub fn new_lazy(network_override: Option<String>) -> Self {
        Self {
            ctx_provider: ContextProvider::Lazy {
                cell: Arc::new(tokio::sync::OnceCell::new()),
                network_override,
            },
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
            ContextProvider::Lazy {
                cell,
                network_override,
            } => cell
                .get_or_try_init(|| async { init_app_context(network_override.as_deref()) })
                .await
                .cloned(),
        }
    }
}

/// Initialize an AppContext for standalone/CLI mode.
/// `network_override` from `--network` flag takes precedence over the database default.
#[cfg(feature = "cli")]
fn init_app_context(network_override: Option<&str>) -> Result<Arc<AppContext>, McpError> {
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

    // Network: --network flag > database setting > mainnet default.
    let network = if let Some(name) = network_override {
        parse_network_name(name)?
    } else {
        db.get_settings()
            .ok()
            .flatten()
            .map(|(network, ..)| network)
            .unwrap_or(Network::Dash)
    };

    let subtasks = Arc::new(TaskManager::new());
    let connection_status = Arc::new(ConnectionStatus::new());

    AppContext::new(
        data_dir,
        network,
        db,
        None, // no wallet passwords in MCP server
        subtasks,
        connection_status,
        egui::Context::default(),
    )
    .ok_or_else(|| McpError::internal_error("failed to create AppContext".to_string(), None))
}

/// Parse a network name string into a `Network` enum.
#[cfg(feature = "cli")]
fn parse_network_name(name: &str) -> Result<dash_sdk::dpp::dashcore::Network, McpError> {
    use dash_sdk::dpp::dashcore::Network;
    match name {
        "mainnet" | "dash" => Ok(Network::Dash),
        "testnet" => Ok(Network::Testnet),
        "devnet" => Ok(Network::Devnet),
        "regtest" => Ok(Network::Regtest),
        other => Err(McpError::internal_error(
            format!("unknown network: {other}. Use: mainnet, testnet, devnet, regtest"),
            None,
        )),
    }
}

#[tool_router]
impl DashMcpService {
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
