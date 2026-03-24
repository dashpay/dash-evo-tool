//! MCP (Model Context Protocol) server for programmatic access.
//!
//! Two modes, each behind its own feature flag:
//! - `mcp`: HTTP server embedded in the GUI app, shares AppContext via ArcSwap
//! - `cli`: standalone CLI with in-process MCP service + HTTP client mode

#[cfg(feature = "mcp")]
pub mod auth;
pub mod config;
pub mod dispatch;
pub mod error;
pub mod resolve;
pub mod server;
pub mod tools;

#[cfg(test)]
mod tests;

pub use config::McpConfig;

/// Start the MCP server over stdin/stdout.
#[cfg(feature = "cli")]
pub async fn start_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use rmcp::ServiceExt;

    let service = server::DashMcpService::new_lazy();
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}

/// Start the MCP server over HTTP (embedded in GUI app).
#[cfg(feature = "mcp")]
pub async fn start_http_server(
    app_context: std::sync::Arc<arc_swap::ArcSwap<crate::context::AppContext>>,
    config: McpConfig,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use auth::ApiKey;
    use axum::{Router, middleware};
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use server::DashMcpService;
    use std::sync::Arc;

    let api_key = ApiKey(Arc::from(config.api_key.as_str()));

    let ctx = app_context.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(DashMcpService::new_shared(ctx.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig {
            cancellation_token: cancel.clone(),
            ..Default::default()
        },
    );

    let health = Router::new().route("/health", axum::routing::get(|| async { "OK" }));

    let mcp = Router::new()
        .nest_service("/mcp", mcp_service)
        .route_layer(middleware::from_fn_with_state(api_key, auth::bearer_auth));

    let app = health.merge(mcp);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("MCP server listening on {}", config.listen_addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(cancel.cancelled_owned())
    .await?;

    tracing::info!("MCP server stopped");
    Ok(())
}
