use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::ContextProvider;
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
/// network reports in proven responses. A rebuilt connection starts there
/// too: nothing learned from the endpoints it replaces is carried over.
pub fn initialize_sdk<P: ContextProvider + 'static>(
    address_list: AddressList,
    network: Network,
    context_provider: P,
) -> Result<Sdk, String> {
    let request_settings = request_settings();

    let sdk = SdkBuilder::new(address_list)
        .with_network(network)
        .with_context_provider(context_provider)
        .with_settings(request_settings)
        .build()
        .map_err(|e| format!("Failed to build SDK: {e}"))?;

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
