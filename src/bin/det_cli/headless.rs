//! Headless HTTP MCP server daemon.

/// Run det-cli as a headless HTTP MCP server.
///
/// Eagerly initializes AppContext, starts SPV, serves MCP tools over HTTP.
/// Terminates via [`std::process::exit`] on clean shutdown — bypassing Tokio
/// runtime teardown to prevent coordinator OS threads from panicking against a
/// shutting-down timer wheel.  See `DashMcpService::shutdown_wallet_backend`
/// for the race analysis.
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

    let result: Result<(), Box<dyn std::error::Error>> = runtime.block_on(async {
        let ctx = init_app_context()
            .await
            .map_err(|e| format!("Failed to initialize: {}", e.message))?;
        let swappable = std::sync::Arc::new(arc_swap::ArcSwap::new(ctx));

        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_on_signal = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutting down...");
            cancel_on_signal.cancel();
        });

        let result = start_http_server(swappable.clone(), config, cancel)
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { e });

        // Drain the wallet backend's persister.  `swappable.load_full()` yields
        // the CURRENTLY active context; network_switch already drained the
        // outgoing backend at swap time (see NetworkSwitch::invoke in
        // src/mcp/tools/network.rs), so only the current context needs
        // draining here.
        let current_ctx = swappable.load_full();
        if let Ok(backend) = current_ctx.wallet_backend() {
            backend.shutdown().await;
        }

        result
    });

    // Hard-exit: bypass runtime teardown to prevent coordinator OS threads from
    // panicking against the shutting-down timer wheel.
    let exit_code: i32 = match result {
        Ok(()) => 0,
        Err(ref e) => {
            eprintln!("Headless server error: {e}");
            1
        }
    };
    use std::io::Write as _;
    let _ = std::io::stdout().lock().flush();
    let _ = std::io::stderr().lock().flush();
    std::process::exit(exit_code);
}
