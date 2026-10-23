use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::ContextProvider;
use dash_sdk::sdk::min_protocol_version;
use dash_sdk::{RequestSettings, Sdk, SdkBuilder};
use std::time::Duration;
use tracing::info;

/// Request settings every DET SDK client runs with.
fn request_settings() -> RequestSettings {
    RequestSettings {
        connect_timeout: Some(Duration::from_secs(1)),
        timeout: Some(Duration::from_secs(10)),
        retries: Some(6),
        ban_failed_address: Some(true),
        max_decoding_message_size: None,
    }
}

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
    let request_settings = request_settings();

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
    use crate::backend_task::error::TaskError;
    use crate::context_provider::MASTERNODE_LIST_NOT_READY_DETAIL;
    use dash_sdk::ProofVerifierError;
    use dash_sdk::dapi_client::{
        Address, ExecutionError, ExecutionResult, update_address_ban_status,
    };
    use dash_sdk::error::ContextProviderError;

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

    /// Pins the pinned SDK's behaviour that makes pre-sync proof reads harmful:
    /// the context provider's "masternode list not ready" answer surfaces as a
    /// retryable proof error, and DET's settings ban the node it came from.
    /// Screens must therefore not send proof-verifying reads before readiness.
    /// If an SDK upgrade stops this ban, the read gates can be relaxed.
    #[test]
    fn a_proof_read_before_masternodes_are_ready_bans_the_node() {
        let not_ready = || {
            dash_sdk::Error::Proof(ProofVerifierError::ContextProviderError(
                ContextProviderError::Config(MASTERNODE_LIST_NOT_READY_DETAIL.to_string()),
            ))
        };
        assert!(
            matches!(
                TaskError::from(not_ready()),
                TaskError::MasternodeListNotReady { .. }
            ),
            "this is the error DET reports for a read sent before readiness"
        );

        let address: Address = "https://127.0.0.1:1443".parse().expect("address");
        let mut addresses = AddressList::new();
        addresses.add(address.clone());
        assert!(!addresses.is_banned(&address));
        let result: ExecutionResult<(), dash_sdk::Error> = Err(ExecutionError {
            inner: not_ready(),
            retries: 0,
            address: Some(address.clone()),
        });
        update_address_ban_status(&addresses, &result, &request_settings().finalize());
        assert!(
            addresses.is_banned(&address),
            "the SDK bans a healthy node for a local readiness failure"
        );
    }
}
