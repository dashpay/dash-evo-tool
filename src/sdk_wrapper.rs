use crate::config::NetworkConfig;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::ContextProvider;
use dash_sdk::{RequestSettings, Sdk, SdkBuilder}; // Adjust imports
use std::time::Duration;
use tracing::info;

pub fn initialize_sdk<P: ContextProvider + 'static>(
    config: &NetworkConfig,
    network: Network,
    context_provider: P,
) -> Sdk {
    // Setup Platform SDK
    let address_list = config.dapi_address_list();
    let request_settings = RequestSettings {
        connect_timeout: Some(Duration::from_secs(10)),
        timeout: Some(Duration::from_secs(10)),
        retries: Some(6),
        ..Default::default()
    };

    let sdk = SdkBuilder::new(address_list)
        .with_version(PlatformVersion::get(4).unwrap())
        .with_network(network)
        .with_context_provider(context_provider)
        .with_settings(request_settings)
        .build()
        .expect("Failed to build SDK");

    info!("SDK initialized successfully");

    sdk
}
