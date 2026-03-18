//! MCP server configuration from environment variables.

/// Configuration for the MCP HTTP server.
///
/// Loaded from environment variables. Returns `None` if `MCP_API_KEY` is
/// empty or missing -- the server simply won't start.
pub struct McpConfig {
    pub api_key: String,
    pub listen_addr: String,
}

impl McpConfig {
    /// Read config from env. Returns `None` when disabled (no API key).
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("MCP_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
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
