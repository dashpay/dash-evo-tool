//! MCP service definition — DashMcpService struct, context providers, and ServerHandler impl.

use crate::context::AppContext;
use crate::mcp::tools;
use rmcp::handler::server::tool::{ToolCallContext, ToolRouter};
use rmcp::model::*;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, service::RequestContext};
use std::sync::Arc;

/// Abstracts how the MCP service stores and swaps its AppContext.
/// Both variants support `load` and `store` for network switching.
#[derive(Clone)]
enum ContextHolder {
    /// HTTP mode: shared with the GUI app via the same `ArcSwap`.
    /// GUI calls `store()` on network switch; MCP sees it immediately.
    #[cfg(feature = "mcp")]
    Shared(Arc<arc_swap::ArcSwap<AppContext>>),
    /// Stdio/CLI mode: standalone, lazily initialized on first tool call.
    #[cfg(feature = "cli")]
    Standalone(Arc<arc_swap::ArcSwapOption<AppContext>>),
}

impl ContextHolder {
    fn load(&self) -> Option<Arc<AppContext>> {
        match self {
            #[cfg(feature = "mcp")]
            Self::Shared(swap) => Some(swap.load_full()),
            #[cfg(feature = "cli")]
            Self::Standalone(swap) => swap.load_full(),
        }
    }

    fn store(&self, ctx: Arc<AppContext>) {
        match self {
            #[cfg(feature = "mcp")]
            Self::Shared(swap) => swap.store(ctx),
            #[cfg(feature = "cli")]
            Self::Standalone(swap) => swap.store(Some(ctx)),
        }
    }

    /// Whether this holder needs lazy initialization before the first load.
    /// Only standalone (stdio/CLI) contexts start empty; shared (HTTP) contexts
    /// are pre-populated by the GUI. A `match` stays correct whether or not the
    /// `mcp` feature compiles in the `Shared` variant — unlike an `if let`, which
    /// becomes an irrefutable pattern in a `cli`-only build.
    #[cfg(feature = "cli")]
    fn needs_lazy_init(&self) -> bool {
        match self {
            #[cfg(feature = "mcp")]
            Self::Shared(_) => false,
            Self::Standalone(_) => true,
        }
    }
}

/// MCP service backed by the app's context.
///
/// HTTP mode shares the GUI's `ArcSwap` so network switches propagate
/// bidirectionally. Stdio/CLI mode uses a standalone `ArcSwapOption` with
/// lazy initialization. Both modes support `swap_context` for the
/// `network_switch` tool.
#[derive(Clone)]
pub struct DashMcpService {
    ctx: ContextHolder,
    /// Guards lazy initialization in stdio/CLI mode.
    #[cfg(feature = "cli")]
    init_guard: Arc<tokio::sync::OnceCell<()>>,
    pub(crate) tool_router: ToolRouter<DashMcpService>,
}

impl std::fmt::Debug for DashMcpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashMcpService").finish_non_exhaustive()
    }
}

impl DashMcpService {
    /// For HTTP mode: wrap the GUI's shared ArcSwap (same reference).
    #[cfg(feature = "mcp")]
    pub fn new_shared(app_context: Arc<arc_swap::ArcSwap<AppContext>>) -> Self {
        Self {
            ctx: ContextHolder::Shared(app_context),
            #[cfg(feature = "cli")]
            init_guard: Arc::new(tokio::sync::OnceCell::const_new()),
            tool_router: Self::tool_router(),
        }
    }

    /// For stdio/CLI mode: lazy init on first tool call.
    #[cfg(feature = "cli")]
    pub fn new_lazy() -> Self {
        Self {
            ctx: ContextHolder::Standalone(Arc::new(arc_swap::ArcSwapOption::empty())),
            init_guard: Arc::new(tokio::sync::OnceCell::new()),
            tool_router: Self::tool_router(),
        }
    }

    /// Get the current AppContext.
    ///
    /// In HTTP mode, loads from the shared ArcSwap (always initialized).
    /// In stdio/CLI mode, initializes on first call, then loads.
    pub(crate) async fn ctx(&self) -> Result<Arc<AppContext>, McpError> {
        #[cfg(feature = "cli")]
        if self.ctx.needs_lazy_init() {
            let ctx_holder = self.ctx.clone();
            self.init_guard
                .get_or_try_init(|| async {
                    let app_context = init_app_context().await.map_err(|e| {
                        tracing::error!("MCP context initialization failed: {e}");
                        McpError::internal_error("Failed to initialize application context", None)
                    })?;
                    ctx_holder.store(app_context);
                    Ok::<(), McpError>(())
                })
                .await?;
        }
        self.ctx
            .load()
            .ok_or_else(|| McpError::internal_error("AppContext not initialized", None))
    }

    /// Replace the active context. Used by `network_switch` to point the
    /// server at a newly created network context. Works in all modes.
    pub(crate) fn swap_context(&self, new_ctx: Arc<AppContext>) {
        self.ctx.store(new_ctx);
    }

    /// Drain the wallet backend's persister before process exit.
    ///
    /// Called from the standalone stdio serve path (`start_stdio`), inside
    /// `block_on` while the Tokio runtime is still alive.  Ensures any in-flight
    /// `TokenBalanceChangeSet` / `PlatformWalletChangeSet` persister writes issued
    /// by the coordinator sync loops complete before the process exits.
    ///
    /// ## Why this does NOT stop the coordinator timer panic
    ///
    /// Each coordinator (`identity-sync`, `platform-address-sync`, `shielded-sync`)
    /// runs on a **dedicated OS thread** that calls [`Handle::block_on`].  Their
    /// inner loop ends with:
    ///
    /// ```text
    /// tokio::select! {
    ///     _ = tokio::time::sleep(interval) => {}   // panics if runtime shut down
    ///     _ = cancel.cancelled()            => break,
    /// }
    /// ```
    ///
    /// `backend.shutdown()` → `quiesce()` cancels the tokens and waits for
    /// `is_syncing == false`, but **does not join the OS threads** — it returns
    /// as soon as the last persister write completes.  At that point the coordinator
    /// threads are still alive and may poll `sleep(interval)` in `select!`.
    ///
    /// `tokio::select!` picks arms in **random order** for fairness.  If
    /// `sleep(interval)` is polled before `cancel.cancelled()` (which is ready
    /// immediately) while the Tokio runtime is shutting down, `Sleep::poll`
    /// panics: *"A Tokio 1.x context was found, but it is being shutdown."*
    ///
    /// A `tokio::time::sleep` grace period was tried and also fails: DAPI retries
    /// that are already in flight when `quiesce()` returns can extend past any
    /// fixed sleep window.
    ///
    /// **The deterministic fix** is `std::process::exit` in the CLI entry points
    /// (`run_stdio_server`, `run_headless`, and the one-shot tool path in `main`),
    /// applied after the tool result is flushed to stdout.  `process::exit`
    /// reclaims all OS threads before they can poll the shutting-down timer wheel.
    /// The upstream fix (storing and joining the OS thread's `JoinHandle` in
    /// `quiesce()`) would be the correct library-level solution.
    ///
    /// ## Graceful teardown — plan for when upstream delivers
    ///
    /// Once `WalletBackend::quiesce()` (or a new `shutdown_and_join()` variant)
    /// joins the coordinator OS threads before returning, the `process::exit`
    /// stopgap can be removed from all three CLI call-sites.  The replacement
    /// would look like:
    ///
    /// ```text
    /// // TODO(graceful-teardown): remove process::exit once WalletBackend exposes
    /// // coordinator JoinHandles and quiesce() joins them before returning.
    ///
    /// // 1. Quiesce persister writes AND join all coordinator OS threads.
    /// backend.shutdown_and_join().await;
    ///
    /// // 2. At this point NO coordinator thread holds a Tokio timer registration,
    /// //    so the runtime can be dropped (or allowed to fall off the stack)
    /// //    without triggering the "context is being shutdown" panic.
    /// drop(runtime);   // or just let it fall out of scope
    ///
    /// // 3. Return normally — no hard-exit required.
    /// return result;
    /// ```
    ///
    /// Call-sites to update when the upstream fix lands:
    /// - `src/bin/det_cli/connect.rs`  — `run_stdio_server()`
    /// - `src/bin/det_cli/main.rs`     — one-shot tool path in `main()`
    /// - `src/bin/det_cli/headless.rs` — `run_headless()`
    ///
    /// ## Safe to call unconditionally
    ///
    /// - Context never initialized → `ctx.load()` returns `None` → no-op.
    /// - Context init'd, backend never wired → `wallet_backend()` returns
    ///   `Err(WalletBackendNotYetWired)` → no-op (no coordinators were started).
    #[cfg(feature = "cli")]
    pub async fn shutdown_wallet_backend(&self) {
        let Some(ctx) = self.ctx.load() else { return };
        let Ok(backend) = ctx.wallet_backend() else {
            return;
        };
        // Drain in-flight persister writes.  Does not join coordinator threads.
        backend.shutdown().await;
    }

    /// Build the tool router using trait-based tool composition.
    pub fn tool_router() -> ToolRouter<Self> {
        ToolRouter::new()
            .with_async_tool::<tools::network::NetworkTool>()
            .with_async_tool::<tools::network::NetworkReinitSdk>()
            .with_async_tool::<tools::network::NetworkSwitch>()
            .with_async_tool::<tools::wallet::ListWalletsTool>()
            .with_async_tool::<tools::wallet::ImportWallet>()
            .with_async_tool::<tools::wallet::GenerateReceiveAddress>()
            .with_async_tool::<tools::wallet::WalletBalancesQuery>()
            .with_async_tool::<tools::wallet::FetchPlatformBalances>()
            .with_async_tool::<tools::wallet::SendCoreFunds>()
            .with_async_tool::<tools::platform::QueryWithdrawals>()
            .with_async_tool::<tools::meta::DescribeTool>()
            // Identity tools
            .with_async_tool::<tools::identity::IdentityCreditsTopup>()
            .with_async_tool::<tools::identity::IdentityCreditsTopupFromPlatform>()
            .with_async_tool::<tools::identity::IdentityCreditsTransfer>()
            .with_async_tool::<tools::identity::IdentityCreditsWithdraw>()
            .with_async_tool::<tools::identity::IdentityCreditsToAddress>()
            // Masternode / evonode tools
            .with_async_tool::<tools::masternode::MasternodeIdentityLoad>()
            .with_async_tool::<tools::masternode::MasternodeCreditsWithdraw>()
            // Shielded tools
            .with_async_tool::<tools::shielded::ShieldedShieldFromCore>()
            .with_async_tool::<tools::shielded::ShieldedShieldFromPlatform>()
            .with_async_tool::<tools::shielded::ShieldedTransferTool>()
            .with_async_tool::<tools::shielded::ShieldedUnshield>()
            .with_async_tool::<tools::shielded::ShieldedWithdrawTool>()
            // Shielded read/control tools (Phase G — agent self-verification)
            .with_async_tool::<tools::shielded::ShieldedInit>()
            .with_async_tool::<tools::shielded::ShieldedSync>()
            .with_async_tool::<tools::shielded::ShieldedBalanceGet>()
            .with_async_tool::<tools::shielded::ShieldedAddressGet>()
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

    let app_kv = AppContext::open_app_kv(&data_dir)
        .map_err(|e| McpError::internal_error(format!("app k/v open: {e}"), None))?;
    let secret_store = AppContext::open_secret_store(&data_dir).map_err(|e| {
        // A legacy passphrase-protected vault can only be unlocked through the
        // GUI's boot prompt — name the real cause and the path forward instead
        // of the generic "another copy is running" lock message.
        if e.is_secret_store_wrong_passphrase() {
            McpError::internal_error(
                "Your saved keys are protected by a passphrase set in an earlier version. \
                 Open the Dash Evo Tool desktop app and enter the passphrase to unlock them, \
                 then run this command again."
                    .to_string(),
                None,
            )
        } else {
            McpError::internal_error(format!("secret store open: {e}"), None)
        }
    })?;
    let network = app_kv
        .get::<crate::model::settings::AppSettings>(
            crate::wallet_backend::DetScope::Global,
            crate::model::settings::AppSettings::KV_KEY,
        )
        .ok()
        .flatten()
        .map(|s| s.network)
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
        subtasks,
        connection_status,
        egui::Context::default(),
        app_kv,
        secret_store,
    )
    .ok_or_else(|| {
        McpError::internal_error(
            "failed to create AppContext -- check logs for details".to_string(),
            None,
        )
    })?;

    // Chain sync is SPV-only (owned by upstream platform-wallet). Starting it
    // here would fast-fail: the wallet backend is not wired yet at boot. SPV is
    // instead wired-then-started lazily by `resolve::ensure_spv_synced` on the
    // first gated tool call — the single chokepoint that also covers the HTTP
    // context swap and the post-network-switch path.
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
