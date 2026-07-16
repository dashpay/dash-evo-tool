//! HTTP bearer-auth contract for the MCP server.
//!
//! Pins the wire format the server's `bearer_auth` middleware accepts, so the
//! `det-cli` HTTP client and the headless server can never drift apart again.
//! Regression guard for the double-`Bearer` bug: rmcp's client `auth_header`
//! takes a raw token and prepends `Bearer ` itself, so a client that also
//! prepends puts `Bearer Bearer <token>` on the wire and is rejected.

#![cfg(feature = "mcp")]

use std::net::SocketAddr;

use axum::{Router, middleware, routing::get};
use dash_evo_tool::mcp::auth::{ApiKey, bearer_auth};

const TOKEN: &str = "test-api-key-0123456789";

async fn spawn_server() -> SocketAddr {
    let api_key = ApiKey(std::sync::Arc::from(TOKEN));
    let protected = Router::new()
        .route("/mcp", get(|| async { "ok" }))
        .route_layer(middleware::from_fn_with_state(api_key, bearer_auth));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            protected.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

/// The raw token — exactly what rmcp's `auth_header` puts through
/// reqwest's `bearer_auth` — is accepted.
#[tokio::test]
async fn raw_token_is_accepted() {
    let addr = spawn_server().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/mcp"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

/// A pre-prefixed token yields `Authorization: Bearer Bearer <token>` on the
/// wire and MUST be rejected — this is the exact shape of the old client bug.
#[tokio::test]
async fn double_bearer_prefix_is_rejected() {
    let addr = spawn_server().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/mcp"))
        .bearer_auth(format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// No credentials at all is rejected.
#[tokio::test]
async fn missing_authorization_is_rejected() {
    let addr = spawn_server().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/mcp"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}
