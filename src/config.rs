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
    /// Hostname of the Dash Platform node to connect to
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

    /// Write the current configuration back to the `.env` file so that
    /// subsequent calls to `Config::load()` will reflect changes.
    pub fn save(&self) -> Result<(), ConfigError> {
        let env_file_path =
            app_user_data_file_path(".env").map_err(|e| ConfigError::LoadError(e.to_string()))?;

        // Create / truncate the `.env` file
        let mut env_file =
            File::create(&env_file_path).map_err(|e| ConfigError::LoadError(e.to_string()))?;

        // Helper function to write a single network config to the `.env` file
        let mut write_network_config = |prefix: &str, config: &NetworkConfig| {
            // Each line becomes e.g.  MAINNET_dapi_addresses=...
            // For "local" (regtest), you'll see LOCAL_dapi_addresses=...
            //
            // Use the environment variable scheme you prefer. Make sure it
            // matches what `load()` expects (i.e. `envy::prefixed("MAINNET_")`,
            // etc.).

            writeln!(
                env_file,
                "{}dapi_addresses={}",
                prefix, config.dapi_addresses
            )
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            writeln!(env_file, "{}core_host={}", prefix, config.core_host)
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            writeln!(env_file, "{}core_rpc_port={}", prefix, config.core_rpc_port)
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            writeln!(env_file, "{}core_rpc_user={}", prefix, config.core_rpc_user)
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            writeln!(
                env_file,
                "{}core_rpc_password={}",
                prefix, config.core_rpc_password
            )
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            writeln!(
                env_file,
                "{}insight_api_url={}",
                prefix, config.insight_api_url
            )
            .map_err(|e| ConfigError::LoadError(e.to_string()))?;

            if let Some(core_zmq_endpoint) = &config.core_zmq_endpoint {
                writeln!(
                    env_file,
                    "{}core_zmq_endpoint={}",
                    prefix, core_zmq_endpoint
                )
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            }

            if let Some(devnet_name) = &config.devnet_name {
                // Only write devnet name if it exists
                writeln!(env_file, "{}devnet_name={}", prefix, devnet_name)
                    .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            }
            if let Some(wallet_private_key) = &config.wallet_private_key {
                writeln!(
                    env_file,
                    "{}wallet_private_key={}",
                    prefix, wallet_private_key
                )
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
            }

            // Whether or not to show in UI
            writeln!(env_file, "{}show_in_ui={}", prefix, config.show_in_ui)
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;

            // Add a blank line after each config block
            writeln!(env_file).map_err(|e| ConfigError::LoadError(e.to_string()))?;

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
                .map_err(|e| ConfigError::LoadError(e.to_string()))?;
        }

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
    pub fn dapi_address_list(&self) -> AddressList {
        AddressList::from_str(&self.dapi_addresses).expect("Could not parse DAPI addresses")
    }

    /// Insight API URI
    #[allow(dead_code)] // May be used for insight API access
    pub fn insight_api_uri(&self) -> Uri {
        Uri::from_str(&self.insight_api_url).expect("invalid insight API URL")
    }

    /// Update just the `core_rpc_password` in a builder-like manner.
    /// Returns a new `NetworkConfig` with the updated password.
    pub fn update_core_rpc_password(mut self, new_password: String) -> Self {
        self.core_rpc_password = new_password;
        self
    }
}
