//! MCP (Model Context Protocol) HTTP server for programmatic access.
//!
//! Gated by the `mcp` feature and `MCP_API_KEY` env var. When enabled,
//! embeds a streamable-HTTP MCP server that exposes backend tasks as tools.

pub mod auth;
pub mod config;
pub mod dispatch;
pub mod server;

pub use config::McpConfig;

use crate::context::AppContext;
use arc_swap::ArcSwap;
use auth::ApiKey;
use axum::{Router, middleware};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use server::DashMcpService;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Start the MCP HTTP server.
///
/// Binds to `config.listen_addr`, serves `/mcp` (authenticated) and
/// `/health` (unauthenticated). Shuts down gracefully on cancellation.
pub async fn start_server(
    app_context: Arc<ArcSwap<AppContext>>,
    config: McpConfig,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api_key = ApiKey(Arc::from(config.api_key.as_str()));

    // MCP service factory -- creates a new service instance per session
    let ctx = app_context.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(DashMcpService::new(ctx.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig {
            cancellation_token: cancel.clone(),
            ..Default::default()
        },
    );

    // Health endpoint (unauthenticated)
    let health = Router::new().route("/health", axum::routing::get(|| async { "OK" }));

    // MCP endpoint (authenticated)
    let mcp = Router::new()
        .nest_service("/mcp", mcp_service)
        .route_layer(middleware::from_fn_with_state(api_key, auth::bearer_auth));

    let app = health.merge(mcp);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("MCP server listening on {}", config.listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await?;

    tracing::info!("MCP server stopped");
    Ok(())
}
