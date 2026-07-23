//! Shared support for unit tests across the library crate.

use std::sync::Mutex;

use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::state::document::duplicate_unique_index_error::DuplicateUniqueIndexError;
use dash_sdk::platform::Identifier;

/// Serializes every unit test that mutates the process-global data directory.
pub(crate) static DASH_EVO_DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn duplicate_unique_index_broadcast_error(properties: Vec<&str>) -> dash_sdk::Error {
    let consensus = ConsensusError::from(DuplicateUniqueIndexError::new(
        Identifier::random(),
        properties.into_iter().map(str::to_string).collect(),
    ));

    dash_sdk::Error::StateTransitionBroadcastError(dash_sdk::error::StateTransitionBroadcastError {
        code: 40105,
        message: "duplicate unique index".to_string(),
        cause: Some(consensus),
    })
}
