use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::{PlatformVersion, v14::PLATFORM_V14};
use dash_sdk::platform::ContextProvider;

/// The oldest Platform version DET uses to build new operations.
pub const MIN_PLATFORM_VERSION: &PlatformVersion = &PLATFORM_V14;
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
        // Upstream default (ban); requests to tightly limited methods opt out.
        ban_busy_address: None,
        max_decoding_message_size: None,
    }
}

/// Build an SDK for `network` that tracks the network's protocol version.
///
/// Starts at DET's minimum on every network and upgrades after verified proofs.
/// Rebuilding starts a fresh observation; the seed does not prove network support.
pub fn initialize_sdk<P: ContextProvider + 'static>(
    address_list: AddressList,
    network: Network,
    context_provider: P,
) -> Result<Sdk, Box<dash_sdk::Error>> {
    let request_settings = request_settings();

    let sdk = SdkBuilder::new(address_list)
        .with_network(network)
        .with_initial_version(MIN_PLATFORM_VERSION)
        .with_context_provider(context_provider)
        .with_settings(request_settings)
        .build()
        .map_err(Box::new)?;

    info!(
        ?network,
        protocol_version = sdk.protocol_version_number(),
        "SDK initialized successfully"
    );

    Ok(sdk)
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

    /// A proof-verifying read sent before the masternode list has synced.
    /// The provider answers "not ready", the proof verifier reports that the
    /// quorum key is unavailable, and the SDK's conversion turns that into a
    /// provider configuration error: DET reports it as the readiness wait
    /// that clears when sync catches up, and the SDK bans no node for a
    /// failure that is local. The error is built through that conversion, so
    /// a change in what the SDK returns shows up here.
    #[test]
    fn a_proof_read_before_masternodes_are_ready_is_a_readiness_wait_and_bans_no_node() {
        let not_ready = || {
            dash_sdk::Error::from(ProofVerifierError::QuorumKeyUnavailable {
                quorum_type: 6,
                quorum_hash: [0x51; 32],
                core_chain_locked_height: 1_000,
                error: ContextProviderError::Config(MASTERNODE_LIST_NOT_READY_DETAIL.to_string()),
            })
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
            possibly_accepted: false,
            inner: not_ready(),
            retries: 0,
            address: Some(address.clone()),
        });
        update_address_ban_status(&addresses, &result, &request_settings().finalize());
        assert!(
            !addresses.is_banned(&address),
            "a local readiness failure must not ban the node that answered"
        );
    }
}
