//! Bearer token authentication middleware for MCP endpoints.
//!
//! Only used in HTTP mode (`mcp` feature).

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Shared state for auth middleware.
#[derive(Clone)]
pub struct ApiKey(pub Arc<str>);

/// Axum middleware that validates `Authorization: Bearer <token>` headers.
/// Uses constant-time comparison to prevent timing attacks.
pub async fn bearer_auth(
    State(api_key): State<ApiKey>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
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
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
