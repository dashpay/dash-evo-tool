use crate::config::NetworkConfig;
use crate::context::default_platform_version;
use dash_sdk::dpp::block::extended_epoch_info::v0::ExtendedEpochInfoV0Getters;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::v1::PLATFORM_V1;
use dash_sdk::dpp::version::{PlatformVersion, ProtocolVersion};
use dash_sdk::platform::types::epoch::{Epoch, EpochQuery};
use dash_sdk::platform::{ContextProvider, Fetch};
use dash_sdk::{RequestSettings, Sdk, SdkBuilder};
use std::sync::Arc;
use tokio::runtime::Runtime;
// Adjust imports
use std::time::Duration;
use tracing::info;

pub fn initialize_sdk<P: ContextProvider + 'static>(
    config: &NetworkConfig,
    network: Network,
    context_provider: P,
    platform_version: Option<&'static PlatformVersion>,
) -> Sdk {
    // Default platform version if not provided
    let platform_version = platform_version.unwrap_or_else(|| default_platform_version(&network));

    // Setup Platform SDK
    let address_list = config.dapi_address_list();
    let request_settings = RequestSettings {
        connect_timeout: Some(Duration::from_secs(1)),
        timeout: Some(Duration::from_secs(10)),
        retries: Some(6),
        ban_failed_address: Some(true),
    };

    let sdk = SdkBuilder::new(address_list)
        .with_version(platform_version)
        .with_network(network)
        .with_context_provider(context_provider)
        .with_settings(request_settings)
        .build()
        .expect("Failed to build SDK");

    info!(
        ?network,
        protocol_version = sdk.version().protocol_version,
        "SDK initialized successfully"
    );

    sdk
}

async fn update_sdk_protocol_version(sdk: &Sdk) -> Result<&'static PlatformVersion, String> {
    let query = EpochQuery {
        start: None,
        ascending: false,
    };

    let detected_protocol_version =
        match Epoch::fetch(sdk, query).await.map_err(|e| e.to_string())? {
            Some(epoch) => Ok::<u32, String>(epoch.protocol_version()),
            None => {
                return Err("Failed to fetch the latest epoch".to_string());
            }
        }?;

    let max_supported = PlatformVersion::latest().protocol_version;
    let best_version = max_supported.min(detected_protocol_version);

    PlatformVersion::get(best_version).map_err(|e| e.to_string())
}
