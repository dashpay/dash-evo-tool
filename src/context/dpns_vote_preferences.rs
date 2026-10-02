//! Per-network operator preferences for masternode voting: the last-used
//! Masternodes segment and the saved node set.
//!
//! Stored in the app-level k/v store under network-prefixed keys, so they are
//! readable before the wallet backend is wired and never cross networks.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::operator::{MasternodesSegment, NodeSet};
use crate::wallet_backend::DetScope;
use dash_sdk::dpp::dashcore::Network;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn segment_key(network: Network) -> String {
    format!("{network}:dpns_voting:masternodes_segment")
}

fn node_set_key(network: Network) -> String {
    format!("{network}:dpns_voting:node_set")
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeSet;

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
