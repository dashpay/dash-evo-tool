//! Key schema for the DPNS vote journal's k/v namespaces.
//!
//! Every key the journal owns is built here so the on-disk layout, including
//! the v1 names still read by migration, has one definition.

use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::DpnsVoteOperationId;
use crate::wallet_backend::{KvAdapterError, network_prefix};
use dash_sdk::dpp::dashcore::Network;

pub(super) const LEGACY_OPERATION_INDEX_KEY: &str = "det:dpns_vote_operations:v1";
pub(super) const LEGACY_OPERATION_KEY_PREFIX: &str = "det:dpns_vote_operation:v1:";
pub(super) const OPERATION_INDEX_KEY_PREFIX: &str = "det:dpns_vote_operations:v2:";
pub(super) const OPERATION_KEY_PREFIX: &str = "det:dpns_vote_operation:v2:";
pub(super) const OPERATION_LOCK_INDEX_KEY_PREFIX: &str = "det:dpns_vote_operation_locks:v2:";
pub(super) const OPERATION_LOCK_INDEX_DIRTY_KEY_PREFIX: &str =
    "det:dpns_vote_operation_locks_dirty:v2:";

pub(super) const SCHEDULE_DISMISSAL_KEY_PREFIX: &str = "det:dpns_vote_schedule_dismissals:v1:";
pub(super) const IMMEDIATE_HISTORY_KEY_PREFIX: &str = "det:dpns_vote_immediate_history:v1:";
pub(super) const SCHEDULED_HISTORY_KEY_PREFIX: &str = "det:dpns_vote_scheduled_history:v1:";

pub(super) fn operation_index_key(network: Network) -> String {
    format!("{OPERATION_INDEX_KEY_PREFIX}{}", network_prefix(network))
}

pub(super) fn operation_key(network: Network, id: DpnsVoteOperationId) -> String {
    format!("{OPERATION_KEY_PREFIX}{}:{id}", network_prefix(network))
}

pub(super) fn operation_key_prefix(network: Network) -> String {
    format!("{OPERATION_KEY_PREFIX}{}:", network_prefix(network))
}

pub(super) fn schedule_dismissal_key(network: Network, id: DpnsVoteOperationId) -> String {
    format!(
        "{SCHEDULE_DISMISSAL_KEY_PREFIX}{}:{id}",
        network_prefix(network)
    )
}

pub(super) fn operation_lock_index_key(network: Network) -> String {
    format!(
        "{OPERATION_LOCK_INDEX_KEY_PREFIX}{}",
        network_prefix(network)
    )
}

pub(super) fn operation_lock_index_dirty_key(network: Network) -> String {
    format!(
        "{OPERATION_LOCK_INDEX_DIRTY_KEY_PREFIX}{}",
        network_prefix(network)
    )
}

pub(super) fn legacy_operation_key(id: DpnsVoteOperationId) -> String {
    format!("{LEGACY_OPERATION_KEY_PREFIX}{id}")
}

pub(super) fn operation_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationStorage { source }
}

pub(super) fn unreadable_operation_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationUnreadable { source }
}
