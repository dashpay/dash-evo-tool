//! Proved, per-node DPNS current-vote snapshots.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::DpnsCurrentVoteState;
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
use dash_sdk::dpp::voting::votes::resource_vote::accessors::v0::ResourceVoteGettersV0;
use dash_sdk::drive::query::contested_resource_votes_given_by_identity_query::ContestedResourceVotesGivenByIdentityQuery;
use dash_sdk::platform::{FetchMany, Identifier};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

const CURRENT_VOTES_KEY: &str = "det:dpns_current_votes:v1";
const VOTE_QUERY_PAGE_SIZE: u16 = 100;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredCurrentVotes {
    available: bool,
    updated_at: u64,
    votes: BTreeMap<[u8; 32], ResourceVoteChoice>,
}

fn vote_state_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationStorage { source }
}

fn load_snapshot(
    kv: &DetKv,
    voter_id: &Identifier,
) -> Result<Option<StoredCurrentVotes>, TaskError> {
    kv.get(DetScope::Identity(&voter_id.to_buffer()), CURRENT_VOTES_KEY)
        .map_err(vote_state_err)
}

fn save_snapshot(
    kv: &DetKv,
    voter_id: &Identifier,
    snapshot: &StoredCurrentVotes,
) -> Result<(), TaskError> {
    kv.put(
        DetScope::Identity(&voter_id.to_buffer()),
        CURRENT_VOTES_KEY,
        snapshot,
    )
    .map_err(vote_state_err)
}

impl AppContext {
    /// Build the exact Platform vote-poll ID for one normalized DPNS label.
    pub fn dpns_vote_poll_id(&self, name: &str) -> Result<Identifier, TaskError> {
        let document_type = self
            .dpns_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err(TaskError::ContractSchemaMismatch {
                detail: "DPNS domain document type has no contested index",
            });
        };
        let normalized_name = convert_to_homograph_safe_chars(name);
        ContestedDocumentResourceVotePoll {
            index_name: contested_index.name.clone(),
            index_values: vec![Value::from("dash"), Value::Text(normalized_name)],
            document_type_name: document_type.name().to_owned(),
            contract_id: self.dpns_contract.id(),
        }
        .unique_id()
        .map_err(|source| TaskError::SdkError {
            source_error: Box::new(dash_sdk::Error::Protocol(source)),
        })
    }

    /// Read the latest proved state without performing network I/O in the frame loop.
    pub fn dpns_current_vote_state(
        &self,
        voter_id: Identifier,
        vote_poll_id: Identifier,
    ) -> Result<DpnsCurrentVoteState, TaskError> {
        Ok(match load_snapshot(&self.det_kv()?, &voter_id)? {
            None => DpnsCurrentVoteState::Checking,
            Some(snapshot) if !snapshot.available => DpnsCurrentVoteState::Unavailable,
            Some(snapshot) => DpnsCurrentVoteState::Available(
                snapshot.votes.get(&vote_poll_id.to_buffer()).copied(),
            ),
        })
    }

    /// Refresh proved vote state once per loaded masternode, paging only as needed.
    pub(crate) async fn refresh_dpns_vote_states(&self, sdk: &Sdk) {
        let voters = match self.load_local_masternode_identities() {
            Ok(voters) => voters,
            Err(error) => {
                tracing::warn!(?error, "Could not load nodes for DPNS vote-state refresh");
                return;
            }
        };
        let kv = match self.det_kv() {
            Ok(kv) => kv,
            Err(error) => {
                tracing::warn!(?error, "Could not open DPNS vote-state storage");
                return;
            }
        };

        stream::iter(voters)
            .map(|voter| {
                let sdk = sdk.clone();
                let kv = kv.clone();
                async move {
                    let voter_id = voter.identity.id();
                    match fetch_votes_for_voter(&sdk, voter_id).await {
                        Ok(votes) => {
                            let snapshot = StoredCurrentVotes {
                                available: true,
                                updated_at: now_ms(),
                                votes,
                            };
                            if let Err(error) = save_snapshot(&kv, &voter_id, &snapshot) {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %voter_id,
                                    "Could not save proved DPNS vote state"
                                );
                            }
                        }
                        Err(error) => {
                            let snapshot = StoredCurrentVotes {
                                available: false,
                                updated_at: now_ms(),
                                votes: BTreeMap::new(),
                            };
                            if let Err(storage_error) = save_snapshot(&kv, &voter_id, &snapshot) {
                                tracing::warn!(
                                    ?storage_error,
                                    voter_id = %voter_id,
                                    "Could not save unavailable DPNS vote state"
                                );
                            }
                            tracing::warn!(
                                ?error,
                                voter_id = %voter_id,
                                "Proved DPNS vote-state query was unavailable"
                            );
                        }
                    }
                }
            })
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;
    }

    /// Update the proved-state cache after a confirmed target.
    pub(crate) fn cache_confirmed_dpns_vote(
        &self,
        voter_id: Identifier,
        vote_poll_id: Identifier,
        choice: ResourceVoteChoice,
    ) -> Result<(), TaskError> {
        let kv = self.det_kv()?;
        let mut snapshot = load_snapshot(&kv, &voter_id)?.unwrap_or_default();
        snapshot.available = true;
        snapshot.updated_at = now_ms();
        snapshot.votes.insert(vote_poll_id.to_buffer(), choice);
        save_snapshot(&kv, &voter_id, &snapshot)
    }
}

async fn fetch_votes_for_voter(
    sdk: &Sdk,
    voter_id: Identifier,
) -> Result<BTreeMap<[u8; 32], ResourceVoteChoice>, dash_sdk::Error> {
    let mut votes = BTreeMap::new();
    let mut start_at = None;
    loop {
        let page = ResourceVote::fetch_many(
            sdk,
            ContestedResourceVotesGivenByIdentityQuery {
                identity_id: voter_id,
                offset: None,
                limit: Some(VOTE_QUERY_PAGE_SIZE),
                start_at,
                order_ascending: true,
            },
        )
        .await?;
        let page_len = page.len();
        let last_key = page.last().map(|(id, _)| id.to_buffer());
        for (poll_id, vote) in page {
            if let Some(vote) = vote {
                votes.insert(poll_id.to_buffer(), vote.resource_vote_choice());
            }
        }
        if page_len < usize::from(VOTE_QUERY_PAGE_SIZE) {
            break;
        }
        let Some(last_key) = last_key else {
            break;
        };
        start_at = Some((last_key, false));
    }
    Ok(votes)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use std::sync::Arc;

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    /// VOTE-TC-001: a proved current choice round-trips by node and poll.
    #[test]
    fn proved_current_vote_round_trips() {
        let kv = kv();
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        let snapshot = StoredCurrentVotes {
            available: true,
            updated_at: 3,
            votes: BTreeMap::from([(poll.to_buffer(), ResourceVoteChoice::Lock)]),
        };
        save_snapshot(&kv, &voter, &snapshot).unwrap();

        assert_eq!(load_snapshot(&kv, &voter).unwrap(), Some(snapshot));
    }

    /// VOTE-TC-007: query failure is represented explicitly, never as no vote.
    #[test]
    fn unavailable_snapshot_is_distinct_from_an_empty_proved_snapshot() {
        let unavailable = StoredCurrentVotes {
            available: false,
            ..Default::default()
        };
        let empty = StoredCurrentVotes {
            available: true,
            ..Default::default()
        };

        assert_ne!(unavailable, empty);
    }
}
