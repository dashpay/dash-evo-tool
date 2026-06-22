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
///
/// Runs until the stdio transport closes (client disconnects), then
/// gracefully stops the wallet backend — if one was started — before
/// returning.  This ensures the `platform-address-sync` / `identity-sync`
/// coordinator threads are quiesced while the Tokio runtime is still alive,
/// preventing panics from timer registration during runtime shutdown.
#[cfg(feature = "cli")]
pub async fn start_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use rmcp::ServiceExt;

    let service = server::DashMcpService::new_lazy();
    // Keep a clone so we can access the context after `serve()` moves `service`.
    // `DashMcpService` is cheaply cloneable (all fields are `Arc`-wrapped).
    let service_for_shutdown = service.clone();

    // S6: capture the server result WITHOUT short-circuiting via `?` so that
    // `shutdown_wallet_backend` is ALWAYS called regardless of whether `serve`
    // or `waiting` returns an error.  The `?` is deferred to after the shutdown.
    //
    // `serve().await` errors with `ServerInitializeError`; `waiting().await`
    // errors with `JoinError` and succeeds with `QuitReason` — both are
    // mapped to the function's `Box<dyn Error + Send + Sync>` return type.
    let result: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
        let server = service
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        server
            .waiting()
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
            .map(|_| ())
    }
    .await;

    // Quiesce the wallet backend (coordinator threads) before returning.
    // The caller's runtime is still alive at this point; the shutdown must
    // complete here, inside `block_on`, NOT during runtime drop.
    service_for_shutdown.shutdown_wallet_backend().await;

    result
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
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.clone()),
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
