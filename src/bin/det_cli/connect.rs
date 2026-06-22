use rmcp::ServiceExt;

use super::McpClient;

/// Format an rmcp ServiceError into a user-friendly string.
/// Extracts the message from McpError; passes other variants through as-is.
pub(super) fn format_service_error(e: rmcp::service::ServiceError) -> String {
    use rmcp::service::ServiceError;
    match e {
        ServiceError::McpError(mcp) => format!("{}  (code {})", mcp.message, mcp.code.0),
        other => other.to_string(),
    }
}

/// Run as a standalone MCP stdio server (replaces the separate dash-evo-tool-mcp binary).
///
/// Always terminates via [`std::process::exit`] rather than returning — this
/// bypasses Tokio runtime teardown and prevents coordinator OS threads
/// (`identity-sync`, `platform-address-sync`, `shielded-sync`) from panicking
/// when they poll `tokio::time::sleep` against a shutting-down timer wheel.
/// See `DashMcpService::shutdown_wallet_backend` for the full race analysis.
pub(super) fn run_stdio_server() -> ! {
    use dash_evo_tool::logging::initialize_logger;

    initialize_logger();
    tracing::info!(
        version = dash_evo_tool::VERSION,
        "Starting Dash Evo Tool MCP server (stdio)"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");

    // `start_stdio` drains the wallet backend's persister (quiesce) before
    // returning.  We do NOT call `runtime.shutdown_timeout` afterwards —
    // instead we hard-exit below so coordinator threads cannot race the
    // timer-wheel teardown.
    let result = runtime.block_on(dash_evo_tool::mcp::start_stdio());

    let exit_code: i32 = match result {
        Ok(()) => 0,
        Err(ref e) => {
            eprintln!("MCP server error: {e}");
            1
        }
    };

    use std::io::Write as _;
    let _ = std::io::stdout().lock().flush();
    let _ = std::io::stderr().lock().flush();
    // TODO(graceful-teardown): replace with normal return once WalletBackend::quiesce() joins coordinator threads.
    std::process::exit(exit_code);
}

pub(super) async fn connect_in_process() -> Result<McpClient, Box<dyn std::error::Error>> {
    use dash_evo_tool::mcp::server::DashMcpService;

    // Create two duplex byte channels, cross-connected:
    // client writes to a, server reads from a; server writes to b, client reads from b.
    let (client_read, server_write) = tokio::io::duplex(8192);
    let (server_read, client_write) = tokio::io::duplex(8192);

    // Spawn the MCP service in a background task.
    // .serve() returns a RunningService -- keep it alive with .waiting().
    let service = DashMcpService::new_lazy();
    tokio::spawn(async move {
        match service.serve((server_read, server_write)).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(e) => eprintln!("MCP service error: {e}"),
        }
    });

    let client = ().serve((client_read, client_write)).await?;
    Ok(client)
}

pub(super) async fn connect_http(
    addr: &str,
    bearer: Option<&str>,
) -> Result<McpClient, Box<dyn std::error::Error>> {
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };

    let mut config = StreamableHttpClientTransportConfig::with_uri(addr);
    if let Some(token) = bearer {
        config = config.auth_header(format!("Bearer {token}"));
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await?;
    Ok(client)
}
