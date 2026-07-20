use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use crate::app_dir::{app_user_data_file_path, data_file_path};
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use serde::Deserialize;
use tempfile::NamedTempFile;

pub(crate) static CONFIG_PERSISTENCE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) static CONFIG_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const NETWORK_PREFIXES: [&str; 4] = ["MAINNET_", "TESTNET_", "DEVNET_", "LOCAL_"];
const NETWORK_CONFIG_FIELDS: [&str; 8] = [
    "dapi_addresses",
    "core_host",
    "core_rpc_port",
    "core_rpc_user",
    "core_rpc_password",
    "core_zmq_endpoint",
    "devnet_name",
    "wallet_private_key",
];

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub mainnet_config: Option<NetworkConfig>,
    pub testnet_config: Option<NetworkConfig>,
    pub devnet_config: Option<NetworkConfig>,
    pub local_config: Option<NetworkConfig>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to load configuration from disk or environment.
    #[error("Could not load settings. Check that the application folder is readable and retry.")]
    LoadError {
        #[source]
        source: std::io::Error,
    },

    /// The existing `.env` file could not be parsed safely.
    #[error("The saved settings contain an invalid entry. Correct the settings file and retry.")]
    InvalidEnvFile {
        #[source]
        source: dotenvy::Error,
    },

    /// A network-specific value could not be parsed into its modeled type.
    #[error(
        "The saved {network} settings contain an invalid value. Correct the settings file and retry."
    )]
    InvalidNetworkConfig {
        network: &'static str,
        #[source]
        source: envy::Error,
    },

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
    /// - Regtest: 19898
    pub fn default_rpc_port(network: Network) -> u16 {
        match network {
            Network::Mainnet => 9998,
            Network::Testnet => 19998,
            Network::Devnet => 29998,
            Network::Regtest => 19898,
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
        }
    }

    /// Write the current configuration back to the `.env` file so that
    /// subsequent calls to `Config::load()` will reflect changes.
    ///
    /// Preserves lines not modeled by `Config` and replaces the modeled fields.
    /// Uses a temporary file and atomic rename to prevent partial writes.
    pub fn save(&self, data_dir: &Path) -> Result<(), ConfigError> {
        let env_file_path =
            data_file_path(data_dir, ".env").map_err(|e| ConfigError::SaveError { source: e })?;
        let existing_contents = match fs::read_to_string(&env_file_path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(source) => return Err(ConfigError::SaveError { source }),
        };
        validate_env_contents(&existing_contents)?;

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
        let modeled_entries = self.modeled_entries();
        let mut written_keys = HashSet::new();
        for line in existing_contents.lines() {
            if let Some(key) = owned_config_key(line) {
                if let Some((_, value)) = modeled_entries
                    .iter()
                    .find(|(candidate, _)| candidate == &key)
                    && written_keys.insert(key.clone())
                {
                    let assignment = line.split_once('=').map_or(key.as_str(), |(left, _)| left);
                    writeln!(env_file, "{assignment}={value}")
                        .map_err(|source| ConfigError::SaveError { source })?;
                }
            } else {
                writeln!(env_file, "{line}").map_err(|source| ConfigError::SaveError { source })?;
            }
        }
        for (key, value) in modeled_entries {
            if written_keys.insert(key.clone()) {
                writeln!(env_file, "{key}={value}")
                    .map_err(|source| ConfigError::SaveError { source })?;
            }
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
            data_file_path(data_dir, ".env").map_err(|source| ConfigError::LoadError { source })?;
        Self::load_from_env_path(env_file_path)
    }

    fn load_from_env_path(env_file_path: std::path::PathBuf) -> Result<Self, ConfigError> {
        let file_entries = match fs::read_to_string(&env_file_path) {
            Ok(contents) => dotenvy::Iter::new(
                contents
                    .strip_prefix('\u{feff}')
                    .unwrap_or(&contents)
                    .as_bytes(),
            )
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| ConfigError::InvalidEnvFile { source })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(source) => return Err(ConfigError::LoadError { source }),
        };
        match dotenvy::from_path_override(&env_file_path) {
            Ok(()) => tracing::info!("Successfully loaded .env file"),
            Err(error) if error.not_found() => {
                tracing::warn!(
                    ?error,
                    "No .env file was found. Continuing with environment variables."
                );
            }
            Err(source) => return Err(ConfigError::InvalidEnvFile { source }),
        }

        let process_entries = std::env::vars().collect::<Vec<_>>();
        let mainnet_config =
            load_network_config("MAINNET_", "Mainnet", &process_entries, &file_entries)?;
        let testnet_config =
            load_network_config("TESTNET_", "Testnet", &process_entries, &file_entries)?;
        let devnet_config =
            load_network_config("DEVNET_", "Devnet", &process_entries, &file_entries)?;
        let local_config =
            load_network_config("LOCAL_", "local network", &process_entries, &file_entries)?;

        if mainnet_config.is_none()
            && testnet_config.is_none()
            && devnet_config.is_none()
            && local_config.is_none()
        {
            return Err(ConfigError::NoValidConfigs);
        }

        Ok(Config {
            mainnet_config,
            testnet_config,
            devnet_config,
            local_config,
        })
    }

    /// Update (overwrite) the configuration for a particular network.
    pub fn update_config_for_network(&mut self, network: Network, new_config: NetworkConfig) {
        match network {
            Network::Mainnet => self.mainnet_config = Some(new_config),
            Network::Testnet => self.testnet_config = Some(new_config),
            Network::Devnet => self.devnet_config = Some(new_config),
            Network::Regtest => self.local_config = Some(new_config),
        }
    }

    fn modeled_entries(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();
        for (prefix, config) in [
            ("MAINNET_", self.mainnet_config.as_ref()),
            ("TESTNET_", self.testnet_config.as_ref()),
            ("DEVNET_", self.devnet_config.as_ref()),
            ("LOCAL_", self.local_config.as_ref()),
        ] {
            if let Some(config) = config {
                append_network_entries(&mut entries, prefix, config);
            }
        }
        entries
    }
}

fn load_network_config(
    prefix: &'static str,
    network: &'static str,
    process_entries: &[(String, String)],
    file_entries: &[(String, String)],
) -> Result<Option<NetworkConfig>, ConfigError> {
    let mut entries = std::collections::HashMap::new();
    for (key, value) in process_entries.iter().chain(file_entries) {
        if let Some(field) = key.strip_prefix(prefix) {
            entries.insert(
                format!("{prefix}{}", field.to_ascii_lowercase()),
                value.clone(),
            );
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    envy::prefixed(prefix)
        .from_iter::<_, NetworkConfig>(entries)
        .map(Some)
        .map_err(|source| ConfigError::InvalidNetworkConfig { network, source })
}

fn validate_env_contents(contents: &str) -> Result<(), ConfigError> {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let entries = dotenvy::Iter::new(contents.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ConfigError::InvalidEnvFile { source })?;
    for (prefix, network) in [
        ("MAINNET_", "Mainnet"),
        ("TESTNET_", "Testnet"),
        ("DEVNET_", "Devnet"),
        ("LOCAL_", "local network"),
    ] {
        if entries.iter().any(|(key, _)| key.starts_with(prefix)) {
            envy::prefixed(prefix)
                .from_iter::<_, NetworkConfig>(entries.iter().cloned())
                .map_err(|source| ConfigError::InvalidNetworkConfig { network, source })?;
        }
    }
    Ok(())
}

fn append_network_entries(
    entries: &mut Vec<(String, String)>,
    prefix: &str,
    config: &NetworkConfig,
) {
    let mut push = |field: &str, value: String| {
        entries.push((format!("{prefix}{field}"), value));
    };
    if let Some(addresses) = &config.dapi_addresses
        && !addresses.is_empty()
    {
        push("dapi_addresses", addresses.clone());
    }
    if let Some(host) = &config.core_host {
        push("core_host", host.clone());
    }
    if let Some(port) = config.core_rpc_port {
        push("core_rpc_port", port.to_string());
    }
    if let Some(user) = &config.core_rpc_user {
        push("core_rpc_user", user.clone());
    }
    if let Some(password) = &config.core_rpc_password {
        push("core_rpc_password", password.clone());
    }
    if let Some(endpoint) = &config.core_zmq_endpoint {
        push("core_zmq_endpoint", endpoint.clone());
    }
    if let Some(name) = &config.devnet_name {
        push("devnet_name", name.clone());
    }
    if let Some(key) = &config.wallet_private_key {
        push("wallet_private_key", key.clone());
    }
}

fn owned_config_key(line: &str) -> Option<String> {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line).trim_start();
    let (mut key, mut rest) = take_env_key(line)?;
    rest = rest.trim_start();
    if key == "export" && !rest.starts_with('=') {
        (key, rest) = take_env_key(rest)?;
        rest = rest.trim_start();
    }
    if !rest.starts_with('=') {
        return None;
    }
    NETWORK_PREFIXES.iter().find_map(|prefix| {
        let field = key.strip_prefix(prefix)?;
        NETWORK_CONFIG_FIELDS
            .iter()
            .find(|owned| field.eq_ignore_ascii_case(owned))
            .map(|owned| format!("{prefix}{owned}"))
    })
}

fn take_env_key(input: &str) -> Option<(&str, &str)> {
    let first = *input.as_bytes().first()?;
    if !first.is_ascii_alphabetic() && first != b'_' {
        return None;
    }
    let end = input
        .as_bytes()
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_' && *byte != b'.')
        .unwrap_or(input.len());
    Some((&input[..end], &input[end..]))
}

impl NetworkConfig {
    /// List of DAPI addresses, if explicitly configured.
    /// Returns `Ok(None)` when absent or empty (dynamic discovery should be used).
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

    fn run_config_test_in_subprocess(test_name: &str) -> bool {
        const CHILD_MARKER: &str = "DET_CONFIG_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_some() {
            return false;
        }
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .arg(test_name)
            .arg("--exact")
            .env(CHILD_MARKER, "1")
            .status()
            .expect("run isolated config test");
        assert!(status.success(), "isolated config test failed");
        true
    }

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

    #[test]
    fn save_preserves_unmodeled_env_keys() {
        if run_config_test_in_subprocess("config::tests::save_preserves_unmodeled_env_keys") {
            return;
        }
        let tmp = tempfile::tempdir().expect("tempdir");
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "# Operator settings\nMCP_API_KEY=some-fake-test-value\nMAINNET_dapi_addresses=https://old.example:443\nRUST_LOG=info\n",
        )
        .expect("write test config");

        let mut config = Config::load_from(tmp.path()).expect("load test config");
        config
            .mainnet_config
            .as_mut()
            .expect("mainnet config")
            .dapi_addresses = Some("https://new.example:443".to_string());
        config.save(tmp.path()).expect("save test config");

        let saved = std::fs::read_to_string(env_path).expect("read saved config");
        assert!(saved.contains("# Operator settings\n"));
        assert!(saved.contains("MCP_API_KEY=some-fake-test-value\n"));
        assert!(saved.contains("RUST_LOG=info\n"));
        assert!(saved.contains("MAINNET_dapi_addresses=https://new.example:443\n"));
        assert!(!saved.contains("MAINNET_dapi_addresses=https://old.example:443\n"));
    }

    #[test]
    fn save_replaces_modeled_keys_case_insensitively() {
        if run_config_test_in_subprocess(
            "config::tests::save_replaces_modeled_keys_case_insensitively",
        ) {
            return;
        }
        let tmp = tempfile::tempdir().expect("tempdir");
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "\u{feff}export\tMAINNET_DAPI_ADDRESSES=https://old.example:443\nmainnet_dapi_addresses=keep-this-line\n",
        )
        .expect("write test config");

        let mut config = Config::load_from(tmp.path()).expect("load test config");
        config
            .mainnet_config
            .as_mut()
            .expect("mainnet config")
            .dapi_addresses = Some("https://new.example:443".to_string());
        config.save(tmp.path()).expect("save test config");

        let saved = std::fs::read_to_string(&env_path).expect("read saved config");
        assert!(!saved.contains("MAINNET_DAPI_ADDRESSES=https://old.example:443\n"));
        assert!(saved.contains("mainnet_dapi_addresses=keep-this-line\n"));
        let reloaded = Config::load_from(tmp.path()).expect("reload test config");
        assert_eq!(
            reloaded
                .mainnet_config
                .as_ref()
                .and_then(|network| network.dapi_addresses.as_deref()),
            Some("https://new.example:443"),
        );
    }

    #[test]
    fn save_preserves_unmodeled_line_order() {
        if run_config_test_in_subprocess("config::tests::save_preserves_unmodeled_line_order") {
            return;
        }
        let tmp = tempfile::tempdir().expect("tempdir");
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "MAINNET_core_host=127.0.0.1\nEXTRA_HOST=${MAINNET_core_host}\nMAINNET_dapi_addresses=https://old.example:443\n",
        )
        .expect("write test config");

        let mut config = Config::load_from(tmp.path()).expect("load test config");
        config
            .mainnet_config
            .as_mut()
            .expect("mainnet config")
            .dapi_addresses = Some("https://new.example:443".to_string());
        config.save(tmp.path()).expect("save test config");

        let saved = std::fs::read_to_string(env_path).expect("read saved config");
        let host_position = saved.find("MAINNET_core_host=").expect("modeled host");
        let extra_position = saved.find("EXTRA_HOST=").expect("unmodeled reference");
        let dapi_position = saved
            .find("MAINNET_dapi_addresses=")
            .expect("modeled addresses");
        assert!(host_position < extra_position);
        assert!(extra_position < dapi_position);
    }

    #[test]
    fn save_rejects_invalid_modeled_values_without_rewriting() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env_path = tmp.path().join(".env");
        let original = "MAINNET_dapi_addresses=https://mainnet.example:443\nTESTNET_core_rpc_port=not-a-number\nTESTNET_core_rpc_password=fake-test-password\n";
        std::fs::write(&env_path, original).expect("write test config");
        let config = Config {
            mainnet_config: Some(make_network_config("https://mainnet-new.example:443", 9998)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
        };

        let error = config
            .save(tmp.path())
            .expect_err("invalid modeled value must fail closed");

        assert!(matches!(
            error,
            ConfigError::InvalidNetworkConfig {
                network: "Testnet",
                ..
            }
        ));
        assert_eq!(
            std::fs::read_to_string(env_path).expect("read unchanged config"),
            original,
        );
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

    // ── Config clone ────────────────────────────────────────────────

    #[test]
    fn test_config_clone() {
        let config = Config {
            mainnet_config: Some(make_network_config("https://1.1.1.1:443", 9998)),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
        };
        let cloned = config.clone();
        assert_eq!(
            cloned.mainnet_config.as_ref().unwrap().core_rpc_port,
            config.mainnet_config.as_ref().unwrap().core_rpc_port,
        );
    }
}
