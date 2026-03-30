//! Dynamic DAPI node discovery via the Dash trusted masternode endpoint.
//!
//! When no explicit DAPI addresses are configured in `.env`, this module
//! contacts a Dash-operated HTTPS service to discover available masternodes:
//!
//! - **Mainnet**: `https://quorums.mainnet.networks.dash.org/masternodes`
//! - **Testnet**: `https://quorums.testnet.networks.dash.org/masternodes`
//!
//! Devnet supports dynamic discovery when a `devnet_name` is configured.
//! Regtest does **not** support dynamic discovery and requires explicit
//! DAPI addresses in the `.env` configuration.
//!
//! ## Trust model
//!
//! The discovery endpoint is operated by DCG (Dash Core Group) and served
//! over TLS. The application trusts this endpoint to return a correct list
//! of masternodes. An attacker who compromises the endpoint or performs a
//! TLS MITM could direct the client to malicious DAPI nodes, which would
//! then be subject to Platform's own proof-verification before any state
//! is accepted. Discovery is therefore a *convenience trust* layer, not a
//! *security trust* layer -- incorrect results degrade availability but
//! cannot forge Platform proofs.
//!
//! Results are not cached across restarts; discovery runs once per
//! `AppContext` initialization for the active network.

use dash_sdk::dapi_client::{Address as DapiAddress, AddressList, AddressListError};
use dash_sdk::dpp::dashcore::Network;
use rs_sdk_trusted_context_provider::{TrustedContextProviderError, TrustedHttpContextProvider};
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::time::Duration;

/// Errors from DAPI address resolution and discovery.
#[derive(Debug, thiserror::Error)]
pub enum DapiDiscoveryError {
    /// Explicitly configured DAPI addresses could not be parsed.
    #[error(
        "Could not parse the configured node addresses. Check the DAPI addresses in your network settings."
    )]
    InvalidAddresses {
        #[source]
        source: AddressListError,
    },

    /// Network requires explicit DAPI addresses in `.env`.
    #[error(
        "Node addresses must be configured for this network. Add DAPI addresses in your network settings."
    )]
    AddressesRequired { network: Network },

    /// No tokio runtime available for async discovery.
    #[error("The application could not start node discovery. Please restart the application.")]
    NoRuntime,

    /// The trusted discovery provider could not be initialized.
    #[error("Could not start node discovery. Check your network connection and retry.")]
    ProviderInit {
        #[source]
        source: TrustedContextProviderError,
    },

    /// Discovery HTTP request timed out.
    #[error("Node discovery timed out. Check your network connection and retry.")]
    Timeout,

    /// Discovery HTTP request failed.
    #[error("Node discovery failed. Check your network connection and retry.")]
    RequestFailed {
        #[source]
        source: TrustedContextProviderError,
    },

    /// Discovery succeeded but returned no usable addresses.
    #[error(
        "No available nodes were found. The network may be temporarily unavailable -- try again later."
    )]
    NoResults,
}

/// Hardcoded fallback seed nodes for mainnet (a small subset of known-good
/// masternodes). Used only when dynamic discovery fails so the app can still
/// start in a degraded state.
const MAINNET_SEED_NODES: &[&str] = &[
    "https://52.33.9.172:443",
    "https://134.255.182.186:443",
    "https://168.119.102.10:443",
    "https://65.108.246.145:443",
    "https://37.27.83.17:443",
    "https://135.181.110.216:443",
];

/// Hardcoded fallback seed nodes for testnet.
const TESTNET_SEED_NODES: &[&str] = &[
    "https://34.214.48.68:1443",
    "https://52.12.176.90:1443",
    "https://52.34.144.50:1443",
    "https://44.240.98.102:1443",
    "https://54.201.32.131:1443",
    "https://52.10.229.11:1443",
];

/// Resolve DAPI addresses synchronously.
///
/// If explicit addresses are configured, parses them directly. For mainnet,
/// testnet, and devnet (when `devnet_name` is set) without explicit config,
/// attempts dynamic discovery via `tokio::task::block_in_place` (requires a
/// multi-thread tokio runtime). When discovery fails for mainnet or testnet,
/// falls back to hardcoded seed nodes so the app can still start. Devnet has
/// no seed-node fallback, so discovery failure is final. Returns an error
/// only if all resolution methods fail.
pub fn resolve_dapi_addresses_sync(
    network: Network,
    dapi_addresses: &Option<String>,
    devnet_name: Option<&str>,
) -> Result<AddressList, DapiDiscoveryError> {
    if let Some(addrs) = dapi_addresses {
        let trimmed = addrs.trim();
        if !trimmed.is_empty() {
            return AddressList::from_str(trimmed)
                .map_err(|source| DapiDiscoveryError::InvalidAddresses { source });
        }
    }

    match network {
        Network::Mainnet | Network::Testnet => {}
        Network::Devnet if devnet_name.is_some() => {}
        _ => return Err(DapiDiscoveryError::AddressesRequired { network }),
    }

    let discovery_result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // block_in_place requires a multi-thread runtime; on current-thread
            // it would panic.  Guard defensively for future callers.
            if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                Err(DapiDiscoveryError::NoRuntime)
            } else {
                tokio::task::block_in_place(|| {
                    handle.block_on(try_discover_nodes(network, devnet_name))
                })
            }
        }
        Err(_) => Err(DapiDiscoveryError::NoRuntime),
    };

    match discovery_result {
        Ok(list) => Ok(list),
        Err(e) => {
            // Devnet has no hardcoded seed nodes -- discovery failure is final.
            if network == Network::Devnet {
                return Err(e);
            }
            tracing::warn!(
                ?network,
                error = %e,
                "Dynamic DAPI discovery failed, falling back to seed nodes"
            );
            fallback_seed_nodes(network)
        }
    }
}

/// Build an `AddressList` from hardcoded seed nodes for the given network.
fn fallback_seed_nodes(network: Network) -> Result<AddressList, DapiDiscoveryError> {
    let seeds = match network {
        Network::Mainnet => MAINNET_SEED_NODES,
        Network::Testnet => TESTNET_SEED_NODES,
        _ => return Err(DapiDiscoveryError::AddressesRequired { network }),
    };

    let mut list = AddressList::new();
    for seed in seeds {
        if let Ok(addr) = DapiAddress::from_str(seed) {
            list.add(addr);
        }
    }

    if list.is_empty() {
        return Err(DapiDiscoveryError::NoResults);
    }

    tracing::info!(
        ?network,
        count = list.len(),
        "Using fallback seed nodes for DAPI"
    );
    Ok(list)
}

async fn try_discover_nodes(
    network: Network,
    devnet_name: Option<&str>,
) -> Result<AddressList, DapiDiscoveryError> {
    let provider = TrustedHttpContextProvider::new(
        network,
        devnet_name.map(|s| s.to_string()),
        NonZeroUsize::new(10).unwrap(),
    )
    .map_err(|source| DapiDiscoveryError::ProviderInit { source })?;

    let urls = tokio::time::timeout(
        Duration::from_secs(10),
        provider.fetch_masternode_addresses(),
    )
    .await
    .map_err(|_| DapiDiscoveryError::Timeout)?
    .map_err(|source| DapiDiscoveryError::RequestFailed { source })?;

    let mut address_list = AddressList::new();
    for url in urls {
        match DapiAddress::from_str(url.as_str()) {
            Ok(addr) => {
                address_list.add(addr);
            }
            Err(e) => tracing::debug!(url = %url, error = %e, "Skipping invalid address"),
        }
    }

    if address_list.is_empty() {
        return Err(DapiDiscoveryError::NoResults);
    }

    tracing::info!(
        ?network,
        count = address_list.len(),
        "Discovered DAPI nodes dynamically"
    );

    Ok(address_list)
}
