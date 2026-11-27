//! Why the app could not set up its connection to one network.

use dash_sdk::dapi_client::AddressListError;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::dashcore::Network;

use crate::backend_task::error::TaskError;
use crate::config::{ConfigError, NetworkSettingError};

/// A failure of [`AppContext::new`](crate::context::AppContext::new).
#[derive(Debug, thiserror::Error)]
pub enum NetworkContextError {
    /// The `.env` settings file could not be read.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// The settings file has no entries for this network.
    #[error(
        "The settings file has no entries for {network}. Add them to the .env file in the app data folder, then restart the application."
    )]
    NotConfigured { network: Network },

    /// A saved setting for this network has a value that cannot be used.
    #[error(
        "A setting saved for {network} has a value that is not valid. Correct it in the .env file in the app data folder, then restart the application."
    )]
    InvalidSettings {
        network: Network,
        #[source]
        source: NetworkSettingError,
    },

    /// No node addresses are saved for this network.
    #[error(
        "No node addresses are set up for {network}. Add them to the .env file in the app data folder, then restart the application."
    )]
    NoDapiAddresses { network: Network },

    /// The saved node addresses could not be read.
    #[error(
        "The node addresses saved for {network} are not valid. Correct them in the .env file in the app data folder, then restart the application."
    )]
    InvalidDapiAddresses {
        network: Network,
        #[source]
        source: AddressListError,
    },

    /// The Platform client could not be built.
    #[error("Could not set up the connection to {network}. Restart the application to try again.")]
    Sdk {
        network: Network,
        #[source]
        source: Box<dash_sdk::Error>,
    },

    /// The built-in Platform contracts could not be loaded.
    #[error(
        "Built-in Dash Platform data could not be loaded. Install the application again, then restart it."
    )]
    SystemContracts {
        #[source]
        source: Box<ProtocolError>,
    },

    /// The Dash Core connection could not be configured.
    #[error(
        "The Dash Core connection settings for {network} could not be used. Check them in the .env file in the app data folder, then restart the application."
    )]
    CoreClient {
        network: Network,
        #[source]
        source: Box<TaskError>,
    },

    /// The context provider lock was poisoned by an earlier panic.
    #[error("Could not finish setting up {network}. Restart the application to try again.")]
    ContextProviderUnavailable { network: Network },
}
