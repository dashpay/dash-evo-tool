use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::ContextProvider;
use dash_sdk::sdk::min_protocol_version;
use dash_sdk::{RequestSettings, Sdk, SdkBuilder};
use std::time::Duration;
use tracing::info;

/// Build an SDK for `network` that tracks the network's protocol version.
///
/// The SDK is never pinned: it starts at upstream's per-network minimum (the
/// lowest version the network still runs) and ratchets upward to whatever the
/// network reports in proven responses. `carried_protocol_version` is the
/// version a previous SDK of this network already reached (0 if none), so a
/// replacement does not fall back below it.
pub fn initialize_sdk<P: ContextProvider + 'static>(
    address_list: AddressList,
    network: Network,
    context_provider: P,
    carried_protocol_version: u32,
) -> Result<Sdk, String> {
    let request_settings = RequestSettings {
        connect_timeout: Some(Duration::from_secs(1)),
        timeout: Some(Duration::from_secs(10)),
        retries: Some(6),
        ban_failed_address: Some(true),
        max_decoding_message_size: None,
    };

    let mut builder = SdkBuilder::new(address_list)
        .with_network(network)
        .with_context_provider(context_provider)
        .with_settings(request_settings);
    if let Some(seed) = sdk_seed(network, carried_protocol_version) {
        builder = builder.with_initial_version(seed);
    }
    let sdk = builder
        .build()
        .map_err(|e| format!("Failed to build SDK: {e}"))?;

    info!(
        ?network,
        protocol_version = sdk.protocol_version_number(),
        "SDK initialized successfully"
    );

    Ok(sdk)
}

/// The seed replacing upstream's per-network default: the carried version,
/// when it is known and above that default. `None` keeps the default.
fn sdk_seed(network: Network, carried_protocol_version: u32) -> Option<&'static PlatformVersion> {
    if carried_protocol_version <= min_protocol_version(network) {
        return None;
    }
    PlatformVersion::get_optional(carried_protocol_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETWORKS: [Network; 4] = [
        Network::Mainnet,
        Network::Testnet,
        Network::Devnet,
        Network::Regtest,
    ];

    /// Nothing carried, or nothing above the network minimum: upstream's
    /// per-network default seeds the SDK.
    #[test]
    fn the_network_minimum_seeds_a_fresh_sdk() {
        for network in NETWORKS {
            let minimum = min_protocol_version(network);
            for carried in [0, minimum - 1, minimum] {
                assert!(
                    sdk_seed(network, carried).is_none(),
                    "{network:?} carried {carried}"
                );
            }
        }
    }

    /// A version a previous SDK already reached is kept, unless this build
    /// does not know it.
    #[test]
    fn a_carried_version_above_the_minimum_is_kept() {
        let latest = PlatformVersion::latest().protocol_version;
        assert_eq!(
            sdk_seed(Network::Mainnet, latest).map(|v| v.protocol_version),
            Some(latest)
        );
        assert!(sdk_seed(Network::Mainnet, latest + 1).is_none());
    }
}
