use dash_sdk::dapi_client::{Address as DapiAddress, AddressList};
use dash_sdk::dpp::dashcore::Network;
use rs_sdk_trusted_context_provider::TrustedHttpContextProvider;
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::time::Duration;

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
) -> Result<AddressList, String> {
    if let Some(addrs) = dapi_addresses {
        let trimmed = addrs.trim();
        if !trimmed.is_empty() {
            return AddressList::from_str(trimmed)
                .map_err(|e| format!("Invalid DAPI addresses: {e}"));
        }
    }

    match network {
        Network::Mainnet | Network::Testnet => {}
        _ => return Err("DAPI addresses must be configured in .env for this network".into()),
    }

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| {
            handle.block_on(try_discover_nodes(network, devnet_name))
        }),
        Err(_) => Err("No tokio runtime available for dynamic DAPI discovery".to_string()),
    }
}

async fn try_discover_nodes(
    network: Network,
    devnet_name: Option<&str>,
) -> Result<AddressList, String> {
    let provider = TrustedHttpContextProvider::new(
        network,
        devnet_name.map(|s| s.to_string()),
        NonZeroUsize::new(10).unwrap(),
    )
    .map_err(|e| format!("Could not initialize node discovery: {e}"))?;

    let urls = tokio::time::timeout(
        Duration::from_secs(10),
        provider.fetch_masternode_addresses(),
    )
    .await
    .map_err(|_| "Node discovery timed out after 10 seconds".to_string())?
    .map_err(|e| format!("Node discovery request failed: {e}"))?;

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
        return Err("Dynamic discovery returned no valid DAPI addresses".to_string());
    }

    tracing::info!(
        ?network,
        count = address_list.len(),
        "Discovered DAPI nodes dynamically"
    );

    Ok(address_list)
}
