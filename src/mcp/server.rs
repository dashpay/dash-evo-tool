//! MCP service definition — DashMcpService struct, context providers, and ServerHandler impl.

use crate::context::AppContext;
use crate::mcp::tools;
use rmcp::handler::server::tool::{ToolCallContext, ToolRouter};
use rmcp::model::*;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, service::RequestContext};
use std::sync::Arc;

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
    pub(crate) tool_router: ToolRouter<DashMcpService>,
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
    ///
    /// Each tool must call this exactly once and pass the resulting `Arc` to
    /// both validation and the operation to avoid TOCTOU issues with ArcSwap.
    pub(crate) async fn ctx(&self) -> Result<Arc<AppContext>, McpError> {
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

    /// Build the tool router using trait-based tool composition.
    pub fn tool_router() -> ToolRouter<Self> {
        ToolRouter::new()
            .with_async_tool::<tools::network::NetworkTool>()
            .with_async_tool::<tools::wallet::ListWalletsTool>()
            .with_async_tool::<tools::wallet::GenerateReceiveAddress>()
            .with_async_tool::<tools::wallet::WalletBalancesQuery>()
            .with_async_tool::<tools::wallet::FetchPlatformBalances>()
            .with_async_tool::<tools::wallet::WalletFundsSend>()
            .with_async_tool::<tools::platform::QueryWithdrawals>()
            .with_async_tool::<tools::meta::DescribeTool>()
    }
}

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

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tcc = ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

/// Initialize an AppContext for standalone/CLI mode.
/// Uses the last network selected in the GUI (from database), defaults to mainnet.
/// Starts the SPV client so wallet operations work, but does not block waiting
/// for wallets to load -- individual tools wait for their specific wallet.
#[cfg(feature = "cli")]
pub async fn init_app_context() -> Result<Arc<AppContext>, McpError> {
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
    if env_path.exists()
        && let Err(e) = dotenvy::from_path(&env_path)
    {
        tracing::warn!("Failed to load .env file at {}: {e}", env_path.display());
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
        .unwrap_or(Network::Mainnet);

    let subtasks = Arc::new(TaskManager::new());
    let connection_status = Arc::new(ConnectionStatus::new());

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
            "failed to create AppContext -- check logs for details".to_string(),
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
pub(crate) fn collect_available(config: &crate::config::Config) -> Vec<&'static str> {
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
pub(crate) fn network_display_name(network: dash_sdk::dpp::dashcore::Network) -> &'static str {
    use dash_sdk::dpp::dashcore::Network;
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "local",
        _ => "unknown",
    }
}

/// Return a comma-separated list of configured network names.
fn available_network_names(config: &crate::config::Config) -> String {
    let names = collect_available(config);
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}
