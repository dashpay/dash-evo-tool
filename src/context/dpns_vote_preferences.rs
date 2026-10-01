//! Per-network operator preferences for masternode voting: the last-used
//! Masternodes segment, the saved node set, and the display label of
//! relative ("before the end") schedules.
//!
//! Stored in the app-level k/v store under network-prefixed keys, so they are
//! readable before the wallet backend is wired and never cross networks.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::DpnsVoteTargetKey;
use crate::model::dpns_voting::operator::{MasternodesSegment, NodeSet};
use crate::wallet_backend::DetScope;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

fn segment_key(network: Network) -> String {
    format!("{network}:dpns_voting:masternodes_segment")
}

fn node_set_key(network: Network) -> String {
    format!("{network}:dpns_voting:node_set")
}

/// Keyed by the absolute time too: an edit to another time orphans the label,
/// and the row falls back to the absolute time alone. The target is hashed to
/// fit the store's key-length limit.
fn relative_label_key(key: &DpnsVoteTargetKey, scheduled_at: TimestampMillis) -> String {
    use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};
    let mut preimage = Vec::with_capacity(72);
    preimage.extend_from_slice(key.voter_id.as_bytes());
    preimage.extend_from_slice(key.vote_poll_id.as_bytes());
    preimage.extend_from_slice(&scheduled_at.to_be_bytes());
    format!(
        "{network}:dpns_voting:relative:{digest}",
        network = key.network,
        digest = sha256::Hash::hash(&preimage),
    )
}

impl AppContext {
    fn voting_preference<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, TaskError> {
        self.app_kv().get(DetScope::Global, key).map_err(|source| {
            TaskError::VotingPreferenceStorage {
                source: Box::new(source),
            }
        })
    }

    fn put_voting_preference<T: Serialize>(&self, key: &str, value: &T) -> Result<(), TaskError> {
        self.app_kv()
            .put(DetScope::Global, key, value)
            .map_err(|source| TaskError::VotingPreferenceStorage {
                source: Box::new(source),
            })
    }

    /// The Masternodes segment last opened on this network, if any.
    pub fn masternodes_last_segment(&self) -> Result<Option<MasternodesSegment>, TaskError> {
        self.voting_preference(&segment_key(self.network()))
    }

    /// Remember the Masternodes segment just opened on this network.
    pub fn set_masternodes_last_segment(
        &self,
        segment: MasternodesSegment,
    ) -> Result<(), TaskError> {
        self.put_voting_preference(&segment_key(self.network()), &segment)
    }

    /// The node set saved as default on this network (`All` when none is saved).
    pub fn saved_dpns_node_set(&self) -> Result<NodeSet, TaskError> {
        Ok(self
            .voting_preference(&node_set_key(self.network()))?
            .unwrap_or_default())
    }

    /// Save `node_set` as the default on this network.
    pub fn save_dpns_node_set(&self, node_set: &NodeSet) -> Result<(), TaskError> {
        self.put_voting_preference(&node_set_key(self.network()), node_set)
    }

    /// Remember that the target scheduled at `scheduled_at` was picked as
    /// `preset` before its contest's end (VOTE-FR-081). Display only: the
    /// absolute time in the journal stays authoritative.
    pub fn save_dpns_relative_schedule_label(
        &self,
        key: &DpnsVoteTargetKey,
        scheduled_at: TimestampMillis,
        preset: Duration,
    ) -> Result<(), TaskError> {
        let preset_ms = u64::try_from(preset.as_millis()).unwrap_or(u64::MAX);
        self.put_voting_preference(&relative_label_key(key, scheduled_at), &preset_ms)
    }

    /// The relative preset a scheduled target was picked with, if recorded.
    pub fn dpns_relative_schedule_label(
        &self,
        key: &DpnsVoteTargetKey,
        scheduled_at: TimestampMillis,
    ) -> Option<Duration> {
        self.voting_preference::<u64>(&relative_label_key(key, scheduled_at))
            .inspect_err(|error| tracing::debug!(?error, "Relative schedule label unreadable"))
            .ok()
            .flatten()
            .map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeSet;

    /// VOTE-TC-067 (label half): the label is bound to the exact time; a
    /// different time reads no label, so the row shows the absolute time only.
    #[test]
    fn relative_label_is_bound_to_its_absolute_time() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let key = DpnsVoteTargetKey {
            network: context.network(),
            voter_id: Identifier::from([1; 32]),
            vote_poll_id: Identifier::from([2; 32]),
        };
        let six_hours = Duration::from_secs(6 * 3600);
        context
            .save_dpns_relative_schedule_label(&key, 1_000, six_hours)
            .expect("save label");
        assert_eq!(
            context.dpns_relative_schedule_label(&key, 1_000),
            Some(six_hours)
        );
        assert_eq!(context.dpns_relative_schedule_label(&key, 2_000), None);
    }

    #[test]
    fn keys_are_network_scoped() {
        assert_ne!(segment_key(Network::Mainnet), segment_key(Network::Testnet));
        assert_ne!(
            node_set_key(Network::Mainnet),
            node_set_key(Network::Testnet)
        );
        assert_ne!(
            segment_key(Network::Testnet),
            node_set_key(Network::Testnet)
        );
    }

    /// VOTE-TC-090 (store half): a saved node set reads back on its network only.
    #[test]
    fn node_set_and_segment_persist_per_network() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        assert_eq!(context.saved_dpns_node_set().expect("read"), NodeSet::All);
        assert_eq!(context.masternodes_last_segment().expect("read"), None);

        let custom = NodeSet::Custom(BTreeSet::from([Identifier::from([3; 32])]));
        context.save_dpns_node_set(&custom).expect("save");
        context
            .set_masternodes_last_segment(MasternodesSegment::Votes)
            .expect("save segment");

        assert_eq!(context.saved_dpns_node_set().expect("read"), custom);
        assert_eq!(
            context.masternodes_last_segment().expect("read"),
            Some(MasternodesSegment::Votes)
        );
        let other = context
            .app_kv()
            .get::<NodeSet>(DetScope::Global, &node_set_key(Network::Mainnet))
            .expect("read other network");
        assert_eq!(other, None, "another network sees no saved set");
    }
}
