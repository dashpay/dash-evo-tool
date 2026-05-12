use crate::context::default_platform_version;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::ContextProvider;
use dash_sdk::{RequestSettings, Sdk, SdkBuilder};
use std::time::Duration;
use tracing::info;

pub fn initialize_sdk<P: ContextProvider + 'static>(
    address_list: AddressList,
    network: Network,
    context_provider: P,
) -> Result<Sdk, String> {
    let request_settings = RequestSettings {
        connect_timeout: Some(Duration::from_secs(1)),
        timeout: Some(Duration::from_secs(10)),
        retries: Some(6),
        ban_failed_address: Some(true),
        max_decoding_message_size: None,
    };
    let platform_version = default_platform_version(&network);

    let sdk = SdkBuilder::new(address_list)
        .with_version(platform_version)
        .with_network(network)
        .with_context_provider(context_provider)
        .with_settings(request_settings)
        .build()
        .map_err(|e| format!("Failed to build SDK: {e}"))?;

    info!(
        ?network,
        protocol_version = platform_version.protocol_version,
        "SDK initialized successfully"
    );

    Ok(sdk)
}
