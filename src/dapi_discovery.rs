//! Dynamic DAPI node discovery via the Dash trusted masternode endpoint.
//!
//! When no explicit DAPI addresses are configured in `.env`, this module
//! contacts a Dash-operated HTTPS service to discover available masternodes:
//!
//! - **Mainnet**: `https://quorums.mainnet.networks.dash.org/masternodes`
//! - **Testnet**: `https://quorums.testnet.networks.dash.org/masternodes`
//! - **Devnet**: `https://quorums.devnet.<name>.networks.dash.org/masternodes`
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
    #[error("Could not parse the configured node addresses. Check the DAPI addresses in your network settings.")]
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
    #[error("No available nodes were found. The network may be temporarily unavailable -- try again later.")]
    NoResults,
}

/// Resolve DAPI addresses synchronously.
///
/// If explicit addresses are configured, parses them directly. For mainnet and
/// testnet without explicit config, attempts dynamic discovery via
/// `tokio::task::block_in_place` (requires a tokio runtime). Returns an error
/// if discovery fails or no runtime is available.
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
        _ => return Err(DapiDiscoveryError::AddressesRequired { network }),
    }

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| {
            handle.block_on(try_discover_nodes(network, devnet_name))
        }),
        Err(_) => Err(DapiDiscoveryError::NoRuntime),
    }
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
