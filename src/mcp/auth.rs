//! Bearer token authentication middleware for MCP endpoints.
//!
//! Only used in HTTP mode (`mcp` feature).

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Shared state for auth middleware.
#[derive(Clone)]
pub struct ApiKey(pub Arc<str>);

/// Axum middleware that validates `Authorization: Bearer <token>` headers.
/// Uses constant-time comparison to prevent timing attacks.
pub async fn bearer_auth(
    State(api_key): State<ApiKey>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let provided = &header["Bearer ".len()..];
            if provided.as_bytes().ct_eq(api_key.0.as_bytes()).into() {
                Ok(next.run(request).await)
            } else {
                tracing::warn!("MCP auth failed from {}", addr);
                Err(unauthorized_response())
            }
        }
        _ => {
            tracing::warn!("MCP auth failed from {}", addr);
            Err(unauthorized_response())
        }
    }
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({"error": "unauthorized"})),
    )
        .into_response()
}
