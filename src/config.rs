use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use crate::app_dir::{app_user_data_file_path, data_file_path};
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use serde::Deserialize;
use tempfile::NamedTempFile;

/// Known old hardcoded mainnet DAPI address lists that should be migrated
/// to dynamic discovery. These are exact-match values from previous .env.example
/// files (pre-update and post-update from mnowatch.org scrape).
const KNOWN_OLD_MAINNET_ADDRESSES: &[&str] = &[
    // Original .env.example mainnet addresses (pre-mnowatch update)
    "https://104.200.24.196:443,https://134.255.182.185:443,https://134.255.182.186:443,https://134.255.182.187:443,https://134.255.183.247:443,https://134.255.183.248:443,https://134.255.183.250:443,https://135.181.110.216:443,https://146.59.4.9:443,https://147.135.199.138:443,https://149.28.241.190:443,https://149.28.247.165:443,https://157.10.199.125:443,https://157.10.199.77:443,https://157.10.199.79:443,https://157.10.199.82:443,https://157.66.81.130:443,https://157.66.81.162:443,https://157.66.81.218:443,https://157.90.238.161:443,https://159.69.204.162:443,https://167.179.90.255:443,https://167.88.169.16:443,https://168.119.102.10:443,https://172.104.90.249:443,https://173.212.239.124:443,https://173.249.53.139:443,https://178.157.91.184:443,https://185.158.107.124:443,https://185.192.96.70:443,https://185.194.216.84:443,https://185.197.250.227:443,https://185.198.234.17:443,https://185.215.166.126:443,https://188.208.196.183:443,https://188.245.90.255:443,https://192.248.178.237:443,https://193.203.15.209:443,https://194.146.13.7:443,https://194.195.87.34:443,https://198.7.115.43:443,https://207.244.247.40:443,https://213.199.34.248:443,https://213.199.34.250:443,https://213.199.34.251:443,https://213.199.35.15:443,https://213.199.35.18:443,https://213.199.35.6:443,https://213.199.44.112:443,https://2.58.82.231:443,https://31.220.84.93:443,https://31.220.85.180:443,https://31.220.88.116:443,https://37.27.83.17:443,https://37.60.236.151:443,https://37.60.236.161:443,https://37.60.236.201:443,https://37.60.236.212:443,https://37.60.236.247:443,https://37.60.236.249:443,https://37.60.243.119:443,https://37.60.243.59:443,https://37.60.244.220:443,https://44.240.99.214:443,https://49.12.102.105:443,https://49.13.154.121:443,https://49.13.193.251:443,https://49.13.237.193:443,https://49.13.28.255:443,https://51.195.118.43:443,https://51.83.191.208:443,https://5.189.186.78:443,https://52.10.213.198:443,https://52.33.9.172:443,https://54.69.95.118:443,https://5.75.133.148:443,https://64.23.134.67:443,https://65.108.246.145:443,https://65.109.65.126:443,https://65.21.145.147:443,https://79.137.71.84:443,https://81.17.101.141:443,https://91.107.204.136:443,https://91.107.226.241:443,https://93.190.140.101:443,https://93.190.140.111:443,https://93.190.140.112:443,https://93.190.140.114:443,https://93.190.140.162:443,https://95.216.146.18:443",
    // mnowatch.org-scraped mainnet addresses (279 nodes, 2026-03-17)
    "https://147.45.183.128:443,https://216.238.75.46:443,https://89.125.209.110:443,https://84.247.180.201:443,https://134.255.182.186:443,https://93.115.172.39:443,https://5.189.164.253:443,https://38.242.197.189:443,https://83.222.9.253:443,https://95.111.241.20:443,https://66.245.196.52:443,https://173.212.245.118:443,https://202.71.14.79:443,https://185.252.234.238:443,https://178.215.237.134:443,https://207.180.233.6:443,https://89.125.209.69:443,https://136.244.99.17:443,https://173.212.232.90:443,https://92.255.76.59:443,https://37.60.244.253:443,https://157.90.238.161:443,https://85.92.111.111:443,https://213.199.54.171:443,https://84.247.180.198:443,https://82.211.21.252:443,https://161.97.75.36:443,https://23.88.63.58:443,https://207.244.247.40:443,https://45.32.70.131:443,https://31.220.91.43:443,https://52.33.9.172:443,https://161.97.88.199:443,https://109.199.124.30:443,https://185.198.234.17:443,https://139.84.236.208:443,https://38.242.218.26:443,https://207.180.224.96:443,https://62.171.170.222:443,https://75.119.138.9:443,https://75.119.153.10:443,https://31.220.91.60:443,https://82.211.21.251:443,https://95.179.159.65:443,https://44.240.99.214:443,https://5.75.133.148:443,https://213.199.54.37:443,https://161.97.88.219:443,https://89.35.131.149:443,https://192.248.178.237:443,https://161.97.153.122:443,https://37.60.235.218:443,https://207.180.241.242:443,https://195.26.254.228:443,https://45.77.11.194:443,https://139.84.232.129:443,https://161.97.175.233:443,https://147.45.103.99:443,https://89.125.209.195:443,https://91.198.108.35:443,https://85.193.90.107:443,https://161.97.180.105:443,https://213.199.54.34:443,https://65.108.246.145:443,https://64.176.10.71:443,https://158.247.247.241:443,https://37.60.254.213:443,https://149.102.140.101:443,https://139.180.143.115:443,https://37.60.235.205:443,https://213.199.44.112:443,https://213.199.53.161:443,https://178.253.42.64:443,https://162.212.35.100:443,https://185.239.209.6:443,https://62.171.138.186:443,https://173.212.196.214:443,https://37.60.254.202:443,https://134.255.182.185:443,https://139.84.137.143:443,https://144.91.87.82:443,https://161.97.91.217:443,https://82.211.21.249:443,https://194.163.159.171:443,https://80.240.19.200:443,https://144.126.141.62:443,https://173.249.21.12:443,https://161.97.159.172:443,https://194.163.156.190:443,https://139.84.170.10:443,https://164.68.118.37:443,https://143.198.145.184:443,https://84.247.180.200:443,https://37.60.234.205:443,https://43.133.171.101:443,https://192.248.175.198:443,https://81.200.152.144:443,https://161.97.142.210:443,https://43.167.244.109:443,https://92.53.120.89:443,https://172.236.244.81:443,https://62.171.144.192:443,https://146.59.153.204:443,https://84.247.180.190:443,https://185.215.164.84:443,https://172.238.7.25:443,https://157.173.122.20:443,https://45.153.70.126:443,https://185.141.216.4:443,https://82.208.20.153:443,https://95.111.239.54:443,https://85.190.243.3:443,https://51.195.235.166:443,https://156.67.29.45:443,https://82.211.21.38:443,https://93.115.172.37:443,https://89.35.131.39:443,https://93.115.172.38:443,https://161.97.104.37:443,https://75.119.128.71:443,https://161.97.117.125:443,https://49.13.28.255:443,https://52.36.102.91:443,https://139.99.201.103:443,https://109.199.120.79:443,https://161.97.74.173:443,https://168.119.102.10:443,https://45.135.180.79:443,https://49.13.237.193:443,https://45.135.180.130:443,https://173.212.251.130:443,https://38.242.206.103:443,https://37.60.254.201:443,https://75.119.156.254:443,https://37.27.83.17:443,https://38.242.145.178:443,https://45.135.180.114:443,https://66.29.147.83:443,https://161.97.83.102:443,https://37.60.236.230:443,https://89.35.131.23:443,https://89.125.50.206:443,https://37.60.244.35:443,https://38.242.206.56:443,https://194.163.166.76:443,https://70.34.206.123:443,https://51.158.169.237:443,https://213.199.54.35:443,https://108.61.165.170:443,https://89.35.131.219:443,https://161.97.163.75:443,https://185.166.217.154:443,https://157.173.122.21:443,https://37.60.236.41:443,https://38.242.200.227:443,https://193.168.3.82:443,https://149.28.223.171:443,https://217.76.54.175:443,https://167.86.93.21:443,https://109.123.244.131:443,https://155.133.26.122:443,https://51.195.47.118:443,https://45.135.180.70:443,https://167.88.169.16:443,https://216.238.99.9:443,https://37.60.244.87:443,https://91.222.237.98:443,https://157.173.122.26:443,https://82.211.21.18:443,https://52.10.213.198:443,https://139.84.231.221:443,https://45.94.58.58:443,https://178.18.254.136:443,https://84.247.180.205:443,https://161.97.106.14:443,https://49.13.193.251:443,https://78.141.225.100:443,https://77.232.129.86:443,https://91.198.108.37:443,https://167.86.94.138:443,https://89.23.112.100:443,https://95.179.241.182:443,https://92.63.176.202:443,https://188.225.39.14:443,https://176.57.213.170:443,https://93.115.172.36:443,https://82.211.21.16:443,https://37.60.235.224:443,https://95.216.146.18:443,https://167.114.153.110:443,https://37.60.235.89:443,https://95.111.233.139:443,https://109.199.97.144:443,https://37.60.235.24:443,https://31.220.84.93:443,https://161.97.180.182:443,https://161.97.91.68:443,https://89.125.209.120:443,https://89.125.50.14:443,https://161.97.179.214:443,https://45.85.147.192:443,https://37.60.254.205:443,https://109.199.124.135:443,https://158.220.101.10:443,https://87.228.24.64:443,https://5.189.151.7:443,https://158.247.208.247:443,https://37.60.254.211:443,https://64.176.35.235:443,https://46.62.250.247:443,https://185.215.164.186:443,https://161.97.102.156:443,https://89.125.209.106:443,https://213.171.12.108:443,https://91.198.108.36:443,https://49.12.102.105:443,https://37.60.246.228:443,https://89.125.209.178:443,https://147.45.236.121:443,https://72.62.58.108:443,https://62.171.136.245:443,https://54.69.95.118:443,https://31.220.77.84:443,https://185.119.57.243:443,https://161.97.96.120:443,https://89.125.209.234:443,https://195.201.238.55:443,https://135.181.110.216:443,https://207.180.213.141:443,https://89.223.120.216:443,https://45.76.141.74:443,https://161.97.66.31:443,https://161.97.85.182:443,https://84.247.187.76:443,https://188.245.90.255:443,https://82.211.21.250:443,https://144.91.106.188:443,https://167.235.102.194:443,https://164.68.114.36:443,https://43.167.239.145:443,https://207.180.233.246:443,https://167.86.77.58:443,https://185.239.208.110:443,https://75.119.137.66:443,https://157.173.122.25:443,https://82.211.21.40:443,https://89.125.209.152:443,https://62.171.170.14:443,https://45.77.129.235:443,https://185.209.230.13:443,https://75.119.152.219:443,https://37.60.234.168:443,https://89.23.117.144:443,https://37.60.243.59:443,https://89.35.131.218:443,https://193.168.3.112:443,https://5.189.145.80:443,https://178.18.244.255:443,https://37.60.235.39:443,https://77.221.148.204:443,https://89.125.209.27:443,https://91.198.108.38:443,https://207.180.218.245:443,https://77.42.74.68:443,https://158.220.101.28:443,https://89.35.131.158:443,https://161.97.106.164:443,https://185.202.236.58:443,https://173.199.71.83:443,https://185.215.166.126:443,https://57.131.28.197:443,https://217.25.89.156:443,https://72.60.38.160:443,https://82.97.240.124:443,https://89.125.209.133:443,https://213.159.77.221:443,https://109.199.119.192:443,https://114.132.172.215:443,https://89.125.209.156:443",
];

const KNOWN_OLD_TESTNET_ADDRESSES: &[&str] = &[
    "https://34.214.48.68:1443,https://52.12.176.90:1443,https://52.34.144.50:1443,https://44.240.98.102:1443,https://54.201.32.131:1443,https://52.10.229.11:1443,https://52.13.132.146:1443,https://52.40.219.41:1443,https://54.149.33.167:1443,https://35.164.23.245:1443,https://52.33.28.47:1443,https://52.43.13.92:1443,https://52.89.154.48:1443,https://52.24.124.162:1443,https://35.85.21.179:1443,https://54.187.14.232:1443,https://54.68.235.201:1443,https://52.13.250.182:1443",
];

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
    /// Hostname of Dash Platform node to connect to.
    /// `None` or empty means dynamic discovery will be used.
    /// Dynamic discovery is currently supported for Mainnet and Testnet only.
    /// Devnet and Regtest require explicit addresses.
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

    /// Migrate old hardcoded DAPI addresses in the `.env` file to dynamic discovery.
    ///
    /// Scans for `MAINNET_dapi_addresses=` and `TESTNET_dapi_addresses=` lines whose
    /// values match a known old hardcoded list. Matching lines are commented out so
    /// that `envy` will no longer parse them and dynamic discovery kicks in.
    pub fn migrate_env_file_if_needed(env_file_path: &Path) {
        let content = match std::fs::read_to_string(env_file_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[migration] Could not read .env file at {}: {e}",
                    env_file_path.display()
                );
                return;
            }
        };

        let mut changed = false;
        let mut new_lines: Vec<String> = Vec::new();

        for line in content.lines() {
            if let Some(value) = line.strip_prefix("MAINNET_dapi_addresses=")
                && KNOWN_OLD_MAINNET_ADDRESSES
                    .iter()
                    .any(|old| value.trim() == *old)
            {
                new_lines.push(format!("# {line} # Migrated to dynamic discovery"));
                changed = true;
                continue;
            }
            if let Some(value) = line.strip_prefix("TESTNET_dapi_addresses=")
                && KNOWN_OLD_TESTNET_ADDRESSES
                    .iter()
                    .any(|old| value.trim() == *old)
            {
                new_lines.push(format!("# {line} # Migrated to dynamic discovery"));
                changed = true;
                continue;
            }
            new_lines.push(line.to_string());
        }

        if !changed {
            return;
        }

        let parent_dir = match env_file_path.parent() {
            Some(p) => p,
            None => {
                eprintln!(
                    "[migration] No parent directory for .env file at {}",
                    env_file_path.display()
                );
                return;
            }
        };

        let mut new_content = new_lines.join("\n");
        if content.ends_with('\n') {
            new_content.push('\n');
        }

        let mut tmp = match NamedTempFile::new_in(parent_dir) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[migration] Failed to create temp file for .env migration: {e}");
                return;
            }
        };
        if let Err(e) = tmp.write_all(new_content.as_bytes()) {
            eprintln!("[migration] Failed to write migrated .env file: {e}");
            return;
        }
        if let Err(e) = tmp.as_file().sync_all() {
            eprintln!("[migration] Failed to sync migrated .env file: {e}");
            return;
        }
        if let Err(e) = tmp.persist(env_file_path) {
            eprintln!("[migration] Failed to persist migrated .env file: {e}");
            return;
        }

        // Restrict file permissions on Unix (config contains RPC credentials).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = std::fs::set_permissions(env_file_path, perms) {
                eprintln!("[migration] Could not set config file permissions to 0600: {e}");
            }
        }

        eprintln!("[migration] Migrated old hardcoded DAPI addresses to dynamic discovery");
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
