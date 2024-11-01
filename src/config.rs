use std::str::FromStr;

use crate::app_dir::app_user_data_file_path;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::sdk::Uri;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// The mainnet network config
    pub mainnet_config: Option<NetworkConfig>,
    /// The testnet network config
    pub testnet_config: Option<NetworkConfig>,
}

impl Config {
    pub fn config_for_network(&self, network: Network) -> &Option<NetworkConfig> {
        match network {
            Network::Dash => &self.mainnet_config,
            Network::Testnet => &self.testnet_config,
            Network::Devnet => &None,
            Network::Regtest => &None,
            _ => &None,
        }
    }
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
    /// Devnet network name if one exists
    pub devnet_name: Option<String>,
    /// Optional wallet private key to instantiate the wallet
    pub wallet_private_key: Option<String>,
    /// Should this network be visible in the UI
    pub show_in_ui: bool,
}

impl Config {
    /// Loads the configuration for all networks from environment variables and `.env` file.
    pub fn load() -> Result<Self, ConfigError> {
        // Load the .env file if available
        let env_file_path = app_user_data_file_path(".env").expect("should create .env file path");
        if let Err(err) = dotenvy::from_path(env_file_path) {
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

        if mainnet_config.is_none() && testnet_config.is_none() {
            return Err(ConfigError::NoValidConfigs);
        } else if mainnet_config.is_none() {
            return Err(ConfigError::LoadError(
                "Failed to load mainnet configuration".into(),
            ));
        } else if testnet_config.is_none() {
            tracing::warn!(
                "Failed to load testnet configuration, but successfully loaded mainnet config"
            );
        }

        Ok(Config {
            mainnet_config,
            testnet_config,
        })
    }
}

impl NetworkConfig {
    /// Check if configuration is set
    pub fn is_valid(&self) -> bool {
        !self.core_rpc_user.is_empty()
            && !self.core_rpc_password.is_empty()
            && self.core_rpc_port != 0
            && !self.dapi_addresses.is_empty()
            && Uri::from_str(&self.insight_api_url).is_ok()
    }

    /// List of DAPI addresses
    pub fn dapi_address_list(&self) -> AddressList {
        AddressList::from(self.dapi_addresses.as_str())
    }

    /// Insight API URI
    pub fn insight_api_uri(&self) -> Uri {
        Uri::from_str(&self.insight_api_url).expect("invalid insight API URL")
    }
}
