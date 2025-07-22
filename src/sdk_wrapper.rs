use crate::config::NetworkConfig;
use crate::context::default_platform_version;
use dash_sdk::dpp::dashcore::Network;
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
    tracing::info!("Initializing SDK with DAPI addresses: {:?}", address_list);
    
    let request_settings = RequestSettings {
        connect_timeout: Some(Duration::from_secs(1)),
        timeout: Some(Duration::from_secs(10)),
        retries: Some(6),
        ban_failed_address: Some(true),
    };
    tracing::info!("SDK request settings: {:?}", request_settings);
    
    let platform_version = default_platform_version(&network);

    let sdk = SdkBuilder::new(address_list)
        .with_version(platform_version)
        .with_network(network)
        .with_context_provider(context_provider)
        .with_settings(request_settings)
        .build()
        .expect("Failed to build SDK");

    info!(
        ?network,
        protocol_version = platform_version.protocol_version,
        "SDK initialized successfully"
    );

    sdk
}
