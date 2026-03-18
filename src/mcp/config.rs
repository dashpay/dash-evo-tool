//! MCP server configuration from environment variables.

/// Configuration for the MCP server.
pub struct McpConfig {
    /// API key for HTTP bearer auth. Only used by HTTP mode.
    pub api_key: String,
    /// Listen address for the HTTP server. Only used by HTTP mode.
    pub listen_addr: String,
}

impl McpConfig {
    /// Read config for HTTP mode. Returns `None` when disabled (no API key).
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
