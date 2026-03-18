//! Standalone MCP server for Dash Evo Tool.
//!
//! Communicates via stdin/stdout using the MCP protocol.
//! AppContext is lazily initialized on first tool call.

use dash_evo_tool::logging::initialize_logger;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    initialize_logger();
    tracing::info!(
        version = dash_evo_tool::VERSION,
        "Starting Dash Evo Tool MCP server (stdio)"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    runtime.block_on(dash_evo_tool::mcp::start_stdio())
}
