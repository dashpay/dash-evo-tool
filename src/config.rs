use std::{path::PathBuf, str::FromStr};

use dash_sdk::sdk::Uri;
use dpp::dashcore::Network;
use rs_dapi_client::AddressList;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
/// Configuration for platform explorer.
///
/// Content of this configuration is loaded from environment variables or `.env`
/// file when the [Config::load()] is called.
/// Variable names in the enviroment and `.env` file must be prefixed with
/// either [LOCAL_EXPLORER_](Config::CONFIG_PREFIX) or
/// [TESTNET_EXPLORER_](Config::CONFIG_PREFIX) and written as
/// SCREAMING_SNAKE_CASE (e.g. `EXPLORER_DAPI_ADDRESSES`).
pub struct Config {
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
    /// Network name
    pub network: String,
    /// Optional wallet private key to instantiate the wallet
    pub wallet_private_key: Option<String>,
}

impl Config {
    /// Prefix of configuration options in the environment variables and `.env`
    /// file.
    const CONFIG_PREFIX: &'static str = "EXPLORER_";

    /// Loads a local configuration from operating system environment variables
    /// and `.env` file.
    ///
    /// Create new [Config] with data from environment variables and
    /// `.env` file. Variable names in the
    /// environment and `.env` file must be converted to SCREAMING_SNAKE_CASE
    /// and prefixed with [LOCAL_EXPLORER_](Config::CONFIG_PREFIX).
    pub fn load() -> Self {
        // load config from .env file
        if let Err(err) = dotenvy::from_path(".env") {
            tracing::warn!(?err, "failed to load config file");
        }

        let config: Self = envy::prefixed(Self::CONFIG_PREFIX)
            .from_env()
            .expect("configuration error");

        if !config.is_valid() {
            panic!("invalid configuration: {:?}", config);
        }

        config
    }

    /// Check if configuration is set
    pub fn is_valid(&self) -> bool {
        !self.core_rpc_user.is_empty()
            && !self.core_rpc_password.is_empty()
            && self.core_rpc_port != 0
            && !self.dapi_addresses.is_empty()
            && Uri::from_str(&self.insight_api_url).is_ok()
            && Network::from_str(&self.core_network_name()).is_ok()
    }

    pub fn core_network(&self) -> Network {
        Network::from_str(self.core_network_name()).expect("invalid network")
    }

    /// List of DAPI addresses
    pub fn dapi_address_list(&self) -> AddressList {
        AddressList::from(self.dapi_addresses.as_str())
    }

    /// Insight API URI
    pub fn insight_api_uri(&self) -> Uri {
        Uri::from_str(&self.insight_api_url).expect("invalid insight API URL")
    }

    /// Returns path to the state file
    pub fn state_file_path(&self) -> PathBuf {
        format!("{}_explorer.state", self.network).into()
    }

    fn core_network_name(&self) -> &str {
        if self.network == "local" {
            "regtest"
        } else {
            &self.network
        }
    }
}
