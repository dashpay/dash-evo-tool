//! Proved, per-node DPNS current-vote snapshots.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::DpnsCurrentVoteState;
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Network;
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
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const LEGACY_CURRENT_VOTES_KEY: &str = "det:dpns_current_votes:v1";
const CURRENT_VOTES_KEY_PREFIX: &str = "det:dpns_current_votes:v3:";
const VOTE_QUERY_PAGE_SIZE: u16 = 100;
const CURRENT_VOTE_MAX_AGE_MS: u64 = 120_000;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredCurrentVotes {
    available: bool,
    updated_at: u64,
    votes: BTreeMap<[u8; 32], ResourceVoteChoice>,
    confirmed: BTreeMap<[u8; 32], (u64, ResourceVoteChoice)>,
}

#[derive(Debug, Default)]
pub(super) struct DpnsVoteStatePublications {
    sequence: AtomicU64,
    voters: Mutex<BTreeMap<Identifier, u64>>,
}

impl DpnsVoteStatePublications {
    fn next_generation(&self) -> Result<u64, TaskError> {
        self.sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| TaskError::DpnsVoteStateChanged)
    }
}

pub(crate) type DpnsVoteRefreshResults =
    BTreeMap<Identifier, Result<RefreshedDpnsVotes, Arc<TaskError>>>;

#[derive(Debug)]
pub(crate) struct RefreshedDpnsVotes {
    voter_id: Identifier,
    generation: u64,
    snapshot: StoredCurrentVotes,
}

impl RefreshedDpnsVotes {
    /// Consume this proof while excluding newer publications through journal revalidation.
    pub(crate) fn with_state<T>(
        &self,
        app_context: &AppContext,
        poll_id: Identifier,
        consume: impl FnOnce(DpnsCurrentVoteState) -> T,
    ) -> Result<T, TaskError> {
        // Lock order: vote-state publications, then the operation journal.
        let publications = app_context
            .dpns_vote_state_publications
            .voters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if publications.get(&self.voter_id) != Some(&self.generation) {
            return Err(TaskError::DpnsVoteStateChanged);
        }
        Ok(consume(snapshot_vote_state(
            Some(&self.snapshot),
            poll_id,
            now_ms(),
        )))
    }

    #[cfg(test)]
    pub(crate) fn state(
        &self,
        app_context: &AppContext,
        poll_id: Identifier,
    ) -> Result<DpnsCurrentVoteState, TaskError> {
        self.with_state(app_context, poll_id, std::convert::identity)
    }
}

fn vote_state_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationStorage { source }
}

fn current_votes_key(network: Network) -> String {
    let network = match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    };
    format!("{CURRENT_VOTES_KEY_PREFIX}{network}")
}

fn load_snapshot(
    kv: &DetKv,
    network: Network,
    voter_id: &Identifier,
) -> Result<Option<StoredCurrentVotes>, TaskError> {
    let scope = DetScope::Identity(&voter_id.to_buffer());
    if let Some(snapshot) = kv
        .get(scope, &current_votes_key(network))
        .map_err(vote_state_err)?
    {
        return Ok(Some(snapshot));
    }
    // Cached v1/v2 values have no independent per-poll freshness and must be re-proved.
    kv.delete(scope, LEGACY_CURRENT_VOTES_KEY)
        .map_err(vote_state_err)?;
    let previous_key = current_votes_key(network).replace(":v3:", ":v2:");
    kv.delete(scope, &previous_key).map_err(vote_state_err)?;
    Ok(None)
}

fn save_snapshot(
    kv: &DetKv,
    network: Network,
    voter_id: &Identifier,
    snapshot: &StoredCurrentVotes,
) -> Result<(), TaskError> {
    kv.put(
        DetScope::Identity(&voter_id.to_buffer()),
        &current_votes_key(network),
        snapshot,
    )
    .map_err(vote_state_err)
}

fn snapshot_vote_state(
    snapshot: Option<&StoredCurrentVotes>,
    vote_poll_id: Identifier,
    checked_at_ms: u64,
) -> DpnsCurrentVoteState {
    if let Some((updated_at, choice)) =
        snapshot.and_then(|snapshot| snapshot.confirmed.get(&vote_poll_id.to_buffer()))
        && checked_at_ms.saturating_sub(*updated_at) <= CURRENT_VOTE_MAX_AGE_MS
    {
        return DpnsCurrentVoteState::Available(Some(*choice));
    }
    match snapshot {
        None => DpnsCurrentVoteState::Checking,
        Some(snapshot)
            if checked_at_ms.saturating_sub(snapshot.updated_at) > CURRENT_VOTE_MAX_AGE_MS =>
        {
            DpnsCurrentVoteState::Checking
        }
        Some(snapshot) if !snapshot.available => DpnsCurrentVoteState::Unavailable,
        Some(snapshot) => {
            DpnsCurrentVoteState::Available(snapshot.votes.get(&vote_poll_id.to_buffer()).copied())
        }
    }
}

impl AppContext {
    #[cfg(test)]
    pub(crate) async fn seed_proved_dpns_votes_for_test(
        &self,
        voter_id: Identifier,
        votes: BTreeMap<[u8; 32], ResourceVoteChoice>,
    ) -> Result<(), TaskError> {
        self.refresh_dpns_vote_state_with(&self.det_kv()?, voter_id, async { Ok(votes) })
            .await
            .map(|_| ())
    }

    async fn refresh_dpns_vote_state_with(
        &self,
        kv: &DetKv,
        voter_id: Identifier,
        fetch: impl Future<
            Output = Result<BTreeMap<[u8; 32], ResourceVoteChoice>, Box<dash_sdk::Error>>,
        >,
    ) -> Result<RefreshedDpnsVotes, TaskError> {
        let generation = self.dpns_vote_state_publications.next_generation()?;
        let checked_at = now_ms();
        let result = fetch.await;
        let mut publications = self
            .dpns_vote_state_publications
            .voters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if publications
            .get(&voter_id)
            .is_some_and(|published| *published > generation)
        {
            return Err(TaskError::DpnsVoteStateChanged);
        }
        let confirmed = if result.is_err() {
            load_snapshot(kv, self.network, &voter_id)?
                .unwrap_or_default()
                .confirmed
        } else {
            BTreeMap::new()
        };
        let snapshot = StoredCurrentVotes {
            available: result.is_ok(),
            updated_at: checked_at,
            votes: result.as_ref().cloned().unwrap_or_default(),
            confirmed,
        };
        publications.insert(voter_id, generation);
        save_snapshot(kv, self.network, &voter_id, &snapshot)?;
        if let Err(error) = result {
            tracing::warn!(?error, %voter_id, "Proved DPNS vote-state query was unavailable");
            return Err(TaskError::from(*error));
        }
        Ok(RefreshedDpnsVotes {
            voter_id,
            generation,
            snapshot,
        })
    }

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
        let snapshot = load_snapshot(&self.det_kv()?, self.network, &voter_id)?;
        Ok(snapshot_vote_state(
            snapshot.as_ref(),
            vote_poll_id,
            now_ms(),
        ))
    }

    /// Read one node's current choices for many polls with a single storage read.
    pub fn dpns_current_vote_states(
        &self,
        voter_id: Identifier,
        vote_poll_ids: impl IntoIterator<Item = Identifier>,
    ) -> Result<BTreeMap<Identifier, DpnsCurrentVoteState>, TaskError> {
        let snapshot = load_snapshot(&self.det_kv()?, self.network, &voter_id)?;
        let checked_at_ms = now_ms();
        Ok(vote_poll_ids
            .into_iter()
            .map(|vote_poll_id| {
                (
                    vote_poll_id,
                    snapshot_vote_state(snapshot.as_ref(), vote_poll_id, checked_at_ms),
                )
            })
            .collect())
    }

    /// Refresh proved vote state once per loaded masternode, paging only as needed.
    pub(crate) async fn refresh_dpns_vote_states(
        &self,
        sdk: &Sdk,
    ) -> Result<DpnsVoteRefreshResults, TaskError> {
        let voters = self.load_local_masternode_identities()?;
        let kv = self.det_kv()?;

        Ok(stream::iter(voters)
            .map(|voter| {
                let sdk = sdk.clone();
                let kv = kv.clone();
                async move {
                    let voter_id = voter.identity.id();
                    let result = self.refresh_dpns_vote_state_with(
                        &kv, voter_id, fetch_votes_for_voter(&sdk, voter_id),
                    ).await.map_err(Arc::new);
                    if let Err(error) = &result {
                        tracing::warn!(?error, %voter_id, "Could not refresh proved DPNS vote state");
                    }
                    (voter_id, result)
                }
            })
            .buffer_unordered(4)
            .collect::<BTreeMap<_, _>>()
            .await)
    }

    /// Update the proved-state cache after a confirmed target.
    pub(crate) fn cache_confirmed_dpns_vote(
        &self,
        voter_id: Identifier,
        vote_poll_id: Identifier,
        choice: ResourceVoteChoice,
    ) -> Result<(), TaskError> {
        let mut publications = self
            .dpns_vote_state_publications
            .voters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        publications.insert(
            voter_id,
            self.dpns_vote_state_publications.next_generation()?,
        );
        let kv = self.det_kv()?;
        let mut snapshot = load_snapshot(&kv, self.network, &voter_id)?.unwrap_or_default();
        let confirmed_at = now_ms();
        snapshot.confirmed.retain(|_, (updated_at, _)| {
            confirmed_at.saturating_sub(*updated_at) <= CURRENT_VOTE_MAX_AGE_MS
        });
        snapshot
            .confirmed
            .insert(vote_poll_id.to_buffer(), (confirmed_at, choice));
        save_snapshot(&kv, self.network, &voter_id, &snapshot)
    }
}

async fn fetch_votes_for_voter(
    sdk: &Sdk,
    voter_id: Identifier,
) -> Result<BTreeMap<[u8; 32], ResourceVoteChoice>, Box<dash_sdk::Error>> {
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
    use platform_wallet_storage::{KvError, KvStore, ObjectId};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingKv {
        inner: InMemoryKv,
        gets: AtomicUsize,
    }

    impl KvStore for CountingKv {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            self.gets.fetch_add(1, Ordering::Relaxed);
            self.inner.get(scope, key)
        }

        fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
            self.inner.put(scope, key, value)
        }

        fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
            self.inner.delete(scope, key)
        }

        fn list_keys(
            &self,
            scope: &ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            self.inner.list_keys(scope, prefix)
        }
    }

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    fn connection_error() -> Box<dash_sdk::Error> {
        use dash_sdk::dapi_client::{DapiClientError, transport::TransportError};
        Box::new(dash_sdk::Error::DapiClientError(
            DapiClientError::Transport(TransportError::Grpc(
                dash_sdk::dapi_grpc::tonic::Status::unavailable("tcp connect error"),
            )),
        ))
    }

    #[tokio::test]
    async fn an_older_refresh_cannot_replace_a_newer_publication() {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        let kv = kv();
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let older = context.refresh_dpns_vote_state_with(&kv, voter, async {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            Ok(BTreeMap::from([(
                poll.to_buffer(),
                ResourceVoteChoice::Abstain,
            )]))
        });
        let newer = async {
            started_rx.await.unwrap();
            let result = context
                .refresh_dpns_vote_state_with(&kv, voter, async {
                    Ok(BTreeMap::from([(
                        poll.to_buffer(),
                        ResourceVoteChoice::Lock,
                    )]))
                })
                .await;
            finish_tx.send(()).unwrap();
            result
        };
        let (older, newer) = tokio::join!(older, newer);
        newer.unwrap();
        assert!(
            older.is_err(),
            "a superseded result cannot authorize preflight"
        );
        assert_eq!(
            context.dpns_current_vote_state(voter, poll).unwrap(),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
    }

    #[tokio::test]
    async fn a_delayed_refresh_failure_cannot_erase_a_confirmation() {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        let kv = kv();
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let refresh = context.refresh_dpns_vote_state_with(&kv, voter, async {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            Err(connection_error())
        });
        let confirmation = async {
            started_rx.await.unwrap();
            context
                .cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Lock)
                .unwrap();
            finish_tx.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(refresh, confirmation);
        assert!(result.is_err());
        assert_eq!(
            context.dpns_current_vote_state(voter, poll).unwrap(),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
    }

    #[tokio::test]
    async fn a_returned_snapshot_cannot_authorize_preflight_after_confirmation() {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        let kv = kv();
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        let result = context
            .refresh_dpns_vote_state_with(&kv, voter, async { Ok(BTreeMap::new()) })
            .await
            .unwrap();
        context
            .cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Lock)
            .unwrap();
        assert!(result.state(&context, poll).is_err());

        use crate::model::dpns_voting::{
            DpnsVoteOperation, DpnsVoteTarget, DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
        };
        let key = DpnsVoteTargetKey {
            network: Network::Testnet,
            voter_id: voter,
            vote_poll_id: poll,
        };
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: key.clone(),
            voter_alias: None,
            contested_name: "example".to_owned(),
            current_choice: None,
            requested_choice: ResourceVoteChoice::Abstain,
            timing: VoteTiming::Now,
        }]);
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        assert!(
            result
                .with_state(&context, poll, |state| {
                    context.revalidate_queued_dpns_vote_target(operation.id, &key, state)
                })
                .is_err()
        );
        assert_eq!(
            context.dpns_vote_target_status(&key).unwrap(),
            Some(DpnsVoteTargetStatus::Queued)
        );
    }

    #[tokio::test]
    async fn a_new_failed_refresh_preserves_unexpired_confirmed_polls() {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        let kv = kv();
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        context
            .cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Lock)
            .unwrap();
        assert!(
            context
                .refresh_dpns_vote_state_with(&kv, voter, async { Err(connection_error()) })
                .await
                .is_err()
        );
        assert_eq!(
            context.dpns_current_vote_state(voter, poll).unwrap(),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
        assert_eq!(
            context
                .dpns_current_vote_state(voter, Identifier::from([3; 32]))
                .unwrap(),
            DpnsCurrentVoteState::Unavailable
        );
    }

    #[test]
    fn concurrent_confirmations_retain_every_poll() {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        let kv = kv();
        context.set_det_kv_override_for_test(kv);
        let voter = Identifier::from([1; 32]);
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for poll_byte in 2..10 {
                let context = &context;
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    context
                        .cache_confirmed_dpns_vote(
                            voter,
                            Identifier::from([poll_byte; 32]),
                            ResourceVoteChoice::Lock,
                        )
                        .unwrap();
                });
            }
        });
        for poll_byte in 2..10 {
            assert_eq!(
                context
                    .dpns_current_vote_state(voter, Identifier::from([poll_byte; 32]))
                    .unwrap(),
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
            );
        }
    }

    #[test]
    fn per_poll_confirmation_expires_without_authorizing_other_polls() {
        let poll = Identifier::from([2; 32]);
        let other = Identifier::from([3; 32]);
        let snapshot = StoredCurrentVotes {
            confirmed: BTreeMap::from([(poll.to_buffer(), (10, ResourceVoteChoice::Lock))]),
            ..Default::default()
        };
        assert_eq!(
            snapshot_vote_state(Some(&snapshot), poll, 11),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
        assert_eq!(
            snapshot_vote_state(Some(&snapshot), other, 11),
            DpnsCurrentVoteState::Unavailable
        );
        assert_eq!(
            snapshot_vote_state(Some(&snapshot), poll, 11 + CURRENT_VOTE_MAX_AGE_MS),
            DpnsCurrentVoteState::Checking
        );
    }

    #[test]
    fn v2_snapshot_is_invalidated_without_decoding_a_new_wire_shape() {
        let kv = kv();
        let voter = Identifier::from([1; 32]);
        let key = "det:dpns_current_votes:v2:testnet";
        let old = (
            true,
            now_ms(),
            BTreeMap::<[u8; 32], ResourceVoteChoice>::new(),
        );
        kv.put(DetScope::Identity(&voter.to_buffer()), key, &old)
            .unwrap();
        assert_eq!(load_snapshot(&kv, Network::Testnet, &voter).unwrap(), None);
        assert!(
            kv.get::<(bool, u64, BTreeMap<[u8; 32], ResourceVoteChoice>)>(
                DetScope::Identity(&voter.to_buffer()),
                key
            )
            .unwrap()
            .is_none()
        );
    }

    #[tokio::test]
    async fn failed_refresh_preserves_its_typed_network_cause() {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        let kv = kv();
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let error = context
            .refresh_dpns_vote_state_with(&kv, voter, async { Err(connection_error()) })
            .await
            .unwrap_err();
        assert!(error.contains_dapi_reachability_failure());
        assert_eq!(
            context
                .dpns_current_vote_state(voter, Identifier::from([2; 32]))
                .unwrap(),
            DpnsCurrentVoteState::Unavailable
        );
    }

    #[test]
    fn confirming_one_poll_preserves_other_polls_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let kv = kv();
        let context = crate::context::test_support::test_app_context(temp.path());
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let confirmed = Identifier::from([2; 32]);
        let other = Identifier::from([3; 32]);
        save_snapshot(
            &kv,
            Network::Testnet,
            &voter,
            &StoredCurrentVotes {
                available: false,
                updated_at: now_ms(),
                ..Default::default()
            },
        )
        .unwrap();

        context
            .cache_confirmed_dpns_vote(voter, confirmed, ResourceVoteChoice::Lock)
            .unwrap();

        assert_eq!(
            context.dpns_current_vote_state(voter, confirmed).unwrap(),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
        assert_eq!(
            context.dpns_current_vote_state(voter, other).unwrap(),
            DpnsCurrentVoteState::Unavailable
        );
    }

    #[test]
    fn confirming_one_poll_does_not_renew_an_expired_full_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let kv = kv();
        let context = crate::context::test_support::test_app_context(temp.path());
        context.set_det_kv_override_for_test(kv.clone());
        let voter = Identifier::from([1; 32]);
        let confirmed = Identifier::from([2; 32]);
        let other = Identifier::from([3; 32]);
        save_snapshot(
            &kv,
            Network::Testnet,
            &voter,
            &StoredCurrentVotes {
                available: true,
                updated_at: 1,
                votes: BTreeMap::from([(other.to_buffer(), ResourceVoteChoice::Abstain)]),
                ..Default::default()
            },
        )
        .unwrap();

        context
            .cache_confirmed_dpns_vote(voter, confirmed, ResourceVoteChoice::Lock)
            .unwrap();

        assert_eq!(
            context.dpns_current_vote_state(voter, confirmed).unwrap(),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
        assert_eq!(
            context.dpns_current_vote_state(voter, other).unwrap(),
            DpnsCurrentVoteState::Checking
        );
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
            ..Default::default()
        };
        save_snapshot(&kv, Network::Testnet, &voter, &snapshot).unwrap();

        assert_eq!(
            load_snapshot(&kv, Network::Testnet, &voter).unwrap(),
            Some(snapshot)
        );
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

    #[test]
    fn current_vote_snapshots_are_network_qualified() {
        let kv = kv();
        let voter = Identifier::from([1; 32]);
        let snapshot = StoredCurrentVotes {
            available: true,
            updated_at: now_ms(),
            votes: BTreeMap::new(),
            ..Default::default()
        };
        save_snapshot(&kv, Network::Testnet, &voter, &snapshot).unwrap();

        assert_eq!(
            load_snapshot(&kv, Network::Testnet, &voter).unwrap(),
            Some(snapshot)
        );
        assert_eq!(load_snapshot(&kv, Network::Mainnet, &voter).unwrap(), None);
    }

    #[test]
    fn legacy_snapshot_is_discarded_instead_of_assigned_to_a_network() {
        let kv = kv();
        let voter = Identifier::from([1; 32]);
        let snapshot = StoredCurrentVotes {
            available: true,
            updated_at: now_ms(),
            votes: BTreeMap::new(),
            ..Default::default()
        };
        kv.put(
            DetScope::Identity(&voter.to_buffer()),
            LEGACY_CURRENT_VOTES_KEY,
            &snapshot,
        )
        .unwrap();

        assert_eq!(load_snapshot(&kv, Network::Testnet, &voter).unwrap(), None);
        assert_eq!(load_snapshot(&kv, Network::Mainnet, &voter).unwrap(), None);
        assert!(
            kv.get::<StoredCurrentVotes>(
                DetScope::Identity(&voter.to_buffer()),
                &current_votes_key(Network::Testnet),
            )
            .unwrap()
            .is_none()
        );
        assert!(
            kv.get::<StoredCurrentVotes>(
                DetScope::Identity(&voter.to_buffer()),
                LEGACY_CURRENT_VOTES_KEY,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn stale_proved_snapshot_cannot_authorize_submission() {
        let poll = Identifier::from([2; 32]);
        let snapshot = StoredCurrentVotes {
            available: true,
            updated_at: 1,
            votes: BTreeMap::from([(poll.to_buffer(), ResourceVoteChoice::Lock)]),
            ..Default::default()
        };

        assert_eq!(
            snapshot_vote_state(Some(&snapshot), poll, CURRENT_VOTE_MAX_AGE_MS + 2),
            DpnsCurrentVoteState::Checking
        );
    }

    #[test]
    fn many_poll_states_use_one_storage_read_per_node() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(CountingKv::default());
        let kv = DetKv::from_store(store.clone());
        let voter = Identifier::from([1; 32]);
        let polls = (0..100)
            .map(|byte| Identifier::from([byte; 32]))
            .collect::<Vec<_>>();
        save_snapshot(
            &kv,
            Network::Testnet,
            &voter,
            &StoredCurrentVotes {
                available: true,
                updated_at: now_ms(),
                votes: BTreeMap::new(),
                ..Default::default()
            },
        )
        .expect("seed current-vote snapshot");
        let context = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv);
        let before = store.gets.load(Ordering::Relaxed);

        let states = context
            .dpns_current_vote_states(voter, polls.iter().copied())
            .expect("load current-vote states");

        assert_eq!(states.len(), polls.len());
        assert_eq!(store.gets.load(Ordering::Relaxed) - before, 1);
    }
}
