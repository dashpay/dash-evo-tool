use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
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
}

impl Config {
    /// Loads the configuration for all networks from environment variables and `.env` file.
    pub fn load() -> Result<Self, ConfigError> {
        // Load the .env file if available
        if let Err(err) = dotenvy::from_path(".env") {
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
    /// List of DAPI addresses
    pub fn dapi_address_list(&self) -> AddressList {
        AddressList::from(self.dapi_addresses.as_str())
    }
}
