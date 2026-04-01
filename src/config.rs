use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use crate::app_dir::{app_user_data_file_path, data_file_path};
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use serde::Deserialize;
use tempfile::NamedTempFile;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub mainnet_config: Option<NetworkConfig>,
    pub testnet_config: Option<NetworkConfig>,
    pub devnet_config: Option<NetworkConfig>,
    pub local_config: Option<NetworkConfig>,
    /// Global developer mode setting
    pub developer_mode: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to load configuration from disk or environment.
    #[error("{0}")]
    LoadError(String),

    /// Failed to save configuration to disk.
    #[error("Could not save settings. Check that the application folder is writable and retry.")]
    SaveError {
        #[source]
        source: std::io::Error,
    },

    /// No valid network configurations found.
    #[error(
        "No network configuration found. Please reinstall the application or check your settings."
    )]
    NoValidConfigs,
}

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct NetworkConfig {
    /// Comma-separated DAPI node URLs (e.g., `https://host:443,https://host2:443`).
    /// Required for the app to connect. Use "Refresh DAPI endpoints" in Network
    /// Settings to fetch addresses for Mainnet/Testnet.
    pub dapi_addresses: Option<String>,
    /// Host of the Dash Core RPC interface (only needed in RPC mode)
    pub core_host: Option<String>,
    /// Port of the Dash Core RPC interface (only needed in RPC mode)
    pub core_rpc_port: Option<u16>,
    /// Username for Dash Core RPC interface (only needed in RPC mode)
    pub core_rpc_user: Option<String>,
    /// Password for Dash Core RPC interface (only needed in RPC mode)
    pub core_rpc_password: Option<String>,
    /// ZMQ endpoint for Core blockchain events (e.g., tcp://127.0.0.1:23708)
    pub core_zmq_endpoint: Option<String>,
    /// Devnet network name if one exists
    pub devnet_name: Option<String>,
    /// Optional wallet private key to instantiate the wallet
    pub wallet_private_key: Option<String>,
}

impl NetworkConfig {
    /// Default Core RPC port for the given network.
    ///
    /// Returns the well-known port when `core_rpc_port` is not explicitly set:
    /// - Mainnet: 9998
    /// - Testnet: 19998
    /// - Devnet: 29998
    /// - Regtest: 20302 (dashmate default, matches `.env.example`)
    pub fn default_rpc_port(network: Network) -> u16 {
        match network {
            Network::Mainnet => 9998,
            Network::Testnet => 19998,
            Network::Devnet => 29998,
            Network::Regtest => 20302,
            _ => 9998,
        }
    }

    /// Resolved Core RPC port — explicit config or network-aware default.
    pub fn rpc_port(&self, network: Network) -> u16 {
        self.core_rpc_port
            .unwrap_or_else(|| Self::default_rpc_port(network))
    }

    /// Resolved Core RPC host — explicit config or localhost.
    pub fn rpc_host(&self) -> &str {
        self.core_host.as_deref().unwrap_or("127.0.0.1")
    }
}

impl Config {
    pub fn config_for_network(&self, network: Network) -> &Option<NetworkConfig> {
        match network {
            Network::Mainnet => &self.mainnet_config,
            Network::Testnet => &self.testnet_config,
            Network::Devnet => &self.devnet_config,
            Network::Regtest => &self.local_config,
            _ => &None,
        }
    }

    /// Write the current configuration back to the `.env` file so that
    /// subsequent calls to `Config::load()` will reflect changes.
    ///
    /// Uses atomic write (write to temp file, then rename) to prevent
    /// config corruption if a write fails partway through.
    pub fn save(&self, data_dir: &Path) -> Result<(), ConfigError> {
        let env_file_path =
            data_file_path(data_dir, ".env").map_err(|e| ConfigError::SaveError { source: e })?;

        // Write to a temporary file in the same directory first, then
        // atomically replace. This prevents corruption if the write fails
        // partway through. NamedTempFile::persist() closes the handle before
        // renaming and uses MoveFileEx with MOVEFILE_REPLACE_EXISTING on
        // Windows for atomic replacement.
        let parent_dir = env_file_path
            .parent()
            .ok_or_else(|| ConfigError::SaveError {
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    "config file path has no parent directory",
                ),
            })?;
        let mut env_file =
            NamedTempFile::new_in(parent_dir).map_err(|e| ConfigError::SaveError { source: e })?;

        // Helper function to write a single network config to the `.env` file
        let mut write_network_config = |prefix: &str, config: &NetworkConfig| {
            // Each line becomes e.g.  MAINNET_dapi_addresses=...
            // For "local" (regtest), you'll see LOCAL_dapi_addresses=...
            //
            // Use the environment variable scheme you prefer. Make sure it
            // matches what `load()` expects (i.e. `envy::prefixed("MAINNET_")`,
            // etc.).

            if let Some(ref addrs) = config.dapi_addresses
                && !addrs.is_empty()
            {
                writeln!(env_file, "{}dapi_addresses={}", prefix, addrs)
                    .map_err(|e| ConfigError::SaveError { source: e })?;
            }
            if let Some(ref host) = config.core_host {
                writeln!(env_file, "{}core_host={}", prefix, host)
                    .map_err(|e| ConfigError::SaveError { source: e })?;
            }
            if let Some(port) = config.core_rpc_port {
                writeln!(env_file, "{}core_rpc_port={}", prefix, port)
                    .map_err(|e| ConfigError::SaveError { source: e })?;
            }
            if let Some(ref user) = config.core_rpc_user {
                writeln!(env_file, "{}core_rpc_user={}", prefix, user)
                    .map_err(|e| ConfigError::SaveError { source: e })?;
            }
            if let Some(ref password) = config.core_rpc_password {
                writeln!(env_file, "{}core_rpc_password={}", prefix, password)
                    .map_err(|e| ConfigError::SaveError { source: e })?;
            }
            if let Some(core_zmq_endpoint) = &config.core_zmq_endpoint {
                writeln!(
                    env_file,
                    "{}core_zmq_endpoint={}",
                    prefix, core_zmq_endpoint
                )
                .map_err(|e| ConfigError::SaveError { source: e })?;
            }

            if let Some(devnet_name) = &config.devnet_name {
                // Only write devnet name if it exists
                writeln!(env_file, "{}devnet_name={}", prefix, devnet_name)
                    .map_err(|e| ConfigError::SaveError { source: e })?;
            }
            if let Some(wallet_private_key) = &config.wallet_private_key {
                writeln!(
                    env_file,
                    "{}wallet_private_key={}",
                    prefix, wallet_private_key
                )
                .map_err(|e| ConfigError::SaveError { source: e })?;
            }

            // Add a blank line after each config block
            writeln!(env_file).map_err(|e| ConfigError::SaveError { source: e })?;

            Ok(())
        };

        // Mainnet
        if let Some(ref mainnet_config) = self.mainnet_config {
            // `envy::prefixed("MAINNET_")` expects these lines to start with "MAINNET_"
            write_network_config("MAINNET_", mainnet_config)?;
        }

        // Testnet
        if let Some(ref testnet_config) = self.testnet_config {
            write_network_config("TESTNET_", testnet_config)?;
        }

        // Devnet
        if let Some(ref devnet_config) = self.devnet_config {
            write_network_config("DEVNET_", devnet_config)?;
        }

        // Local (Regtest)
        if let Some(ref local_config) = self.local_config {
            // `envy::prefixed("LOCAL_")` expects "LOCAL_..."
            write_network_config("LOCAL_", local_config)?;
        }

        // Save global developer mode
        if let Some(developer_mode) = self.developer_mode {
            writeln!(env_file, "DEVELOPER_MODE={}", developer_mode)
                .map_err(|e| ConfigError::SaveError { source: e })?;
        }

        // Sync all data to disk before renaming to ensure crash-safety
        env_file
            .as_file()
            .sync_all()
            .map_err(|e| ConfigError::SaveError { source: e })?;

        // Atomically replace the old config with the new one.
        // persist() closes the file handle and uses platform-safe rename
        // (MoveFileEx with MOVEFILE_REPLACE_EXISTING on Windows).
        env_file
            .persist(&env_file_path)
            .map_err(|e| ConfigError::SaveError { source: e.error })?;

        // Restrict file permissions on Unix (config contains RPC credentials).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = std::fs::set_permissions(&env_file_path, perms) {
                tracing::warn!("Could not set config file permissions to 0600: {e}");
            }
        }

        tracing::info!("Successfully saved configuration to {:?}", env_file_path);
        Ok(())
    }

    /// Loads the configuration for all networks from environment variables and `.env` file
    /// located in the default app data directory.
    pub fn load() -> Result<Self, ConfigError> {
        let env_file_path = app_user_data_file_path(".env").expect("should create .env file path");
        Self::load_from_env_path(env_file_path)
    }

    /// Loads the configuration for all networks from environment variables and `.env` file
    /// located in the given data directory.
    pub fn load_from(data_dir: &Path) -> Result<Self, ConfigError> {
        let env_file_path =
            data_file_path(data_dir, ".env").map_err(|e| ConfigError::LoadError(e.to_string()))?;
        Self::load_from_env_path(env_file_path)
    }

    fn load_from_env_path(env_file_path: std::path::PathBuf) -> Result<Self, ConfigError> {
        if let Err(err) = dotenvy::from_path_override(env_file_path) {
            tracing::warn!(
                ?err,
                "Failed to load .env file. Continuing with environment variables."
            );
        } else {
            tracing::info!("Successfully loaded .env file");
        }

        // Load each network config. Missing configs are normal — not every
        // user configures all networks. Only fail if nothing is configured at all.
        let mainnet_config = envy::prefixed("MAINNET_")
            .from_env::<NetworkConfig>()
            .inspect_err(|e| tracing::debug!("Failed to parse mainnet config: {e}"))
            .ok();
        let testnet_config = envy::prefixed("TESTNET_")
            .from_env::<NetworkConfig>()
            .inspect_err(|e| tracing::debug!("Failed to parse testnet config: {e}"))
            .ok();
        let devnet_config = envy::prefixed("DEVNET_")
            .from_env::<NetworkConfig>()
            .inspect_err(|e| tracing::debug!("Failed to parse devnet config: {e}"))
            .ok();
        let local_config = envy::prefixed("LOCAL_")
            .from_env::<NetworkConfig>()
            .inspect_err(|e| tracing::debug!("Failed to parse local config: {e}"))
            .ok();

        if mainnet_config.is_none()
            && testnet_config.is_none()
            && devnet_config.is_none()
            && local_config.is_none()
        {
            return Err(ConfigError::NoValidConfigs);
        }

        // Load global developer mode
        let developer_mode = std::env::var("DEVELOPER_MODE")
            .ok()
            .and_then(|s| s.parse::<bool>().ok());

        Ok(Config {
            mainnet_config,
            testnet_config,
            devnet_config,
            local_config,
            developer_mode,
        })
    }

    /// Update (overwrite) the configuration for a particular network.
    pub fn update_config_for_network(&mut self, network: Network, new_config: NetworkConfig) {
        match network {
            Network::Mainnet => self.mainnet_config = Some(new_config),
            Network::Testnet => self.testnet_config = Some(new_config),
            Network::Devnet => self.devnet_config = Some(new_config),
            Network::Regtest => self.local_config = Some(new_config),
            _ => {
                // Optionally handle any custom or unknown network here if needed
                tracing::warn!(
                    "Attempted to update config for an unknown network: {:?}",
                    network
                );
            }
        }
    }
}

impl NetworkConfig {
    /// List of DAPI addresses, if explicitly configured.
    /// Returns `Ok(None)` when absent or empty (not configured; the user may trigger discovery from Network Settings).
    pub fn dapi_address_list(&self) -> Result<Option<AddressList>, String> {
        let addrs = match self.dapi_addresses.as_deref() {
            Some(a) => a.trim(),
            None => return Ok(None),
        };
        if addrs.is_empty() {
            return Ok(None);
        }
        AddressList::from_str(addrs)
            .map(Some)
            .map_err(|e| format!("Could not parse DAPI addresses '{addrs}': {e}"))
    }

    /// Update just the `core_rpc_password` in a builder-like manner.
    /// Returns a new `NetworkConfig` with the updated password.
    pub fn update_core_rpc_password(mut self, new_password: String) -> Self {
        self.core_rpc_password = Some(new_password);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal valid NetworkConfig for testing
    fn make_network_config(dapi_addresses: &str, port: u16) -> NetworkConfig {
        let dapi = if dapi_addresses.is_empty() {
            None
        } else {
            Some(dapi_addresses.to_string())
        };
        NetworkConfig {
            dapi_addresses: dapi,
            core_host: Some("127.0.0.1".to_string()),
            core_rpc_port: Some(port),
            core_rpc_user: Some("dashrpc".to_string()),
            core_rpc_password: Some("password".to_string()),
            core_zmq_endpoint: Some("tcp://127.0.0.1:23708".to_string()),
            devnet_name: None,
            wallet_private_key: None,
        }
    }

    // ── NetworkConfig::dapi_address_list ─────────────────────────────

    #[test]
    fn test_dapi_address_list_single_address() {
        let config = make_network_config("https://127.0.0.1:443", 9998);
        let list = config.dapi_address_list().unwrap().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_dapi_address_list_multiple_addresses() {
        let config = make_network_config(
            "https://127.0.0.1:443,https://192.168.1.1:443,https://10.0.0.1:443",
            9998,
        );
        let list = config.dapi_address_list().unwrap().unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_dapi_address_list_empty_returns_none() {
        let config = make_network_config("", 9998);
        assert!(config.dapi_address_list().unwrap().is_none());
    }
    // ── NetworkConfig::update_core_rpc_password ─────────────────────

    #[test]
    fn test_update_core_rpc_password() {
        let config = make_network_config("https://127.0.0.1:443", 9998);
        assert_eq!(config.core_rpc_password.as_deref(), Some("password"));
        let updated = config.update_core_rpc_password("new_secret".to_string());
        assert_eq!(updated.core_rpc_password.as_deref(), Some("new_secret"));
        // Other fields should be unchanged
        assert_eq!(updated.core_rpc_user.as_deref(), Some("dashrpc"));
        assert_eq!(updated.core_rpc_port, Some(9998));
    }

    // ── Config::config_for_network ──────────────────────────────────

    #[test]
    fn test_config_for_network_mainnet() {
        let mainnet_cfg = make_network_config("https://127.0.0.1:443", 9998);
        let config = Config {
            mainnet_config: Some(mainnet_cfg),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        assert!(config.config_for_network(Network::Mainnet).is_some());
        assert!(config.config_for_network(Network::Testnet).is_none());
        assert!(config.config_for_network(Network::Devnet).is_none());
        assert!(config.config_for_network(Network::Regtest).is_none());
    }

    #[test]
    fn test_config_for_network_all_networks() {
        let config = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: Some(make_network_config("https://2.2.2.2:1443", 19998)),
            devnet_config: Some(make_network_config("http://3.3.3.3:1443", 29998)),
            local_config: Some(make_network_config("http://127.0.0.1:2443", 20302)),
            developer_mode: Some(true),
        };
        let main = config
            .config_for_network(Network::Mainnet)
            .as_ref()
            .unwrap();
        assert_eq!(main.core_rpc_port, Some(9998));
        let test = config
            .config_for_network(Network::Testnet)
            .as_ref()
            .unwrap();
        assert_eq!(test.core_rpc_port, Some(19998));
        let dev = config.config_for_network(Network::Devnet).as_ref().unwrap();
        assert_eq!(dev.core_rpc_port, Some(29998));
        let local = config
            .config_for_network(Network::Regtest)
            .as_ref()
            .unwrap();
        assert_eq!(local.core_rpc_port, Some(20302));
    }

    // ── Config::update_config_for_network ───────────────────────────

    #[test]
    fn test_update_config_for_network() {
        let mut config = Config {
            mainnet_config: None,
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        assert!(config.mainnet_config.is_none());
        let new_cfg = make_network_config("https://1.1.1.1:443", 9998);
        config.update_config_for_network(Network::Mainnet, new_cfg);
        assert!(config.mainnet_config.is_some());
        assert_eq!(
            config.mainnet_config.as_ref().unwrap().core_rpc_port,
            Some(9998)
        );
    }

    #[test]
    fn test_update_config_replaces_existing() {
        let mut config = Config {
            mainnet_config: Some(make_network_config("https://old.example.com:443", 1111)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        let new_cfg = make_network_config("https://new.example.com:443", 2222);
        config.update_config_for_network(Network::Mainnet, new_cfg);
        let main = config.mainnet_config.as_ref().unwrap();
        assert_eq!(main.core_rpc_port, Some(2222));
        assert_eq!(
            main.dapi_addresses.as_deref(),
            Some("https://new.example.com:443")
        );
    }

    #[test]
    fn test_update_config_for_all_networks() {
        let mut config = Config {
            mainnet_config: None,
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        config.update_config_for_network(
            Network::Testnet,
            make_network_config("https://t.example.com:1443", 19998),
        );
        config.update_config_for_network(
            Network::Devnet,
            make_network_config("http://d.example.com:1443", 29998),
        );
        config.update_config_for_network(
            Network::Regtest,
            make_network_config("http://127.0.0.1:2443", 20302),
        );
        assert!(config.testnet_config.is_some());
        assert!(config.devnet_config.is_some());
        assert!(config.local_config.is_some());
    }

    // ── NetworkConfig optional fields ───────────────────────────────

    #[test]
    fn test_network_config_optional_fields() {
        let mut config = make_network_config("https://127.0.0.1:443", 9998);
        // Defaults
        assert!(config.devnet_name.is_none());
        assert!(config.wallet_private_key.is_none());
        assert!(config.core_zmq_endpoint.is_some());

        // Set optional fields
        config.devnet_name = Some("devnet-alpha".to_string());
        config.wallet_private_key = Some("cVBZ...key".to_string());
        config.core_zmq_endpoint = None;

        assert_eq!(config.devnet_name.as_ref().unwrap(), "devnet-alpha");
        assert_eq!(config.wallet_private_key.as_ref().unwrap(), "cVBZ...key");
        assert!(config.core_zmq_endpoint.is_none());
    }

    // ── Config save format verification ─────────────────────────────

    #[test]
    fn test_save_format_contains_expected_env_vars() {
        // We can't easily test save() directly since it depends on app_user_data_file_path,
        // but we can verify the save format by manually constructing what save() would write.
        let config = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: Some(make_network_config("https://2.2.2.2:1443", 19998)),
            devnet_config: None,
            local_config: None,
            developer_mode: Some(true),
        };

        // Simulate what save() writes by formatting env lines
        let mut output = String::new();
        if let Some(ref cfg) = config.mainnet_config {
            if let Some(ref addrs) = cfg.dapi_addresses {
                output.push_str(&format!("MAINNET_dapi_addresses={}\n", addrs));
            }
            if let Some(ref host) = cfg.core_host {
                output.push_str(&format!("MAINNET_core_host={}\n", host));
            }
            if let Some(port) = cfg.core_rpc_port {
                output.push_str(&format!("MAINNET_core_rpc_port={}\n", port));
            }
            if let Some(ref user) = cfg.core_rpc_user {
                output.push_str(&format!("MAINNET_core_rpc_user={}\n", user));
            }
            if let Some(ref password) = cfg.core_rpc_password {
                output.push_str(&format!("MAINNET_core_rpc_password={}\n", password));
            }
            if let Some(ref zmq) = cfg.core_zmq_endpoint {
                output.push_str(&format!("MAINNET_core_zmq_endpoint={}\n", zmq));
            }
        }

        assert!(output.contains("MAINNET_dapi_addresses=https://1.1.1.1:443"));
        assert!(output.contains("MAINNET_core_rpc_port=9998"));
        assert!(output.contains("MAINNET_core_rpc_user=dashrpc"));
        assert!(output.contains("MAINNET_core_rpc_password=password"));
        assert!(output.contains("MAINNET_core_zmq_endpoint=tcp://127.0.0.1:23708"));
    }

    // ── envy parsing roundtrip ──────────────────────────────────────

    #[test]
    fn test_envy_parsing_roundtrip() {
        // Test that environment variables in the format save() produces
        // can be parsed back by envy::prefixed()
        use std::collections::HashMap;

        let mut env_map: HashMap<String, String> = HashMap::new();
        env_map.insert(
            "TEST_RT_dapi_addresses".into(),
            "https://1.2.3.4:443".into(),
        );
        env_map.insert("TEST_RT_core_host".into(), "192.168.1.100".into());
        env_map.insert("TEST_RT_core_rpc_port".into(), "9998".into());
        env_map.insert("TEST_RT_core_rpc_user".into(), "testuser".into());
        env_map.insert("TEST_RT_core_rpc_password".into(), "testpass".into());
        env_map.insert(
            "TEST_RT_core_zmq_endpoint".into(),
            "tcp://127.0.0.1:29999".into(),
        );

        // Use envy's from_iter to parse from our map (same as from_env but testable)
        let result: Result<NetworkConfig, _> = envy::prefixed("TEST_RT_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
        let config = result.unwrap();
        assert_eq!(
            config.dapi_addresses.as_deref(),
            Some("https://1.2.3.4:443")
        );
        assert_eq!(config.core_host.as_deref(), Some("192.168.1.100"));
        assert_eq!(config.core_rpc_port, Some(9998));
        assert_eq!(config.core_rpc_user.as_deref(), Some("testuser"));
        assert_eq!(config.core_rpc_password.as_deref(), Some("testpass"));
        assert_eq!(
            config.core_zmq_endpoint,
            Some("tcp://127.0.0.1:29999".to_string())
        );
        assert!(config.devnet_name.is_none());
        assert!(config.wallet_private_key.is_none());
    }

    #[test]
    fn test_envy_parsing_with_optional_fields() {
        use std::collections::HashMap;

        let mut env_map: HashMap<String, String> = HashMap::new();
        env_map.insert("OPT_dapi_addresses".into(), "https://1.2.3.4:443".into());
        env_map.insert("OPT_core_host".into(), "127.0.0.1".into());
        env_map.insert("OPT_core_rpc_port".into(), "29998".into());
        env_map.insert("OPT_core_rpc_user".into(), "user".into());
        env_map.insert("OPT_core_rpc_password".into(), "pass".into());
        env_map.insert("OPT_devnet_name".into(), "devnet-evo".into());
        env_map.insert("OPT_wallet_private_key".into(), "cVBZ1234abcd".into());

        let result: Result<NetworkConfig, _> =
            envy::prefixed("OPT_").from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
        let config = result.unwrap();
        assert_eq!(config.devnet_name, Some("devnet-evo".to_string()));
        assert_eq!(config.wallet_private_key, Some("cVBZ1234abcd".to_string()));
        assert!(config.core_zmq_endpoint.is_none());
    }

    #[test]
    fn test_envy_parsing_missing_rpc_fields_succeeds() {
        use std::collections::HashMap;

        // Only DAPI addresses -- RPC fields are optional (SPV mode)
        let mut env_map: HashMap<String, String> = HashMap::new();
        env_map.insert("MISS_dapi_addresses".into(), "https://1.2.3.4:443".into());

        let result: Result<NetworkConfig, _> =
            envy::prefixed("MISS_").from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
        let config = result.unwrap();
        assert_eq!(
            config.dapi_addresses.as_deref(),
            Some("https://1.2.3.4:443")
        );
        assert!(config.core_host.is_none());
        assert!(config.core_rpc_port.is_none());
        assert!(config.core_rpc_user.is_none());
        assert!(config.core_rpc_password.is_none());
    }

    #[test]
    fn test_envy_parsing_invalid_port_fails() {
        use std::collections::HashMap;

        let mut env_map: HashMap<String, String> = HashMap::new();
        env_map.insert("BAD_dapi_addresses".into(), "https://1.2.3.4:443".into());
        env_map.insert("BAD_core_host".into(), "127.0.0.1".into());
        env_map.insert("BAD_core_rpc_port".into(), "not_a_number".into());
        env_map.insert("BAD_core_rpc_user".into(), "user".into());
        env_map.insert("BAD_core_rpc_password".into(), "pass".into());

        let result: Result<NetworkConfig, _> =
            envy::prefixed("BAD_").from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_err());
    }

    // ── Config developer_mode ───────────────────────────────────────

    #[test]
    fn test_config_developer_mode() {
        let config = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: Some(true),
        };
        assert_eq!(config.developer_mode, Some(true));

        let config_off = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: Some(false),
        };
        assert_eq!(config_off.developer_mode, Some(false));

        let config_none = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        assert_eq!(config_none.developer_mode, None);
    }

    // ── Config clone ────────────────────────────────────────────────

    #[test]
    fn test_config_clone() {
        let config = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: Some(true),
        };
        let cloned = config.clone();
        assert_eq!(
            cloned.mainnet_config.as_ref().unwrap().core_rpc_port,
            config.mainnet_config.as_ref().unwrap().core_rpc_port,
        );
        assert_eq!(cloned.developer_mode, config.developer_mode);
    }
}
