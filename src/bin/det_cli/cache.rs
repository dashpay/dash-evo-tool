use std::path::PathBuf;

use rmcp::model::Tool;

use super::McpClient;
use super::PKG_VERSION;

/// Versioned tool cache stored on disk.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ToolCache {
    pub(super) version: String,
    pub(super) tools: Vec<Tool>,
}

pub(super) fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("org", "dash", "det-cli")
        .map(|p| p.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".det-cli-cache"))
}

pub(super) fn cache_path() -> PathBuf {
    cache_dir().join("tools.json")
}

pub(super) fn load_cache() -> Option<ToolCache> {
    let data = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save tools to cache and install shell completion.
/// Extracts server version from the peer info.
pub(super) fn save_cache(client: &McpClient, tools: &[Tool]) {
    let version = client
        .peer()
        .peer_info()
        .map(|info| info.server_info.version.clone())
        .unwrap_or_else(|| PKG_VERSION.to_string());

    let cache = ToolCache {
        version,
        tools: tools.to_vec(),
    };

    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = serde_json::to_string_pretty(&cache).map(|json| std::fs::write(cache_path(), json));
    }

    super::completion::install_bash_completion();
}
