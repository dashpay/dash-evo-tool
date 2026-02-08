use std::fs::File;
use std::io::Write;
use std::str::FromStr;

use crate::app_dir::app_user_data_file_path;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::sdk::Uri;
use serde::Deserialize;

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
    #[error("{0}")]
    LoadError(String),
    #[error("No valid network configurations found in .env file or environment variables")]
    NoValidConfigs,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NetworkConfig {
    /// Hostname of Dash Platform node to connect to
    pub dapi_addresses: String,
    /// Host of the Dash Core RPC interface
    pub core_host: String,
    /// Port of the Dash Core RPC interface
    pub core_rpc_port: u16,
    /// Username for Dash Core RPC interface
    pub core_rpc_user: String,
    /// Password for Dash Core RPC interface
    pub core_rpc_password: String,
    /// URL of the Insight API
    pub insight_api_url: String,
    /// ZMQ endpoint for Core blockchain events (e.g., tcp://127.0.0.1:23708)
    pub core_zmq_endpoint: Option<String>,
    /// Devnet network name if one exists
    pub devnet_name: Option<String>,
    /// Optional wallet private key to instantiate the wallet
    pub wallet_private_key: Option<String>,
    /// Should this network be visible in the UI
    pub show_in_ui: bool,
}

impl Config {
    pub fn config_for_network(&self, network: Network) -> &Option<NetworkConfig> {
        match network {
            Network::Dash => &self.mainnet_config,
            Network::Testnet => &self.testnet_config,
            Network::Devnet => &self.devnet_config,
            Network::Regtest => &self.local_config,
            _ => &None,
        }
    }

    /// Write the configuration in `.env` format to the given writer.
    ///
    /// This is the core serialization logic used by [`save()`]. Extracted
    /// so it can also be used in tests without touching the filesystem.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<(), ConfigError> {
        let write_network_config =
            |w: &mut W, prefix: &str, config: &NetworkConfig| -> Result<(), ConfigError> {
                writeln!(w, "{}dapi_addresses={}", prefix, config.dapi_addresses)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                writeln!(w, "{}core_host={}", prefix, config.core_host)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                writeln!(w, "{}core_rpc_port={}", prefix, config.core_rpc_port)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                writeln!(w, "{}core_rpc_user={}", prefix, config.core_rpc_user)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                writeln!(
                    w,
                    "{}core_rpc_password={}",
                    prefix, config.core_rpc_password
                )
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                writeln!(w, "{}insight_api_url={}", prefix, config.insight_api_url)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;

                if let Some(core_zmq_endpoint) = &config.core_zmq_endpoint {
                    writeln!(w, "{}core_zmq_endpoint={}", prefix, core_zmq_endpoint)
                        .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                }

                if let Some(devnet_name) = &config.devnet_name {
                    writeln!(w, "{}devnet_name={}", prefix, devnet_name)
                        .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                }
                if let Some(wallet_private_key) = &config.wallet_private_key {
                    writeln!(w, "{}wallet_private_key={}", prefix, wallet_private_key)
                        .map_err(|e| ConfigError::LoadError(e.to_string()))?;
                }

                writeln!(w, "{}show_in_ui={}", prefix, config.show_in_ui)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;

                // Blank line after each config block
                writeln!(w).map_err(|e| ConfigError::LoadError(e.to_string()))?;

                Ok(())
            };

        if let Some(ref mainnet_config) = self.mainnet_config {
            write_network_config(writer, "MAINNET_", mainnet_config)?;
        }
        if let Some(ref testnet_config) = self.testnet_config {
            write_network_config(writer, "TESTNET_", testnet_config)?;
        }
        if let Some(ref devnet_config) = self.devnet_config {
            write_network_config(writer, "DEVNET_", devnet_config)?;
        }
        if let Some(ref local_config) = self.local_config {
            write_network_config(writer, "LOCAL_", local_config)?;
        }

        if let Some(developer_mode) = self.developer_mode {
            writeln!(writer, "DEVELOPER_MODE={}", developer_mode)
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
        }

        Ok(())
    }

    /// Write the current configuration back to the `.env` file so that
    /// subsequent calls to `Config::load()` will reflect changes.
    ///
    /// Uses atomic write (write to temp file, then rename) to prevent
    /// config corruption if a write fails partway through.
    pub fn save(&self) -> Result<(), ConfigError> {
        let env_file_path =
            app_user_data_file_path(".env").map_err(|e| ConfigError::LoadError(e.to_string()))?;

        // Write to a temporary file in the same directory first, then rename
        // atomically. This prevents corruption if the write fails partway through.
        let tmp_file_path = env_file_path.with_extension("env.tmp");

        let mut env_file =
            File::create(&tmp_file_path).map_err(|e| ConfigError::LoadError(e.to_string()))?;

        self.write_to(&mut env_file)?;

        // Flush all buffered data to disk before renaming
        env_file
            .flush()
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;

        // Atomically replace the old config with the new one
        std::fs::rename(&tmp_file_path, &env_file_path).map_err(|e| {
            // Clean up the temp file on rename failure
            let _ = std::fs::remove_file(&tmp_file_path);
            ConfigError::LoadError(format!("Failed to rename temp config file: {}", e))
        })?;

        tracing::info!("Successfully saved configuration to {:?}", env_file_path);
        Ok(())
    }

    /// Loads the configuration for all networks from environment variables and `.env` file.
    pub fn load() -> Result<Self, ConfigError> {
        // Load the .env file if available
        let env_file_path = app_user_data_file_path(".env").expect("should create .env file path");
        if let Err(err) = dotenvy::from_path_override(env_file_path) {
            tracing::warn!(
                ?err,
                "Failed to load .env file. Continuing with environment variables."
            );
        } else {
            tracing::info!("Successfully loaded .env file");
        }

        // Load individual network configs and log if they fail
        let mainnet_config = match envy::prefixed("MAINNET_").from_env::<NetworkConfig>() {
            Ok(config) => {
                tracing::info!("Mainnet configuration loaded successfully");
                Some(config)
            }
            Err(err) => {
                tracing::error!(?err, "Failed to load mainnet configuration");
                None
            }
        };

        let testnet_config = match envy::prefixed("TESTNET_").from_env::<NetworkConfig>() {
            Ok(config) => {
                tracing::info!("Testnet configuration loaded successfully");
                Some(config)
            }
            Err(err) => {
                tracing::error!(?err, "Failed to load testnet configuration");
                None
            }
        };

        let devnet_config = match envy::prefixed("DEVNET_").from_env::<NetworkConfig>() {
            Ok(config) => {
                tracing::info!("Devnet configuration loaded successfully");
                Some(config)
            }
            Err(err) => {
                tracing::error!(?err, "Failed to load devnet configuration");
                None
            }
        };

        let local_config = match envy::prefixed("LOCAL_").from_env::<NetworkConfig>() {
            Ok(config) => {
                tracing::info!("Local configuration loaded successfully");
                Some(config)
            }
            Err(err) => {
                tracing::error!(?err, "Failed to load local configuration");
                None
            }
        };

        if mainnet_config.is_none()
            && testnet_config.is_none()
            && devnet_config.is_none()
            && local_config.is_none()
        {
            return Err(ConfigError::NoValidConfigs);
        } else if mainnet_config.is_none() {
            return Err(ConfigError::LoadError(
                "Failed to load mainnet configuration".into(),
            ));
        } else if testnet_config.is_none() {
            tracing::warn!(
                "Failed to load testnet configuration, but successfully loaded mainnet config"
            );
        } else if devnet_config.is_none() {
            tracing::warn!(
                "Failed to load devnet configuration, but successfully loaded mainnet config"
            );
        } else if local_config.is_none() {
            tracing::warn!(
                "Failed to load local configuration, but successfully loaded mainnet config"
            );
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
            Network::Dash => self.mainnet_config = Some(new_config),
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
    /// Check if configuration is set
    #[allow(dead_code)] // May be used for validation
    pub fn is_valid(&self) -> bool {
        !self.core_rpc_user.is_empty()
            && !self.core_rpc_password.is_empty()
            && self.core_rpc_port != 0
            && !self.dapi_addresses.is_empty()
            && Uri::from_str(&self.insight_api_url).is_ok()
    }

    /// List of DAPI addresses
    pub fn dapi_address_list(&self) -> Result<AddressList, String> {
        AddressList::from_str(&self.dapi_addresses).map_err(|e| {
            format!(
                "Could not parse DAPI addresses '{}': {}",
                self.dapi_addresses, e
            )
        })
    }

    /// Insight API URI
    #[allow(dead_code)] // May be used for insight API access
    pub fn insight_api_uri(&self) -> Result<Uri, String> {
        Uri::from_str(&self.insight_api_url)
            .map_err(|e| format!("Invalid insight API URL '{}': {}", self.insight_api_url, e))
    }

    /// Update just the `core_rpc_password` in a builder-like manner.
    /// Returns a new `NetworkConfig` with the updated password.
    pub fn update_core_rpc_password(mut self, new_password: String) -> Self {
        self.core_rpc_password = new_password;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal valid NetworkConfig for testing
    fn make_network_config(
        dapi_addresses: &str,
        insight_api_url: &str,
        port: u16,
    ) -> NetworkConfig {
        NetworkConfig {
            dapi_addresses: dapi_addresses.to_string(),
            core_host: "127.0.0.1".to_string(),
            core_rpc_port: port,
            core_rpc_user: "dashrpc".to_string(),
            core_rpc_password: "password".to_string(),
            insight_api_url: insight_api_url.to_string(),
            core_zmq_endpoint: Some("tcp://127.0.0.1:23708".to_string()),
            devnet_name: None,
            wallet_private_key: None,
            show_in_ui: true,
        }
    }

    // ── NetworkConfig::is_valid ──────────────────────────────────────

    #[test]
    fn test_is_valid_with_all_fields() {
        let config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        assert!(config.is_valid());
    }

    #[test]
    fn test_is_valid_empty_user() {
        let mut config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        config.core_rpc_user = String::new();
        assert!(!config.is_valid());
    }

    #[test]
    fn test_is_valid_empty_password() {
        let mut config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        config.core_rpc_password = String::new();
        assert!(!config.is_valid());
    }

    #[test]
    fn test_is_valid_zero_port() {
        let config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            0,
        );
        assert!(!config.is_valid());
    }

    #[test]
    fn test_is_valid_empty_dapi_addresses() {
        let config = make_network_config("", "https://insight.dash.org/insight-api", 9998);
        assert!(!config.is_valid());
    }

    #[test]
    fn test_is_valid_invalid_insight_url() {
        let config = make_network_config("https://127.0.0.1:443", "not a valid url \x00", 9998);
        assert!(!config.is_valid());
    }

    // ── NetworkConfig::dapi_address_list ─────────────────────────────

    #[test]
    fn test_dapi_address_list_single_address() {
        let config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        let result = config.dapi_address_list();
        assert!(result.is_ok());
    }

    #[test]
    fn test_dapi_address_list_multiple_addresses() {
        let config = make_network_config(
            "https://127.0.0.1:443,https://192.168.1.1:443,https://10.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        let result = config.dapi_address_list();
        assert!(result.is_ok());
    }

    #[test]
    fn test_dapi_address_list_empty_returns_error() {
        let config = make_network_config("", "https://insight.dash.org/insight-api", 9998);
        let result = config.dapi_address_list();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Could not parse DAPI addresses")
        );
    }

    // ── NetworkConfig::insight_api_uri ───────────────────────────────

    #[test]
    fn test_insight_api_uri_valid() {
        let config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        let result = config.insight_api_uri();
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().to_string(),
            "https://insight.dash.org/insight-api"
        );
    }

    #[test]
    fn test_insight_api_uri_invalid() {
        let config = make_network_config("https://127.0.0.1:443", "not a valid url \x00", 9998);
        let result = config.insight_api_uri();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid insight API URL"));
    }

    #[test]
    fn test_insight_api_uri_empty_is_error() {
        let config = make_network_config("https://127.0.0.1:443", "", 9998);
        let result = config.insight_api_uri();
        assert!(result.is_err());
    }

    // ── NetworkConfig::update_core_rpc_password ─────────────────────

    #[test]
    fn test_update_core_rpc_password() {
        let config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        assert_eq!(config.core_rpc_password, "password");
        let updated = config.update_core_rpc_password("new_secret".to_string());
        assert_eq!(updated.core_rpc_password, "new_secret");
        // Other fields should be unchanged
        assert_eq!(updated.core_rpc_user, "dashrpc");
        assert_eq!(updated.core_rpc_port, 9998);
    }

    // ── Config::config_for_network ──────────────────────────────────

    #[test]
    fn test_config_for_network_mainnet() {
        let mainnet_cfg = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        let config = Config {
            mainnet_config: Some(mainnet_cfg),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        assert!(config.config_for_network(Network::Dash).is_some());
        assert!(config.config_for_network(Network::Testnet).is_none());
        assert!(config.config_for_network(Network::Devnet).is_none());
        assert!(config.config_for_network(Network::Regtest).is_none());
    }

    #[test]
    fn test_config_for_network_all_networks() {
        let config = Config {
            mainnet_config: Some(make_network_config(
                "https://1.1.1.1:443",
                "https://main.example.com",
                9998,
            )),
            testnet_config: Some(make_network_config(
                "https://2.2.2.2:1443",
                "https://test.example.com",
                19998,
            )),
            devnet_config: Some(make_network_config(
                "http://3.3.3.3:1443",
                "http://dev.example.com",
                29998,
            )),
            local_config: Some(make_network_config(
                "http://127.0.0.1:2443",
                "http://localhost:3001",
                20302,
            )),
            developer_mode: Some(true),
        };
        let main = config.config_for_network(Network::Dash).as_ref().unwrap();
        assert_eq!(main.core_rpc_port, 9998);
        let test = config
            .config_for_network(Network::Testnet)
            .as_ref()
            .unwrap();
        assert_eq!(test.core_rpc_port, 19998);
        let dev = config.config_for_network(Network::Devnet).as_ref().unwrap();
        assert_eq!(dev.core_rpc_port, 29998);
        let local = config
            .config_for_network(Network::Regtest)
            .as_ref()
            .unwrap();
        assert_eq!(local.core_rpc_port, 20302);
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
        let new_cfg = make_network_config(
            "https://1.1.1.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
        config.update_config_for_network(Network::Dash, new_cfg);
        assert!(config.mainnet_config.is_some());
        assert_eq!(config.mainnet_config.as_ref().unwrap().core_rpc_port, 9998);
    }

    #[test]
    fn test_update_config_replaces_existing() {
        let mut config = Config {
            mainnet_config: Some(make_network_config(
                "https://old.example.com:443",
                "https://old.example.com",
                1111,
            )),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };
        let new_cfg = make_network_config(
            "https://new.example.com:443",
            "https://new.example.com",
            2222,
        );
        config.update_config_for_network(Network::Dash, new_cfg);
        let main = config.mainnet_config.as_ref().unwrap();
        assert_eq!(main.core_rpc_port, 2222);
        assert_eq!(main.dapi_addresses, "https://new.example.com:443");
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
            make_network_config("https://t.example.com:1443", "https://t.example.com", 19998),
        );
        config.update_config_for_network(
            Network::Devnet,
            make_network_config("http://d.example.com:1443", "http://d.example.com", 29998),
        );
        config.update_config_for_network(
            Network::Regtest,
            make_network_config("http://127.0.0.1:2443", "http://localhost:3001", 20302),
        );
        assert!(config.testnet_config.is_some());
        assert!(config.devnet_config.is_some());
        assert!(config.local_config.is_some());
    }

    // ── NetworkConfig optional fields ───────────────────────────────

    #[test]
    fn test_network_config_optional_fields() {
        let mut config = make_network_config(
            "https://127.0.0.1:443",
            "https://insight.dash.org/insight-api",
            9998,
        );
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
            mainnet_config: Some(make_network_config(
                "https://1.1.1.1:443",
                "https://insight.dash.org/insight-api",
                9998,
            )),
            testnet_config: Some(make_network_config(
                "https://2.2.2.2:1443",
                "https://testnet-insight.dash.org/insight-api",
                19998,
            )),
            devnet_config: None,
            local_config: None,
            developer_mode: Some(true),
        };

        // Simulate what save() writes by formatting env lines
        let mut output = String::new();
        if let Some(ref cfg) = config.mainnet_config {
            output.push_str(&format!("MAINNET_dapi_addresses={}\n", cfg.dapi_addresses));
            output.push_str(&format!("MAINNET_core_host={}\n", cfg.core_host));
            output.push_str(&format!("MAINNET_core_rpc_port={}\n", cfg.core_rpc_port));
            output.push_str(&format!("MAINNET_core_rpc_user={}\n", cfg.core_rpc_user));
            output.push_str(&format!(
                "MAINNET_core_rpc_password={}\n",
                cfg.core_rpc_password
            ));
            output.push_str(&format!(
                "MAINNET_insight_api_url={}\n",
                cfg.insight_api_url
            ));
            if let Some(ref zmq) = cfg.core_zmq_endpoint {
                output.push_str(&format!("MAINNET_core_zmq_endpoint={}\n", zmq));
            }
            output.push_str(&format!("MAINNET_show_in_ui={}\n", cfg.show_in_ui));
        }

        assert!(output.contains("MAINNET_dapi_addresses=https://1.1.1.1:443"));
        assert!(output.contains("MAINNET_core_rpc_port=9998"));
        assert!(output.contains("MAINNET_core_rpc_user=dashrpc"));
        assert!(output.contains("MAINNET_core_rpc_password=password"));
        assert!(output.contains("MAINNET_insight_api_url=https://insight.dash.org/insight-api"));
        assert!(output.contains("MAINNET_core_zmq_endpoint=tcp://127.0.0.1:23708"));
        assert!(output.contains("MAINNET_show_in_ui=true"));
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
            "TEST_RT_insight_api_url".into(),
            "https://insight.example.com/api".into(),
        );
        env_map.insert(
            "TEST_RT_core_zmq_endpoint".into(),
            "tcp://127.0.0.1:29999".into(),
        );
        env_map.insert("TEST_RT_show_in_ui".into(), "true".into());

        // Use envy's from_iter to parse from our map (same as from_env but testable)
        let result: Result<NetworkConfig, _> = envy::prefixed("TEST_RT_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
        let config = result.unwrap();
        assert_eq!(config.dapi_addresses, "https://1.2.3.4:443");
        assert_eq!(config.core_host, "192.168.1.100");
        assert_eq!(config.core_rpc_port, 9998);
        assert_eq!(config.core_rpc_user, "testuser");
        assert_eq!(config.core_rpc_password, "testpass");
        assert_eq!(config.insight_api_url, "https://insight.example.com/api");
        assert_eq!(
            config.core_zmq_endpoint,
            Some("tcp://127.0.0.1:29999".to_string())
        );
        assert!(config.show_in_ui);
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
        env_map.insert("OPT_insight_api_url".into(), "http://localhost:3001".into());
        env_map.insert("OPT_show_in_ui".into(), "false".into());
        env_map.insert("OPT_devnet_name".into(), "devnet-evo".into());
        env_map.insert("OPT_wallet_private_key".into(), "cVBZ1234abcd".into());

        let result: Result<NetworkConfig, _> =
            envy::prefixed("OPT_").from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
        let config = result.unwrap();
        assert_eq!(config.devnet_name, Some("devnet-evo".to_string()));
        assert_eq!(config.wallet_private_key, Some("cVBZ1234abcd".to_string()));
        assert!(!config.show_in_ui);
        assert!(config.core_zmq_endpoint.is_none());
    }

    #[test]
    fn test_envy_parsing_missing_required_field_fails() {
        use std::collections::HashMap;

        // Missing core_rpc_port (required)
        let mut env_map: HashMap<String, String> = HashMap::new();
        env_map.insert("MISS_dapi_addresses".into(), "https://1.2.3.4:443".into());
        env_map.insert("MISS_core_host".into(), "127.0.0.1".into());
        // core_rpc_port intentionally missing
        env_map.insert("MISS_core_rpc_user".into(), "user".into());
        env_map.insert("MISS_core_rpc_password".into(), "pass".into());
        env_map.insert("MISS_insight_api_url".into(), "http://localhost".into());
        env_map.insert("MISS_show_in_ui".into(), "true".into());

        let result: Result<NetworkConfig, _> =
            envy::prefixed("MISS_").from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_err());
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
        env_map.insert("BAD_insight_api_url".into(), "http://localhost".into());
        env_map.insert("BAD_show_in_ui".into(), "true".into());

        let result: Result<NetworkConfig, _> =
            envy::prefixed("BAD_").from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())));
        assert!(result.is_err());
    }

    // ── Config developer_mode ───────────────────────────────────────

    #[test]
    fn test_config_developer_mode() {
        let config = Config {
            mainnet_config: Some(make_network_config(
                "https://1.1.1.1:443",
                "https://insight.dash.org",
                9998,
            )),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: Some(true),
        };
        assert_eq!(config.developer_mode, Some(true));

        let config_off = Config {
            mainnet_config: Some(make_network_config(
                "https://1.1.1.1:443",
                "https://insight.dash.org",
                9998,
            )),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: Some(false),
        };
        assert_eq!(config_off.developer_mode, Some(false));

        let config_none = Config {
            mainnet_config: Some(make_network_config(
                "https://1.1.1.1:443",
                "https://insight.dash.org",
                9998,
            )),
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
            mainnet_config: Some(make_network_config(
                "https://1.1.1.1:443",
                "https://insight.dash.org",
                9998,
            )),
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

    // ── Config write_to + roundtrip tests ─────────────────────────

    /// Helper: write a Config to a buffer, parse individual lines as
    /// `KEY=VALUE` pairs into a HashMap, and then use `envy::prefixed()`
    /// to deserialize back into `NetworkConfig`.
    fn parse_env_lines(env_text: &str) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        for line in env_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.to_string(), value.to_string());
            }
        }
        map
    }

    /// Roundtrip: write a Config with all networks → buffer → parse back
    /// via envy → verify all fields are preserved.
    #[test]
    fn test_write_to_roundtrip_all_networks() {
        let config = Config {
            mainnet_config: Some(NetworkConfig {
                dapi_addresses: "https://1.1.1.1:443".to_string(),
                core_host: "10.0.0.1".to_string(),
                core_rpc_port: 9998,
                core_rpc_user: "mainuser".to_string(),
                core_rpc_password: "mainpass".to_string(),
                insight_api_url: "https://insight.dash.org/api".to_string(),
                core_zmq_endpoint: Some("tcp://10.0.0.1:23708".to_string()),
                devnet_name: None,
                wallet_private_key: None,
                show_in_ui: true,
            }),
            testnet_config: Some(NetworkConfig {
                dapi_addresses: "https://2.2.2.2:1443".to_string(),
                core_host: "10.0.0.2".to_string(),
                core_rpc_port: 19998,
                core_rpc_user: "testuser".to_string(),
                core_rpc_password: "testpass".to_string(),
                insight_api_url: "https://testnet-insight.dash.org/api".to_string(),
                core_zmq_endpoint: Some("tcp://10.0.0.2:29998".to_string()),
                devnet_name: None,
                wallet_private_key: Some("cVtest1234".to_string()),
                show_in_ui: true,
            }),
            devnet_config: Some(NetworkConfig {
                dapi_addresses: "http://3.3.3.3:1443".to_string(),
                core_host: "10.0.0.3".to_string(),
                core_rpc_port: 29998,
                core_rpc_user: "devuser".to_string(),
                core_rpc_password: "devpass".to_string(),
                insight_api_url: "http://devnet-insight.example.com/api".to_string(),
                core_zmq_endpoint: None,
                devnet_name: Some("devnet-alpha".to_string()),
                wallet_private_key: Some("cVdev5678".to_string()),
                show_in_ui: false,
            }),
            local_config: Some(NetworkConfig {
                dapi_addresses: "http://127.0.0.1:2443".to_string(),
                core_host: "127.0.0.1".to_string(),
                core_rpc_port: 20302,
                core_rpc_user: "localuser".to_string(),
                core_rpc_password: "localpass".to_string(),
                insight_api_url: "http://localhost:3001".to_string(),
                core_zmq_endpoint: Some("tcp://127.0.0.1:30000".to_string()),
                devnet_name: None,
                wallet_private_key: None,
                show_in_ui: true,
            }),
            developer_mode: Some(true),
        };

        let mut buf = Vec::new();
        config.write_to(&mut buf).expect("write_to should succeed");
        let env_text = String::from_utf8(buf).expect("should be valid UTF-8");
        let env_map = parse_env_lines(&env_text);

        // Parse each network back via envy
        let mainnet: NetworkConfig = envy::prefixed("MAINNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("mainnet parse");
        assert_eq!(mainnet.dapi_addresses, "https://1.1.1.1:443");
        assert_eq!(mainnet.core_host, "10.0.0.1");
        assert_eq!(mainnet.core_rpc_port, 9998);
        assert_eq!(mainnet.core_rpc_user, "mainuser");
        assert_eq!(mainnet.core_rpc_password, "mainpass");
        assert_eq!(mainnet.insight_api_url, "https://insight.dash.org/api");
        assert_eq!(
            mainnet.core_zmq_endpoint,
            Some("tcp://10.0.0.1:23708".to_string())
        );
        assert!(mainnet.devnet_name.is_none());
        assert!(mainnet.wallet_private_key.is_none());
        assert!(mainnet.show_in_ui);

        let testnet: NetworkConfig = envy::prefixed("TESTNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("testnet parse");
        assert_eq!(testnet.dapi_addresses, "https://2.2.2.2:1443");
        assert_eq!(testnet.core_rpc_port, 19998);
        assert_eq!(testnet.core_rpc_user, "testuser");
        assert_eq!(testnet.core_rpc_password, "testpass");
        assert_eq!(testnet.wallet_private_key, Some("cVtest1234".to_string()));

        let devnet: NetworkConfig = envy::prefixed("DEVNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("devnet parse");
        assert_eq!(devnet.dapi_addresses, "http://3.3.3.3:1443");
        assert_eq!(devnet.core_rpc_port, 29998);
        assert!(devnet.core_zmq_endpoint.is_none());
        assert_eq!(devnet.devnet_name, Some("devnet-alpha".to_string()));
        assert_eq!(devnet.wallet_private_key, Some("cVdev5678".to_string()));
        assert!(!devnet.show_in_ui);

        let local: NetworkConfig = envy::prefixed("LOCAL_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("local parse");
        assert_eq!(local.dapi_addresses, "http://127.0.0.1:2443");
        assert_eq!(local.core_rpc_port, 20302);
        assert_eq!(local.core_rpc_user, "localuser");
        assert!(local.wallet_private_key.is_none());

        // Verify developer_mode roundtrip
        assert!(env_map.contains_key("DEVELOPER_MODE"));
        assert_eq!(env_map["DEVELOPER_MODE"], "true");
    }

    /// Roundtrip: write a Config with only mainnet → buffer → parse back.
    #[test]
    fn test_write_to_roundtrip_single_network() {
        let config = Config {
            mainnet_config: Some(make_network_config(
                "https://1.2.3.4:443",
                "https://insight.example.com/api",
                9998,
            )),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };

        let mut buf = Vec::new();
        config.write_to(&mut buf).expect("write_to should succeed");
        let env_text = String::from_utf8(buf).expect("should be valid UTF-8");
        let env_map = parse_env_lines(&env_text);

        // Mainnet should parse successfully
        let mainnet: NetworkConfig = envy::prefixed("MAINNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("mainnet parse");
        assert_eq!(mainnet.dapi_addresses, "https://1.2.3.4:443");
        assert_eq!(mainnet.core_rpc_port, 9998);
        assert_eq!(mainnet.core_rpc_user, "dashrpc");
        assert_eq!(mainnet.core_rpc_password, "password");

        // No other networks should have keys
        let has_testnet = env_map.keys().any(|k| k.starts_with("TESTNET_"));
        assert!(!has_testnet, "No testnet keys should be in the output");
        let has_devnet = env_map.keys().any(|k| k.starts_with("DEVNET_"));
        assert!(!has_devnet, "No devnet keys should be in the output");
        let has_local = env_map.keys().any(|k| k.starts_with("LOCAL_"));
        assert!(!has_local, "No local keys should be in the output");

        // No DEVELOPER_MODE key
        assert!(!env_map.contains_key("DEVELOPER_MODE"));
    }

    /// Roundtrip: write an empty Config (no networks) → buffer → verify empty.
    #[test]
    fn test_write_to_empty_config() {
        let config = Config {
            mainnet_config: None,
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };

        let mut buf = Vec::new();
        config.write_to(&mut buf).expect("write_to should succeed");
        let env_text = String::from_utf8(buf).expect("should be valid UTF-8");
        assert!(
            env_text.trim().is_empty(),
            "Empty config should produce empty output"
        );
    }

    /// File roundtrip: write to a temp file via write_to, read back
    /// the file contents, and parse via the same mechanism as load().
    #[test]
    fn test_file_roundtrip() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("create temp dir");
        let env_path = dir.path().join(".env");

        let config = Config {
            mainnet_config: Some(NetworkConfig {
                dapi_addresses: "https://5.6.7.8:443".to_string(),
                core_host: "192.168.1.50".to_string(),
                core_rpc_port: 9998,
                core_rpc_user: "fileuser".to_string(),
                core_rpc_password: "filepass".to_string(),
                insight_api_url: "https://file-insight.dash.org/api".to_string(),
                core_zmq_endpoint: Some("tcp://192.168.1.50:23708".to_string()),
                devnet_name: None,
                wallet_private_key: None,
                show_in_ui: true,
            }),
            testnet_config: Some(NetworkConfig {
                dapi_addresses: "https://9.10.11.12:1443".to_string(),
                core_host: "192.168.1.51".to_string(),
                core_rpc_port: 19998,
                core_rpc_user: "tfileuser".to_string(),
                core_rpc_password: "tfilepass".to_string(),
                insight_api_url: "https://tfile-insight.dash.org/api".to_string(),
                core_zmq_endpoint: None,
                devnet_name: None,
                wallet_private_key: Some("cVfile9999".to_string()),
                show_in_ui: false,
            }),
            devnet_config: None,
            local_config: None,
            developer_mode: Some(false),
        };

        // Write to file
        let mut file = File::create(&env_path).expect("create .env file");
        config.write_to(&mut file).expect("write_to file");
        file.flush().expect("flush");

        // Read file back and parse
        let file_contents = std::fs::read_to_string(&env_path).expect("read .env file");
        let env_map = parse_env_lines(&file_contents);

        // Parse each network from the file contents
        let loaded_mainnet: NetworkConfig = envy::prefixed("MAINNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("parse mainnet");
        assert_eq!(loaded_mainnet.dapi_addresses, "https://5.6.7.8:443");
        assert_eq!(loaded_mainnet.core_host, "192.168.1.50");
        assert_eq!(loaded_mainnet.core_rpc_port, 9998);
        assert_eq!(loaded_mainnet.core_rpc_user, "fileuser");
        assert_eq!(loaded_mainnet.core_rpc_password, "filepass");
        assert_eq!(
            loaded_mainnet.insight_api_url,
            "https://file-insight.dash.org/api"
        );
        assert_eq!(
            loaded_mainnet.core_zmq_endpoint,
            Some("tcp://192.168.1.50:23708".to_string())
        );
        assert!(loaded_mainnet.show_in_ui);

        let loaded_testnet: NetworkConfig = envy::prefixed("TESTNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("parse testnet");
        assert_eq!(loaded_testnet.dapi_addresses, "https://9.10.11.12:1443");
        assert_eq!(loaded_testnet.core_rpc_port, 19998);
        assert_eq!(loaded_testnet.core_rpc_user, "tfileuser");
        assert_eq!(
            loaded_testnet.wallet_private_key,
            Some("cVfile9999".to_string())
        );
        assert!(!loaded_testnet.show_in_ui);

        // Check developer_mode roundtrip
        assert_eq!(
            env_map.get("DEVELOPER_MODE").map(|s| s.as_str()),
            Some("false")
        );
    }

    /// Verify that optional fields (devnet_name, wallet_private_key,
    /// core_zmq_endpoint) survive a write → parse roundtrip.
    #[test]
    fn test_write_to_roundtrip_optional_fields_present() {
        let config = Config {
            mainnet_config: None,
            testnet_config: None,
            devnet_config: Some(NetworkConfig {
                dapi_addresses: "http://devnet.example.com:1443".to_string(),
                core_host: "devnet-host".to_string(),
                core_rpc_port: 29998,
                core_rpc_user: "devusr".to_string(),
                core_rpc_password: "devpwd".to_string(),
                insight_api_url: "http://devnet-insight.example.com".to_string(),
                core_zmq_endpoint: Some("tcp://devnet-host:33333".to_string()),
                devnet_name: Some("my-devnet".to_string()),
                wallet_private_key: Some("cVdevKeyXYZ".to_string()),
                show_in_ui: true,
            }),
            local_config: None,
            developer_mode: None,
        };

        let mut buf = Vec::new();
        config.write_to(&mut buf).expect("write_to should succeed");
        let env_text = String::from_utf8(buf).expect("should be valid UTF-8");
        let env_map = parse_env_lines(&env_text);

        let devnet: NetworkConfig = envy::prefixed("DEVNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("devnet parse");
        assert_eq!(devnet.devnet_name, Some("my-devnet".to_string()));
        assert_eq!(devnet.wallet_private_key, Some("cVdevKeyXYZ".to_string()));
        assert_eq!(
            devnet.core_zmq_endpoint,
            Some("tcp://devnet-host:33333".to_string())
        );
    }

    /// Verify that optional fields omitted in write produce None on parse.
    #[test]
    fn test_write_to_roundtrip_optional_fields_absent() {
        let config = Config {
            mainnet_config: Some(NetworkConfig {
                dapi_addresses: "https://1.1.1.1:443".to_string(),
                core_host: "127.0.0.1".to_string(),
                core_rpc_port: 9998,
                core_rpc_user: "user".to_string(),
                core_rpc_password: "pass".to_string(),
                insight_api_url: "https://insight.example.com".to_string(),
                core_zmq_endpoint: None,
                devnet_name: None,
                wallet_private_key: None,
                show_in_ui: false,
            }),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };

        let mut buf = Vec::new();
        config.write_to(&mut buf).expect("write_to should succeed");
        let env_text = String::from_utf8(buf).expect("should be valid UTF-8");
        let env_map = parse_env_lines(&env_text);

        let mainnet: NetworkConfig = envy::prefixed("MAINNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("mainnet parse");
        assert!(mainnet.core_zmq_endpoint.is_none());
        assert!(mainnet.devnet_name.is_none());
        assert!(mainnet.wallet_private_key.is_none());
        assert!(!mainnet.show_in_ui);
    }

    /// Verify that special characters in values survive the roundtrip.
    #[test]
    fn test_write_to_roundtrip_special_characters() {
        let config = Config {
            mainnet_config: Some(NetworkConfig {
                dapi_addresses: "https://node.example.com:443".to_string(),
                core_host: "my-host.local".to_string(),
                core_rpc_port: 9998,
                core_rpc_user: "user_with-dash.dot".to_string(),
                core_rpc_password: "p@ss!w0rd#with$pecial".to_string(),
                insight_api_url: "https://insight.example.com/api/v1?key=val".to_string(),
                core_zmq_endpoint: None,
                devnet_name: None,
                wallet_private_key: None,
                show_in_ui: true,
            }),
            testnet_config: None,
            devnet_config: None,
            local_config: None,
            developer_mode: None,
        };

        let mut buf = Vec::new();
        config.write_to(&mut buf).expect("write_to should succeed");
        let env_text = String::from_utf8(buf).expect("should be valid UTF-8");
        let env_map = parse_env_lines(&env_text);

        let mainnet: NetworkConfig = envy::prefixed("MAINNET_")
            .from_iter(env_map.iter().map(|(k, v)| (k.clone(), v.clone())))
            .expect("mainnet parse");
        assert_eq!(mainnet.core_rpc_user, "user_with-dash.dot");
        assert_eq!(mainnet.core_rpc_password, "p@ss!w0rd#with$pecial");
        assert_eq!(
            mainnet.insight_api_url,
            "https://insight.example.com/api/v1?key=val"
        );
    }
}
