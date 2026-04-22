//! MCP server configuration from environment variables.

/// Minimum length for `MCP_API_KEY` to prevent weak secrets.
const MIN_API_KEY_LENGTH: usize = 16;

/// Configuration for the MCP server.
pub struct McpConfig {
    /// API key for HTTP bearer auth. Only used by HTTP mode.
    pub api_key: String,
    /// Listen address for the HTTP server. Only used by HTTP mode.
    pub listen_addr: String,
}

impl McpConfig {
    /// Read config for HTTP mode. Returns `None` when disabled (no API key).
    ///
    /// Logs an error and returns `None` if the key is shorter than 16 characters.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("MCP_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        if api_key.len() < MIN_API_KEY_LENGTH {
            tracing::error!(
                "MCP_API_KEY is too short ({} chars, minimum {}). \
                 MCP server will not start.",
                api_key.len(),
                MIN_API_KEY_LENGTH,
            );
            return None;
        }
        let listen_addr = std::env::var("MCP_LISTEN")
            .ok()
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "127.0.0.1:9527".to_string());
        Some(Self {
            api_key,
            listen_addr,
        })
    }
}
