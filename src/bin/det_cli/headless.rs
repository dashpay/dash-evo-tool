//! Headless HTTP MCP server daemon.

use std::sync::Arc;

/// Run det-cli as a headless HTTP MCP server.
/// Eagerly initializes AppContext, starts SPV, serves MCP tools over HTTP.
pub(super) fn run_headless() -> Result<(), Box<dyn std::error::Error>> {
    use dash_evo_tool::logging::initialize_logger;
    use dash_evo_tool::mcp::server::init_app_context;
    use dash_evo_tool::mcp::{McpConfig, start_http_server};

    // Require MCP_API_KEY -- headless without auth is not allowed.
    let config = McpConfig::from_env()
        .ok_or("MCP_API_KEY must be set for headless mode. Set it in .env or environment.")?;

    initialize_logger();
    tracing::info!(
        version = dash_evo_tool::VERSION,
        listen = %config.listen_addr,
        "Starting headless Dash Evo Tool (HTTP MCP server)"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let ctx = init_app_context()
            .await
            .map_err(|e| format!("Failed to initialize: {}", e.message))?;
        let swappable = Arc::new(arc_swap::ArcSwap::new(ctx));

        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_on_signal = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutting down...");
            cancel_on_signal.cancel();
        });

        start_http_server(swappable, config, cancel)
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { e })
    })
}
