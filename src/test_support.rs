//! Shared support for tests across the library crate.
//!
//! Compiled only for unit tests and the `testing` / `bench` features, never
//! into release builds. [`keys`] is public so integration tests and benches
//! can reach it; everything else is unit-test only.

pub mod keys;

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
use dash_sdk::dpp::consensus::ConsensusError;
#[cfg(test)]
use dash_sdk::dpp::consensus::state::document::duplicate_unique_index_error::DuplicateUniqueIndexError;
#[cfg(test)]
use dash_sdk::platform::Identifier;

/// Serializes every unit test that mutates the process-global data directory.
#[cfg(test)]
pub(crate) static DASH_EVO_DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
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
