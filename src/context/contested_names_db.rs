//! DPNS contest cache, persisted in the per-network wallet k/v store.
//!
//! One [`StoredContestedName`] entry per normalized name, keyed at
//! `det:contested_name:<normalized_name>` in the global scope (contests
//! are network-scoped, not per-wallet). Contender rows are nested inside
//! the record so a single k/v read returns the whole contest — there is
//! no relational join.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::contested_name::{
    ContestState, Contestant, ContestedName, MasternodeVoteStateSummary,
};
use crate::model::dpns_usernames::{
    RequestPhase, SeenMarkAttempts, UsernameRequest, won_username_is_missing,
};
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTargetKey, VoteTiming,
    authoritative_dpns_vote_outcomes,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::utils::time::now_ms;
use crate::wallet_backend::{DetScope, KvAdapterError};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight};
use dash_sdk::dpp::voting::vote_info_storage::contested_document_vote_poll_winner_info::ContestedDocumentVotePollWinnerInfo;
use dash_sdk::platform::Identifier;
use dash_sdk::query_types::Contenders;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

/// Key prefix for DPNS contest cache entries in the per-network wallet
/// k/v store. Each contest occupies one record at
/// `det:contested_name:<normalized_name>` in the global scope.
const CONTESTED_NAME_KEY_PREFIX: &str = "det:contested_name:";

fn contested_name_key(normalized_name: &str) -> String {
    format!("{CONTESTED_NAME_KEY_PREFIX}{normalized_name}")
}

fn scheduled_vote_journal_summary(
    operations: &[DpnsVoteOperation],
    voter_id: Identifier,
    dismissed: &BTreeSet<(DpnsVoteOperationId, DpnsVoteTargetKey)>,
) -> (bool, bool) {
    let authoritative_by_target = authoritative_dpns_vote_outcomes(operations, |outcome| {
        outcome.target.key.voter_id == voter_id
            && matches!(outcome.target.timing, VoteTiming::Scheduled(_))
    });

    let pending = authoritative_by_target
        .values()
        .any(|(_, outcome)| outcome.status.holds_lock());
    let failed = authoritative_by_target.values().any(|(_, outcome)| {
        outcome.status.is_reviewable_failure()
            && !dismissed.contains(&(outcome.operation_id, outcome.target.key.clone()))
    });
    (pending, failed)
}

/// Persisted shape of a single DPNS contest. Contenders are nested so
/// the whole record reads atomically — pre-C6 stored them in a separate
/// `contestant` table joined at load time.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredContestedName {
    normalized_contested_name: String,
    locked_votes: Option<u32>,
    abstain_votes: Option<u32>,
    awarded_to: Option<[u8; 32]>,
    end_time: Option<TimestampMillis>,
    /// `true` once the contest has been resolved to "locked"
    /// (no winner). Distinct from `awarded_to`.
    locked: bool,
    last_updated: Option<TimestampMillis>,
    contestants: Vec<StoredContestant>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredContestant {
    id: [u8; 32],
    name: String,
    info: String,
    votes: u32,
    created_at: Option<TimestampMillis>,
    created_at_block_height: Option<BlockHeight>,
    created_at_core_block_height: Option<CoreBlockHeight>,
    document_id: [u8; 32],
}

fn vote_state_summary(states: &[DpnsCurrentVoteState]) -> MasternodeVoteStateSummary {
    if states.contains(&DpnsCurrentVoteState::Checking) {
        MasternodeVoteStateSummary::Checking
    } else if states.contains(&DpnsCurrentVoteState::Unavailable) {
        MasternodeVoteStateSummary::Unavailable
    } else {
        MasternodeVoteStateSummary::Ready
    }
}

impl StoredContestedName {
    /// Whether the contest ended and its outcome is stored. That outcome is
    /// final, so the record has nothing left to refresh.
    fn has_outcome(&self) -> bool {
        self.locked || self.awarded_to.is_some()
    }

    fn to_contested_name(&self, network: Network) -> ContestedName {
        let join_window = crate::model::dpns::contest_durations(
            network,
            crate::context::default_platform_version(&network),
        )
        .join;
        let awarded_to_id = self.awarded_to.map(Identifier::from);

        // Match pre-C6 semantics: state is computed from the latest
        // contestant's `created_at`, falling back to `Unknown` when no
        // contestant timestamps are available.
        let latest_created_at = self.contestants.iter().rev().find_map(|c| c.created_at);
        let state = if self.locked {
            ContestState::Locked
        } else if let Some(id) = awarded_to_id {
            ContestState::WonBy(id)
        } else if let Some(created_at) = latest_created_at {
            let elapsed = Duration::from_millis(now_ms().saturating_sub(created_at));
            if elapsed <= join_window {
                ContestState::Joinable
            } else {
                ContestState::Ongoing
            }
        } else {
            ContestState::Unknown
        };

        let contestants = self
            .contestants
            .iter()
            .map(|c| Contestant {
                id: Identifier::from(c.id),
                name: c.name.clone(),
                info: c.info.clone(),
                votes: c.votes,
                created_at: c.created_at,
                created_at_block_height: c.created_at_block_height,
                created_at_core_block_height: c.created_at_core_block_height,
                document_id: Identifier::from(c.document_id),
            })
            .collect();

        ContestedName {
            normalized_contested_name: self.normalized_contested_name.clone(),
            contestants: Some(contestants),
            locked_votes: self.locked_votes,
            abstain_votes: self.abstain_votes,
            awarded_to: awarded_to_id,
            end_time: self.end_time,
            state,
            last_updated: self.last_updated,
            my_votes: BTreeMap::new(),
        }
    }
}

/// Map a k/v adapter failure to the DPNS-contest storage error.
fn contest_err(source: KvAdapterError) -> TaskError {
    TaskError::ContestStorage { source }
}

fn username_err(source: KvAdapterError) -> TaskError {
    TaskError::UsernameStorage { source }
}

impl AppContext {
    /// Protocol version the SDK currently speaks; it sets the community vote fee charged at registration.
    pub fn sdk_platform_version(&self) -> &'static dash_sdk::dpp::version::PlatformVersion {
        self.sdk.load().version()
    }

    /// Warm contestant labels once storage becomes available.
    pub(crate) fn refresh_dpns_candidate_labels(&self) -> Result<(), TaskError> {
        let mut labels = self
            .dpns_candidate_labels
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *labels = self
            .all_contested_names()?
            .into_iter()
            .map(|contest| {
                (
                    contest.normalized_contested_name,
                    contest
                        .contestants
                        .unwrap_or_default()
                        .into_iter()
                        .map(|candidate| (candidate.id, candidate.name))
                        .collect(),
                )
            })
            .collect();
        Ok(())
    }

    /// Return a cached display label without reading persistent storage.
    pub(crate) fn dpns_candidate_label(&self, name: &str, candidate: Identifier) -> Option<String> {
        self.dpns_candidate_labels
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name)?
            .get(&candidate)
            .cloned()
    }

    /// Fetches every DPNS contest cached in the per-network k/v store.
    pub fn all_contested_names(&self) -> std::result::Result<Vec<ContestedName>, TaskError> {
        let kv = self.det_kv()?;
        let keys = kv
            .list(DetScope::Global, Some(CONTESTED_NAME_KEY_PREFIX))
            .map_err(contest_err)?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            match kv.get::<StoredContestedName>(DetScope::Global, &key) {
                Ok(Some(stored)) => out.push(stored.to_contested_name(self.network)),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    key = %key,
                    error = ?e,
                    "Skipping unreadable contested-name entry",
                ),
            }
        }
        Ok(out)
    }

    /// Fetches every DPNS contest cached in the per-network k/v store whose
    /// `end_time` is in the future (or unknown).
    pub fn ongoing_contested_names(&self) -> std::result::Result<Vec<ContestedName>, TaskError> {
        let current_timestamp = now_ms();
        let kv = self.det_kv()?;
        let keys = kv
            .list(DetScope::Global, Some(CONTESTED_NAME_KEY_PREFIX))
            .map_err(contest_err)?;
        let mut out = Vec::new();
        for key in keys {
            match kv.get::<StoredContestedName>(DetScope::Global, &key) {
                Ok(Some(stored)) => match stored.end_time {
                    Some(t) if t <= current_timestamp => {}
                    _ => out.push(stored.to_contested_name(self.network)),
                },
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    key = %key,
                    error = ?e,
                    "Skipping unreadable contested-name entry",
                ),
            }
        }
        Ok(out)
    }

    /// Reload the frame-safe username snapshot from the per-network store.
    ///
    /// Called at startup and after contest refreshes so the hub reads requests,
    /// main-name choices and seen outcomes without touching storage per frame.
    pub(crate) fn refresh_pending_dpns_usernames(&self) -> Result<(), TaskError> {
        let kv = self.det_kv()?;
        let mut current = self.username_cache_mut();
        let mut cache = UsernameCache::default();
        for key in kv
            .list(DetScope::Global, Some(USERNAME_REQUESTS_KEY_PREFIX))
            .map_err(username_err)?
        {
            let Some(id) = identity_from_key(&key, USERNAME_REQUESTS_KEY_PREFIX) else {
                continue;
            };
            if self.is_identity_unloaded(&kv, &id.to_buffer())? && !self.is_identity_listed(&id)? {
                continue;
            }
            if let Some(requests) = kv
                .get::<Vec<UsernameRequest>>(DetScope::Global, &key)
                .map_err(username_err)?
            {
                if requests
                    .iter()
                    .any(|request| request.phase == RequestPhase::Won)
                {
                    cache.hydration_pending.insert(id);
                }
                cache.requests.insert(id, requests);
            }
        }
        for key in kv
            .list(DetScope::Global, Some(MAIN_USERNAME_KEY_PREFIX))
            .map_err(username_err)?
        {
            if let Some(id) = identity_from_key(&key, MAIN_USERNAME_KEY_PREFIX)
                && !(self.is_identity_unloaded(&kv, &id.to_buffer())?
                    && !self.is_identity_listed(&id)?)
                && let Some(name) = kv
                    .get::<String>(DetScope::Global, &key)
                    .map_err(username_err)?
            {
                cache.main.insert(id, name);
            }
        }
        for key in kv
            .list(DetScope::Global, Some(SEEN_OUTCOMES_KEY_PREFIX))
            .map_err(username_err)?
        {
            if let Some(id) = identity_from_key(&key, SEEN_OUTCOMES_KEY_PREFIX)
                && !(self.is_identity_unloaded(&kv, &id.to_buffer())?
                    && !self.is_identity_listed(&id)?)
                && let Some(seen) = kv
                    .get::<BTreeSet<String>>(DetScope::Global, &key)
                    .map_err(username_err)?
            {
                cache.seen.insert(id, seen);
            }
        }
        cache.seen_marks = std::mem::take(&mut current.seen_marks);
        *current = cache;
        Ok(())
    }

    fn username_cache(&self) -> std::sync::RwLockReadGuard<'_, UsernameCache> {
        self.pending_dpns_usernames
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Write guard over the username snapshot. Every username write holds it
    /// across its read-modify-write and store update, so writers never interleave.
    pub(crate) fn username_cache_mut(&self) -> std::sync::RwLockWriteGuard<'_, UsernameCache> {
        self.pending_dpns_usernames
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Every stored username request of `identity_id`, pending first. Frame-safe.
    pub fn username_requests_for(&self, identity_id: &Identifier) -> Vec<UsernameRequest> {
        let now = crate::utils::time::now_ms();
        self.username_cache()
            .requests
            .get(identity_id)
            .into_iter()
            .flatten()
            .cloned()
            .map(|mut request| {
                request.phase = request.phase_at(now);
                request
            })
            .collect()
    }

    /// Whether any identity has a request still in its community vote. Frame-safe.
    pub fn any_pending_username_request(&self) -> bool {
        self.username_cache()
            .requests
            .values()
            .flatten()
            .any(|request| request.phase.is_pending())
    }

    /// Pending contests and saved wins both need periodic refresh until owned names are stored.
    pub fn username_requests_need_refresh(&self) -> bool {
        self.any_pending_username_request() || !self.username_cache().hydration_pending.is_empty()
    }

    pub(crate) fn update_username_hydration_status(
        &self,
        identity_id: Identifier,
        names: &[crate::model::qualified_identity::DPNSNameInfo],
    ) -> bool {
        let mut cache = self.username_cache_mut();
        let pending = cache.requests.get(&identity_id).is_some_and(|requests| {
            requests
                .iter()
                .any(|request| won_username_is_missing(request, names))
        });
        if pending {
            cache.hydration_pending.insert(identity_id);
        } else {
            cache.hydration_pending.remove(&identity_id);
        }
        pending
    }

    /// Persist `requests` as the full request list of `identity_id`.
    pub fn store_username_requests(
        &self,
        identity_id: &Identifier,
        requests: Vec<UsernameRequest>,
    ) -> Result<(), TaskError> {
        self.update_username_requests(identity_id, |_| requests)
    }

    /// Replace the request list of `identity_id` with `update(current list)`.
    ///
    /// The read, the store write and the snapshot update run under one guard,
    /// so a request recorded by a concurrent registration is never lost.
    pub(crate) fn update_username_requests(
        &self,
        identity_id: &Identifier,
        update: impl FnOnce(&[UsernameRequest]) -> Vec<UsernameRequest>,
    ) -> Result<(), TaskError> {
        let kv = self.det_kv()?;
        let key = identity_key(USERNAME_REQUESTS_KEY_PREFIX, identity_id);
        let mut cache = self.username_cache_mut();
        let current = kv
            .get::<Vec<UsernameRequest>>(DetScope::Global, &key)
            .map_err(username_err)?
            .unwrap_or_default();
        let requests = update(&current);
        if requests != current {
            if requests.is_empty() {
                kv.delete(DetScope::Global, &key).map_err(username_err)?;
            } else {
                kv.put(DetScope::Global, &key, &requests)
                    .map_err(username_err)?;
            }
        }
        if requests.is_empty() {
            cache.requests.remove(identity_id);
            cache.hydration_pending.remove(identity_id);
        } else {
            if requests
                .iter()
                .any(|request| request.phase == RequestPhase::Won)
            {
                cache.hydration_pending.insert(*identity_id);
            }
            cache.requests.insert(*identity_id, requests);
        }
        Ok(())
    }

    /// The first pending request while `identity` owns no registered name. Frame-safe.
    pub fn pending_dpns_username_for_identity(
        &self,
        identity: &QualifiedIdentity,
    ) -> Option<UsernameRequest> {
        if identity_owns_dpns_name(identity) {
            return None;
        }
        self.username_requests_for(&identity.identity.id())
            .into_iter()
            .find(|request| request.phase.is_pending())
    }

    /// The registered name `identity` shows as its main one. Frame-safe.
    ///
    /// The local "Show as main" choice wins while the identity still owns that
    /// name; otherwise the first registered name.
    pub fn main_username(&self, identity: &QualifiedIdentity) -> Option<String> {
        let owned = identity
            .dpns_names
            .iter()
            .map(|name| name.name.trim())
            .filter(|name| !name.is_empty());
        let preferred = self
            .username_cache()
            .main
            .get(&identity.identity.id())
            .cloned();
        let mut first = None;
        for name in owned {
            if preferred.as_deref() == Some(name) {
                return Some(name.to_owned());
            }
            first.get_or_insert(name);
        }
        first.map(str::to_owned)
    }

    /// Move the identity's main username to the front of `dpns_names`, so every
    /// surface that shows the first registered name shows the chosen one.
    pub(crate) fn order_main_username_first(&self, identity: &mut QualifiedIdentity) {
        let Some(main) = self.main_username(identity) else {
            return;
        };
        if let Some(index) = identity
            .dpns_names
            .iter()
            .position(|n| n.name.trim() == main)
        {
            identity.dpns_names[..=index].rotate_right(1);
        }
    }

    /// Show `name` as the main username of `identity_id` on this device.
    pub fn set_main_username(&self, identity_id: &Identifier, name: &str) -> Result<(), TaskError> {
        let kv = self.det_kv()?;
        let mut cache = self.username_cache_mut();
        kv.put(
            DetScope::Global,
            &identity_key(MAIN_USERNAME_KEY_PREFIX, identity_id),
            &name.to_owned(),
        )
        .map_err(username_err)?;
        cache.main.insert(*identity_id, name.to_owned());
        Ok(())
    }

    /// Whether the one-time banner for `request`'s outcome was already shown. Frame-safe.
    pub fn username_outcome_seen(
        &self,
        identity_id: &Identifier,
        request: &UsernameRequest,
    ) -> bool {
        self.username_cache()
            .seen
            .get(identity_id)
            .is_some_and(|seen| seen.contains(&outcome_key(request)))
    }

    /// Whether a "banner seen" write for `identity_id` may be dispatched in
    /// frame `pass`; records the dispatch when it may. Frame-safe, in memory only.
    pub fn try_dispatch_username_seen_mark(&self, identity_id: &Identifier, pass: u64) -> bool {
        let mut cache = self.username_cache_mut();
        let attempts = cache.seen_marks.entry(*identity_id).or_default();
        if !attempts.may_dispatch(pass, std::time::Instant::now()) {
            return false;
        }
        attempts.dispatched(pass);
        true
    }

    /// Update the in-memory "banner seen" write bounds as the backend runs it.
    pub(crate) fn note_username_seen_mark(&self, identity_id: &Identifier, outcome: Option<bool>) {
        let mut cache = self.username_cache_mut();
        let attempts = cache.seen_marks.entry(*identity_id).or_default();
        match outcome {
            None => attempts.started(),
            Some(succeeded) => attempts.finished(succeeded, std::time::Instant::now()),
        }
    }

    /// Record that the outcome banners for `requests` were shown.
    pub fn mark_username_outcomes_seen(
        &self,
        identity_id: &Identifier,
        requests: &[UsernameRequest],
    ) -> Result<(), TaskError> {
        let kv = self.det_kv()?;
        let mut cache = self.username_cache_mut();
        let mut seen = cache.seen.get(identity_id).cloned().unwrap_or_default();
        let before = seen.len();
        seen.extend(requests.iter().map(outcome_key));
        if seen.len() == before {
            return Ok(());
        }
        kv.put(
            DetScope::Global,
            &identity_key(SEEN_OUTCOMES_KEY_PREFIX, identity_id),
            &seen,
        )
        .map_err(username_err)?;
        cache.seen.insert(*identity_id, seen);
        Ok(())
    }

    /// Remove a finished request from the identity's list.
    pub fn dismiss_username_request(
        &self,
        identity_id: &Identifier,
        normalized_label: &str,
    ) -> Result<(), TaskError> {
        self.update_username_requests(identity_id, |requests| {
            requests
                .iter()
                .filter(|r| r.phase.is_pending() || r.normalized_label != normalized_label)
                .cloned()
                .collect()
        })
    }

    /// Drop every username record of a removed identity, so its requests stop
    /// driving refreshes and its main-name choice does not return on re-import.
    pub(crate) fn forget_identity_usernames(
        &self,
        kv: &crate::wallet_backend::DetKv,
        identity_id: &Identifier,
    ) -> Result<(), TaskError> {
        let mut cache = self.username_cache_mut();
        cache.requests.remove(identity_id);
        cache.hydration_pending.remove(identity_id);
        cache.main.remove(identity_id);
        cache.seen.remove(identity_id);
        cache.seen_marks.remove(identity_id);
        let mut first_error = None;
        for prefix in [
            USERNAME_REQUESTS_KEY_PREFIX,
            MAIN_USERNAME_KEY_PREFIX,
            SEEN_OUTCOMES_KEY_PREFIX,
        ] {
            if let Err(error) = kv.delete(DetScope::Global, &identity_key(prefix, identity_id)) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), |error| Err(username_err(error)))
    }

    /// Seed a current contest through the persisted shape for backend contract tests.
    #[cfg(test)]
    pub(crate) fn seed_dpns_contest_for_test(
        &self,
        name: &str,
        end_time: Option<u64>,
        closed: bool,
    ) {
        let now = now_ms();
        let stored = StoredContestedName {
            normalized_contested_name: name.into(),
            end_time,
            locked: closed,
            contestants: vec![StoredContestant {
                id: [3; 32],
                name: name.into(),
                info: String::new(),
                votes: 0,
                created_at: Some(now),
                created_at_block_height: None,
                created_at_core_block_height: None,
                document_id: [4; 32],
            }],
            ..Default::default()
        };
        self.det_kv()
            .unwrap()
            .put(DetScope::Global, &contested_name_key(name), &stored)
            .unwrap();
    }

    /// Summarise a masternode/evonode node's DPNS voting position for its card.
    ///
    /// `voter_id` is the node's **own** identity id — its ProTxHash, the id
    /// Platform records masternode votes under and the key both the proved-vote
    /// cache and the vote journal use. It is never the separate
    /// `associated_voter_identity` record, which only supplies the signing key.
    /// Pass `None` for a node with no voting key loaded — it can vote on
    /// nothing, so the summary is empty. The open count reads the ongoing
    /// contest cache and the scheduled-vote flag reuses the existing DPNS
    /// Scheduled Votes state (no new backend concept — §10.1).
    pub fn masternode_contest_summary(
        &self,
        voter_id: Option<Identifier>,
    ) -> std::result::Result<crate::model::contested_name::MasternodeContestSummary, TaskError>
    {
        let Some(voter_id) = voter_id else {
            return Ok(crate::model::contested_name::MasternodeContestSummary::default());
        };
        Ok(self
            .masternode_contest_summaries(&[voter_id])?
            .remove(&voter_id)
            .unwrap_or_default())
    }

    /// Summarise many nodes at once, reading the shared state a single time.
    ///
    /// The contest cache, the derived vote-poll ids, the operation journal and
    /// the schedule dismissals all depend on the contest set rather than the
    /// node, so an operator with a large fleet must not pay for them per card.
    /// Only the proved current-vote snapshot is genuinely per node. Nodes whose
    /// summary cannot be built are omitted.
    pub fn masternode_contest_summaries(
        &self,
        voter_ids: &[Identifier],
    ) -> std::result::Result<
        BTreeMap<Identifier, crate::model::contested_name::MasternodeContestSummary>,
        TaskError,
    > {
        if voter_ids.is_empty() {
            return Ok(BTreeMap::new());
        }

        let contests = self.ongoing_contested_names()?;
        let open_polls: Vec<Option<Identifier>> = contests
            .iter()
            .filter(|contest| contest.is_votable())
            .map(|contest| {
                self.dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok()
            })
            .collect();
        let open_contest_count = open_polls.len();
        let poll_ids: Vec<Identifier> = open_polls.iter().flatten().copied().collect();
        let operations = self.dpns_vote_operations()?;
        let dismissed = self.dismissed_dpns_vote_schedules()?;

        Ok(voter_ids
            .iter()
            .map(|voter_id| {
                // One storage read per node for every open contest's proved state
                // (VOTE-NFR-007), instead of one read per contest. A read failure
                // degrades each contest to `Unavailable`, matching the prior
                // per-contest fallback.
                let poll_states = if poll_ids.is_empty() {
                    BTreeMap::new()
                } else {
                    self.dpns_current_vote_states(*voter_id, poll_ids.iter().copied())
                        .unwrap_or_default()
                };
                let states = open_polls
                    .iter()
                    .map(|poll| {
                        poll.and_then(|poll_id| poll_states.get(&poll_id).copied())
                            .unwrap_or(DpnsCurrentVoteState::Unavailable)
                    })
                    .collect::<Vec<_>>();
                let needs_vote_count = states
                    .iter()
                    .filter(|state| **state == DpnsCurrentVoteState::Available(None))
                    .count();
                let (has_scheduled_vote, has_failed_scheduled_vote) =
                    scheduled_vote_journal_summary(&operations, *voter_id, &dismissed);
                (
                    *voter_id,
                    crate::model::contested_name::MasternodeContestSummary {
                        open_contest_count,
                        needs_vote_count,
                        vote_state: vote_state_summary(&states),
                        has_scheduled_vote,
                        has_failed_scheduled_vote,
                    },
                )
            })
            .collect())
    }

    /// Apply a batch of newly-seen normalized names. New names are stored
    /// as empty contest skeletons; existing names whose `last_updated` is
    /// older than 30 s are returned alongside new names for the caller to
    /// refresh — matching pre-C6 staleness gating. A contest whose outcome
    /// is stored is final: it is stamped as current and never returned, as
    /// the caller spends one query per returned name.
    pub fn insert_name_contests_as_normalized_names(
        &self,
        name_contests: Vec<String>,
    ) -> std::result::Result<Vec<String>, TaskError> {
        let kv = self.det_kv()?;
        let now = chrono::Utc::now().timestamp();
        let stale_threshold = now - 30;
        let mut new_names: Vec<String> = Vec::new();
        let mut stale: Vec<(String, Option<i64>)> = Vec::new();

        for name in name_contests {
            let key = contested_name_key(&name);
            match kv
                .get::<StoredContestedName>(DetScope::Global, &key)
                .map_err(contest_err)?
            {
                None => {
                    let stored = StoredContestedName {
                        normalized_contested_name: name.clone(),
                        ..Default::default()
                    };
                    kv.put(DetScope::Global, &key, &stored)
                        .map_err(contest_err)?;
                    new_names.push(name);
                }
                Some(mut stored) if stored.has_outcome() => {
                    stored.last_updated = Some(now as u64);
                    kv.put(DetScope::Global, &key, &stored)
                        .map_err(contest_err)?;
                }
                Some(stored) => {
                    let last_updated = stored.last_updated.map(|t| t as i64);
                    if last_updated.is_none_or(|t| t < stale_threshold) {
                        stale.push((name, last_updated));
                    }
                }
            }
        }

        // Combine new and stale names (oldest first), preserving the
        // pre-C6 ordering callers may rely on.
        stale.extend(new_names.into_iter().map(|name| (name, None)));
        stale.sort_by_key(|a| a.1.unwrap_or(0));
        Ok(stale.into_iter().map(|(name, _)| name).collect())
    }

    /// Put the contests known to be still open — those with an end time in
    /// the future — ahead of the rest, keeping the order within each group.
    pub(crate) fn open_contests_first(
        &self,
        names: Vec<String>,
    ) -> std::result::Result<Vec<String>, TaskError> {
        let kv = self.det_kv()?;
        let now = now_ms();
        let mut open = Vec::new();
        let mut rest = Vec::new();
        for name in names {
            let end_time = kv
                .get::<StoredContestedName>(DetScope::Global, &contested_name_key(&name))
                .map_err(contest_err)?
                .and_then(|stored| stored.end_time);
            if end_time.is_some_and(|end| end > now) {
                open.push(name);
            } else {
                rest.push(name);
            }
        }
        open.append(&mut rest);
        Ok(open)
    }

    /// Update a single contest record with the latest set of contenders.
    /// Mirrors the pre-C6 `insert_or_update_contenders` behavior: when a
    /// winner is decided, only the resolution fields are written; otherwise
    /// vote tallies and the contender list are refreshed.
    pub fn insert_or_update_contenders(
        &self,
        normalized_contested_name: &str,
        contenders: &Contenders,
        dpns_domain_document_type: DocumentTypeRef,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let mut labels = self
            .dpns_candidate_labels
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = contested_name_key(normalized_contested_name);
        let last_updated = chrono::Utc::now().timestamp() as u64;

        let mut stored = kv
            .get::<StoredContestedName>(DetScope::Global, &key)
            .map_err(contest_err)?
            .unwrap_or_else(|| StoredContestedName {
                normalized_contested_name: normalized_contested_name.to_string(),
                ..Default::default()
            });

        if let Some((winner, block_info)) = contenders.winner {
            match winner {
                ContestedDocumentVotePollWinnerInfo::NoWinner => {}
                ContestedDocumentVotePollWinnerInfo::WonByIdentity(won_by) => {
                    stored.awarded_to = Some(won_by.to_buffer());
                    stored.last_updated = Some(last_updated);
                    stored.end_time = Some(block_info.time_ms);
                    kv.put(DetScope::Global, &key, &stored)
                        .map_err(contest_err)?;
                }
                ContestedDocumentVotePollWinnerInfo::Locked => {
                    stored.locked = true;
                    stored.last_updated = Some(last_updated);
                    stored.end_time = Some(block_info.time_ms);
                    kv.put(DetScope::Global, &key, &stored)
                        .map_err(contest_err)?;
                }
            }
            labels.insert(
                normalized_contested_name.to_owned(),
                stored
                    .contestants
                    .iter()
                    .map(|candidate| (Identifier::from(candidate.id), candidate.name.clone()))
                    .collect(),
            );
            return Ok(());
        }

        stored.locked_votes = Some(contenders.lock_vote_tally.unwrap_or(0));
        stored.abstain_votes = Some(contenders.abstain_vote_tally.unwrap_or(0));
        stored.last_updated = Some(last_updated);

        let mut existing: HashMap<[u8; 32], StoredContestant> =
            stored.contestants.drain(..).map(|c| (c.id, c)).collect();

        for (identity_id, contender) in &contenders.contenders {
            let deserialized_contender = contender
                .try_to_contender(dpns_domain_document_type, self.platform_version())
                .map_err(|source| TaskError::ContractEncoding {
                    source: Box::new(source),
                })?;
            let Some(document) = deserialized_contender.document().as_ref() else {
                tracing::warn!(
                    %identity_id,
                    "Skipping contender with missing document while updating cache",
                );
                continue;
            };
            let Some(name) = document.get("label").and_then(|v| v.as_str()) else {
                tracing::warn!(
                    %identity_id,
                    "Skipping contender with missing label while updating cache",
                );
                continue;
            };
            let buf = identity_id.to_buffer();
            let votes = contender.vote_tally().unwrap_or(0);
            match existing.remove(&buf) {
                Some(mut prev) => {
                    prev.votes = votes;
                    prev.name = name.to_string();
                    prev.document_id = document.id().to_buffer();
                    prev.created_at = document.created_at();
                    prev.created_at_block_height = document.created_at_block_height();
                    prev.created_at_core_block_height = document.created_at_core_block_height();
                    stored.contestants.push(prev);
                }
                None => stored.contestants.push(StoredContestant {
                    id: buf,
                    name: name.to_string(),
                    info: String::new(),
                    votes,
                    created_at: document.created_at(),
                    created_at_block_height: document.created_at_block_height(),
                    created_at_core_block_height: document.created_at_core_block_height(),
                    document_id: document.id().to_buffer(),
                }),
            }
        }

        kv.put(DetScope::Global, &key, &stored)
            .map_err(contest_err)?;
        labels.insert(
            normalized_contested_name.to_owned(),
            stored
                .contestants
                .iter()
                .map(|candidate| (Identifier::from(candidate.id), candidate.name.clone()))
                .collect(),
        );
        Ok(())
    }

    /// Patch the `end_time` for a batch of contests. Mirrors pre-C6:
    /// the new ending time is only written when it advances past any
    /// previously stored one, or when no ending time was recorded yet.
    pub fn update_contested_name_ending_times<I>(
        &self,
        name_contests: I,
    ) -> std::result::Result<(), TaskError>
    where
        I: IntoIterator<Item = (String, TimestampMillis)>,
    {
        let kv = self.det_kv()?;
        for (name, new_end_time) in name_contests {
            let key = contested_name_key(&name);
            let Some(mut stored) = kv
                .get::<StoredContestedName>(DetScope::Global, &key)
                .map_err(contest_err)?
            else {
                continue;
            };
            match stored.end_time {
                Some(t) if t >= new_end_time => continue,
                _ => {
                    stored.end_time = Some(new_end_time);
                    kv.put(DetScope::Global, &key, &stored)
                        .map_err(contest_err)?;
                }
            }
        }
        Ok(())
    }
}

/// Per-identity request lists, keyed `det:username_requests:<identity base58>`.
const USERNAME_REQUESTS_KEY_PREFIX: &str = "det:username_requests:";
/// Device-only main-name choice, keyed `det:main_username:<identity base58>`.
const MAIN_USERNAME_KEY_PREFIX: &str = "det:main_username:";
/// Outcome banners already shown, keyed `det:username_outcomes_seen:<identity base58>`.
const SEEN_OUTCOMES_KEY_PREFIX: &str = "det:username_outcomes_seen:";

/// Frame-safe copy of the identity-side username state.
#[derive(Debug, Default)]
pub(crate) struct UsernameCache {
    requests: HashMap<Identifier, Vec<UsernameRequest>>,
    hydration_pending: BTreeSet<Identifier>,
    main: HashMap<Identifier, String>,
    seen: HashMap<Identifier, BTreeSet<String>>,
    /// In-memory write bounds for "banner seen"; never persisted.
    pub(crate) seen_marks: HashMap<Identifier, SeenMarkAttempts>,
}

fn identity_key(prefix: &str, identity_id: &Identifier) -> String {
    format!(
        "{prefix}{}",
        identity_id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
    )
}

fn identity_from_key(key: &str, prefix: &str) -> Option<Identifier> {
    Identifier::from_string(
        key.strip_prefix(prefix)?,
        dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
    )
    .ok()
}

fn outcome_key(request: &UsernameRequest) -> String {
    format!("{}:{:?}", request.normalized_label, request.phase)
}

fn identity_owns_dpns_name(identity: &QualifiedIdentity) -> bool {
    identity
        .dpns_names
        .iter()
        .any(|name| !name.name.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use crate::model::dpns_usernames::RequestPhase;
    use crate::model::dpns_voting::DpnsVoteTargetStatus;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{
        DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
    };
    use crate::wallet_backend::DetKv;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use platform_wallet_storage::{KvError, KvStore, ObjectId};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn empty_kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    fn contestant(id: u8, created_at: Option<TimestampMillis>) -> StoredContestant {
        StoredContestant {
            id: [id; 32],
            name: format!("name-{id}"),
            info: String::new(),
            votes: u32::from(id),
            created_at,
            created_at_block_height: None,
            created_at_core_block_height: None,
            document_id: [id; 32],
        }
    }

    fn qualified_identity(id: u8, dpns_name: &str) -> QualifiedIdentity {
        let identity =
            Identity::create_basic_identity(Identifier::from([id; 32]), PlatformVersion::latest())
                .expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![DPNSNameInfo {
                name: dpns_name.to_string(),
                acquired_at: 0,
            }],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    fn request(label: &str, phase: RequestPhase) -> UsernameRequest {
        UsernameRequest {
            label: label.to_owned(),
            normalized_label: label.to_owned(),
            phase,
            requested_at: None,
            join_end: None,
            end: None,
            decided_at: None,
            tally: Default::default(),
            last_updated: 0,
            reflected_in_owned_names: false,
        }
    }

    #[derive(Default)]
    struct ReloadReadHookKv {
        inner: InMemoryKv,
        after_read: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl KvStore for ReloadReadHookKv {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            let value = self.inner.get(scope, key)?;
            let hook = self.after_read.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
            Ok(value)
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

    #[test]
    fn review_safety_username_reload_preserves_overlapping_request_write() {
        let temp = tempfile::tempdir().unwrap();
        let context = test_app_context(temp.path());
        let store = Arc::new(ReloadReadHookKv::default());
        context.set_det_kv_override_for_test(DetKv::from_store(store.clone()));
        let id = Identifier::from([1; 32]);
        let original = request("alpha", RequestPhase::Voting);
        context
            .store_username_requests(&id, vec![original.clone()])
            .unwrap();
        let weak = Arc::downgrade(&context);
        let (worker_tx, worker_rx) = std::sync::mpsc::channel();
        *store.after_read.lock().unwrap() = Some(Box::new(move || {
            let context = weak.upgrade().unwrap();
            let serialized = matches!(
                context.pending_dpns_usernames.try_write(),
                Err(std::sync::TryLockError::WouldBlock)
            );
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                context
                    .update_username_requests(&id, |current| {
                        let mut requests = current.to_vec();
                        requests.push(request("beta", RequestPhase::Voting));
                        requests
                    })
                    .unwrap();
                let _ = done_tx.send(());
            });
            worker_tx.send(worker).unwrap();
            // Force the stale-read interleaving unless the reload excludes the writer.
            if !serialized {
                done_rx.recv().unwrap();
            }
        }));
        context.refresh_pending_dpns_usernames().unwrap();
        worker_rx.recv().unwrap().join().unwrap();
        let expected = vec![original, request("beta", RequestPhase::Voting)];
        assert_eq!(context.username_requests_for(&id), expected);
        context.refresh_pending_dpns_usernames().unwrap();
        assert_eq!(context.username_requests_for(&id), expected);
    }

    #[test]
    fn review_safety_username_reload_preserves_snapshot_on_record_read_failure() {
        use crate::wallet_backend::kv_test_support::FailingKv;
        for prefix in [
            USERNAME_REQUESTS_KEY_PREFIX,
            MAIN_USERNAME_KEY_PREFIX,
            SEEN_OUTCOMES_KEY_PREFIX,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let context = test_app_context(temp.path());
            let store = Arc::new(FailingKv::default());
            context.set_det_kv_override_for_test(DetKv::from_store(store.clone()));
            let id = Identifier::from([1; 32]);
            let original = request("alpha", RequestPhase::Won);
            context
                .store_username_requests(&id, vec![original.clone()])
                .unwrap();
            context.set_main_username(&id, "alpha").unwrap();
            context
                .mark_username_outcomes_seen(&id, std::slice::from_ref(&original))
                .unwrap();
            store.fail_next_gets_containing(prefix, 1);
            let result = context.refresh_pending_dpns_usernames();
            assert_eq!(
                context.username_requests_for(&id),
                vec![original.clone()],
                "{prefix}"
            );
            assert_eq!(
                context.username_cache().main.get(&id).map(String::as_str),
                Some("alpha"),
                "{prefix}"
            );
            assert!(context.username_outcome_seen(&id, &original), "{prefix}");
            assert!(result.is_err(), "a failed {prefix} read must be reported");
            context
                .update_username_requests(&id, |current| {
                    let mut requests = current.to_vec();
                    requests.push(request("beta", RequestPhase::Voting));
                    requests
                })
                .unwrap();
            context.refresh_pending_dpns_usernames().unwrap();
            assert_eq!(
                context.username_requests_for(&id),
                vec![original, request("beta", RequestPhase::Voting)]
            );
        }
    }

    #[test]
    fn username_append_after_failed_initial_load_preserves_saved_requests() {
        use crate::wallet_backend::kv_test_support::FailingKv;
        let temp = tempfile::tempdir().unwrap();
        let context = test_app_context(temp.path());
        let store = Arc::new(FailingKv::default());
        let kv = DetKv::from_store(store.clone());
        context.set_det_kv_override_for_test(kv.clone());
        let id = Identifier::from([1; 32]);
        let key = identity_key(USERNAME_REQUESTS_KEY_PREFIX, &id);
        let original = request("alpha", RequestPhase::Won);
        kv.put(DetScope::Global, &key, &vec![original.clone()])
            .unwrap();
        store.fail_next_gets_containing(USERNAME_REQUESTS_KEY_PREFIX, 1);
        assert!(context.refresh_pending_dpns_usernames().is_err());
        assert!(context.username_requests_for(&id).is_empty());

        // An unreadable authoritative record must never be treated as an empty list.
        store.fail_next_gets_containing(USERNAME_REQUESTS_KEY_PREFIX, 1);
        assert!(
            context
                .update_username_requests(&id, |_| panic!("must not update"))
                .is_err()
        );
        context
            .update_username_requests(&id, |current| {
                let mut requests = current.to_vec();
                requests.push(request("beta", RequestPhase::Voting));
                requests
            })
            .unwrap();
        let expected = vec![original, request("beta", RequestPhase::Voting)];
        assert_eq!(context.username_requests_for(&id), expected);
        assert!(context.username_requests_need_refresh());
        assert_eq!(
            kv.get::<Vec<UsernameRequest>>(DetScope::Global, &key)
                .unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn unchanged_username_update_populates_an_unloaded_cache() {
        let temp = tempfile::tempdir().unwrap();
        let context = test_app_context(temp.path());
        let kv = empty_kv();
        context.set_det_kv_override_for_test(kv.clone());
        let id = Identifier::from([1; 32]);
        let saved = vec![request("alpha", RequestPhase::Won)];
        kv.put(
            DetScope::Global,
            &identity_key(USERNAME_REQUESTS_KEY_PREFIX, &id),
            &saved,
        )
        .unwrap();
        context
            .update_username_requests(&id, |current| current.to_vec())
            .unwrap();
        assert_eq!(context.username_requests_for(&id), saved);
        assert!(context.username_requests_need_refresh());
    }

    #[test]
    fn reloaded_expired_request_has_current_phase_without_rewriting_snapshot() {
        let temp_dir = tempfile::tempdir().unwrap();
        let context = test_app_context(temp_dir.path());
        context.set_det_kv_override_for_test(empty_kv());
        let identity = qualified_identity(1, "");
        let id = identity.identity.id();
        let mut expired = request("d1ssh", RequestPhase::Joinable);
        expired.join_end = Some(1);
        expired.end = Some(2);
        context
            .store_username_requests(&id, vec![expired.clone()])
            .unwrap();
        *context.username_cache_mut() = UsernameCache::default();
        context.refresh_pending_dpns_usernames().unwrap();

        assert_eq!(
            context.username_requests_for(&id)[0].phase,
            RequestPhase::AwaitingOutcome
        );
        assert_eq!(
            context
                .pending_dpns_username_for_identity(&identity)
                .unwrap()
                .phase,
            RequestPhase::AwaitingOutcome
        );
        assert!(context.any_pending_username_request());
        assert_eq!(context.username_cache().requests[&id], vec![expired]);
    }

    #[test]
    fn pending_username_is_suppressed_once_identity_owns_a_name() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        context.set_det_kv_override_for_test(empty_kv());
        let owned = qualified_identity(1, "alice");
        let unowned = qualified_identity(2, "   ");
        for identity in [&owned, &unowned] {
            context
                .store_username_requests(
                    &identity.identity.id(),
                    vec![
                        request("lost", RequestPhase::Lost),
                        request("b0b", RequestPhase::Voting),
                    ],
                )
                .expect("store");
        }
        assert!(context.pending_dpns_username_for_identity(&owned).is_none());
        assert_eq!(
            context
                .pending_dpns_username_for_identity(&unowned)
                .map(|r| r.label),
            Some("b0b".to_owned())
        );
        // An owned name never hides the full request list.
        assert_eq!(context.username_requests_for(&owned.identity.id()).len(), 2);
    }

    #[test]
    fn username_state_survives_a_snapshot_reload() {
        // USR-TC-021 persistence half: the main-name choice and seen flags reload.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        context.set_det_kv_override_for_test(empty_kv());
        let mut identity = qualified_identity(1, "alice");
        identity.dpns_names.push(DPNSNameInfo {
            name: "bob".to_owned(),
            acquired_at: 0,
        });
        let id = identity.identity.id();
        assert_eq!(context.main_username(&identity).as_deref(), Some("alice"));
        context.set_main_username(&id, "bob").expect("set main");
        let won = request("carol", RequestPhase::Won);
        context
            .store_username_requests(&id, vec![won.clone()])
            .expect("store");
        context
            .mark_username_outcomes_seen(&id, std::slice::from_ref(&won))
            .expect("seen");

        *context.username_cache_mut() = UsernameCache::default();
        context.refresh_pending_dpns_usernames().expect("reload");

        assert_eq!(context.main_username(&identity).as_deref(), Some("bob"));
        assert!(context.username_outcome_seen(&id, &won));
        assert_eq!(context.username_requests_for(&id), vec![won.clone()]);

        context
            .dismiss_username_request(&id, "carol")
            .expect("dismiss");
        assert!(context.username_requests_for(&id).is_empty());
    }

    #[test]
    fn forgetting_an_identity_drops_its_username_records() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        let kv = empty_kv();
        context.set_det_kv_override_for_test(kv.clone());
        let id = qualified_identity(1, "alice").identity.id();
        let pending = request("carol", RequestPhase::Voting);
        context
            .store_username_requests(&id, vec![pending.clone()])
            .expect("store");
        context.set_main_username(&id, "alice").expect("main");
        assert!(context.any_pending_username_request());

        context.forget_identity_usernames(&kv, &id).expect("forget");
        context.refresh_pending_dpns_usernames().expect("reload");

        assert!(!context.any_pending_username_request());
        assert!(context.username_requests_for(&id).is_empty());
        assert!(!context.username_cache().main.contains_key(&id));
    }

    #[test]
    fn main_username_falls_back_when_preferred_name_is_gone() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        context.set_det_kv_override_for_test(empty_kv());
        let identity = qualified_identity(1, "alice");
        context
            .set_main_username(&identity.identity.id(), "gone")
            .expect("set main");
        assert_eq!(context.main_username(&identity).as_deref(), Some("alice"));
    }

    // ----------------------------------------------------------------
    // Decode: contest state branches the cache resolves at read time.
    // ----------------------------------------------------------------

    #[test]
    fn to_contested_name_reports_won_by_when_awarded() {
        let stored = StoredContestedName {
            normalized_contested_name: "dash".to_string(),
            awarded_to: Some([7u8; 32]),
            ..Default::default()
        };
        let cn = stored.to_contested_name(Network::Testnet);
        assert_eq!(cn.state, ContestState::WonBy(Identifier::from([7u8; 32])));
        assert_eq!(cn.awarded_to, Some(Identifier::from([7u8; 32])));
    }

    #[test]
    fn to_contested_name_reports_locked_when_locked_flag_set() {
        let stored = StoredContestedName {
            normalized_contested_name: "dash".to_string(),
            locked: true,
            ..Default::default()
        };
        let cn = stored.to_contested_name(Network::Testnet);
        assert_eq!(cn.state, ContestState::Locked);
        assert!(cn.awarded_to.is_none());
    }

    #[test]
    fn to_contested_name_locked_wins_over_awarded() {
        // `locked` is checked before `awarded_to`; a record carrying both
        // resolves to `Locked`.
        let stored = StoredContestedName {
            normalized_contested_name: "dash".to_string(),
            locked: true,
            awarded_to: Some([9u8; 32]),
            ..Default::default()
        };
        assert_eq!(
            stored.to_contested_name(Network::Testnet).state,
            ContestState::Locked
        );
    }

    #[test]
    fn to_contested_name_is_unknown_without_winner_or_timestamps() {
        // No winner, not locked, and no contestant timestamps to date the
        // contest against — the state stays `Unknown`.
        let stored = StoredContestedName {
            normalized_contested_name: "dash".to_string(),
            contestants: vec![contestant(1, None), contestant(2, None)],
            ..Default::default()
        };
        let cn = stored.to_contested_name(Network::Testnet);
        assert_eq!(cn.state, ContestState::Unknown);
        assert_eq!(cn.contestants.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn to_contested_name_preserves_contestant_fields() {
        let stored = StoredContestedName {
            normalized_contested_name: "dash".to_string(),
            locked: true,
            contestants: vec![contestant(3, Some(100))],
            ..Default::default()
        };
        let cn = stored.to_contested_name(Network::Testnet);
        let mapped = &cn.contestants.unwrap()[0];
        assert_eq!(mapped.id, Identifier::from([3u8; 32]));
        assert_eq!(mapped.name, "name-3");
        assert_eq!(mapped.votes, 3);
        assert_eq!(mapped.created_at, Some(100));
    }

    // ----------------------------------------------------------------
    // Storage: Global-scoped k/v round-trip of a contest record.
    // ----------------------------------------------------------------

    #[test]
    fn contest_record_round_trips_in_global_scope() {
        let kv = empty_kv();
        let key = contested_name_key("dash");
        let stored = StoredContestedName {
            normalized_contested_name: "dash".to_string(),
            locked_votes: Some(4),
            abstain_votes: Some(2),
            end_time: Some(1_700),
            last_updated: Some(1_600),
            contestants: vec![contestant(1, Some(10))],
            ..Default::default()
        };
        kv.put(DetScope::Global, &key, &stored).unwrap();

        let got: StoredContestedName = kv.get(DetScope::Global, &key).unwrap().unwrap();
        assert_eq!(got.normalized_contested_name, "dash");
        assert_eq!(got.locked_votes, Some(4));
        assert_eq!(got.abstain_votes, Some(2));
        assert_eq!(got.end_time, Some(1_700));
        assert_eq!(got.contestants.len(), 1);
        assert_eq!(got.contestants[0].created_at, Some(10));
    }

    #[test]
    fn missing_contest_record_reads_as_none() {
        let kv = empty_kv();
        let got: Option<StoredContestedName> = kv
            .get(DetScope::Global, &contested_name_key("never-stored"))
            .unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn contest_key_is_prefixed_with_normalized_name() {
        assert_eq!(contested_name_key("dash"), "det:contested_name:dash");
    }

    /// Every name queued here costs one contender query, so a finished contest
    /// must not be queued again, however long its history and however old its
    /// record.
    #[test]
    fn a_long_finished_history_queues_only_open_and_new_contests() {
        const FINISHED_CONTESTS: usize = 600;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        let kv = empty_kv();
        context.set_det_kv_override_for_test(kv.clone());
        let store = |stored: StoredContestedName| {
            let key = contested_name_key(&stored.normalized_contested_name);
            kv.put(DetScope::Global, &key, &stored).unwrap();
        };

        let mut listed = vec!["open".to_owned(), "brand-new".to_owned()];
        for index in 0..FINISHED_CONTESTS {
            let name = format!("finished-{index}");
            // Half were won, half ended locked; all were last read long ago.
            store(StoredContestedName {
                normalized_contested_name: name.clone(),
                awarded_to: (index % 2 == 0).then_some([7; 32]),
                locked: index % 2 == 1,
                last_updated: Some(1),
                ..Default::default()
            });
            listed.push(name);
        }
        store(StoredContestedName {
            normalized_contested_name: "open".to_owned(),
            contestants: vec![contestant(1, Some(1))],
            last_updated: Some(1),
            ..Default::default()
        });

        let mut queued = context
            .insert_name_contests_as_normalized_names(listed)
            .expect("queue contests for refresh");
        queued.sort();

        assert_eq!(queued, ["brand-new", "open"]);
    }

    /// A refresh that lists a finished contest has confirmed it, so History
    /// shows it as just updated although nothing more was asked about it.
    #[test]
    fn a_listed_finished_contest_reads_as_just_updated() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        let kv = empty_kv();
        context.set_det_kv_override_for_test(kv.clone());
        let key = contested_name_key("finished");
        let finished = StoredContestedName {
            normalized_contested_name: "finished".to_owned(),
            awarded_to: Some([7; 32]),
            end_time: Some(1_700),
            last_updated: Some(1),
            contestants: vec![contestant(1, Some(10))],
            ..Default::default()
        };
        kv.put(DetScope::Global, &key, &finished).unwrap();
        let listed_at = chrono::Utc::now().timestamp() as u64;

        let queued = context
            .insert_name_contests_as_normalized_names(vec!["finished".to_owned()])
            .expect("queue contests for refresh");

        assert!(queued.is_empty());
        let stored: StoredContestedName = kv.get(DetScope::Global, &key).unwrap().unwrap();
        assert!(stored.last_updated.is_some_and(|at| at >= listed_at));
        assert_eq!(stored.awarded_to, finished.awarded_to);
        assert_eq!(stored.end_time, finished.end_time);
        assert_eq!(stored.contestants.len(), 1);
    }

    #[test]
    fn card_summary_does_not_treat_checking_as_all_votes_cast() {
        assert_eq!(
            vote_state_summary(&[
                DpnsCurrentVoteState::Available(Some(
                    dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Lock,
                )),
                DpnsCurrentVoteState::Checking,
            ]),
            MasternodeVoteStateSummary::Checking
        );
    }

    #[test]
    fn card_summary_does_not_treat_unavailable_as_all_votes_cast() {
        assert_eq!(
            vote_state_summary(&[
                DpnsCurrentVoteState::Available(None),
                DpnsCurrentVoteState::Unavailable,
            ]),
            MasternodeVoteStateSummary::Unavailable
        );
    }

    #[test]
    fn dismissed_failure_does_not_hide_unresolved_sibling() {
        let temp = tempfile::tempdir().unwrap();
        let context = test_app_context(temp.path());
        context.set_det_kv_override_for_test(empty_kv());
        let voter_id = Identifier::from([7; 32]);
        let targets = ["alice", "bob"].map(|name| crate::model::dpns_voting::DpnsVoteTarget {
            key: crate::model::dpns_voting::DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id,
                vote_poll_id: context.dpns_vote_poll_id(name).unwrap(),
            },
            voter_alias: None,
            contested_name: name.into(),
            requested_choice:
                dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(42),
        });
        let mut operation = AppContext::new_dpns_vote_operation(targets.into());
        operation.targets[0].status = DpnsVoteTargetStatus::Rejected;
        operation.targets[1].status = DpnsVoteTargetStatus::Unconfirmed;
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        assert!(
            context
                .masternode_contest_summary(Some(voter_id))
                .unwrap()
                .has_failed_scheduled_vote
        );
        context
            .remove_scheduled_dpns_vote(operation.id, &operation.targets[0].target.key)
            .unwrap();
        let summary = context.masternode_contest_summary(Some(voter_id)).unwrap();
        assert!(
            !summary.has_failed_scheduled_vote,
            "dismissed terminal history is not an actionable failure"
        );
        assert!(
            summary.has_scheduled_vote,
            "unresolved sibling remains visible"
        );
        assert_eq!(
            context
                .dpns_vote_operation(operation.id)
                .unwrap()
                .unwrap()
                .targets[0]
                .status,
            DpnsVoteTargetStatus::Rejected,
            "removal must retain the original result"
        );
    }

    #[test]
    fn terminal_schedule_failure_is_not_reported_as_pending() {
        let voter_id = Identifier::from([7; 32]);
        let mut operation = AppContext::new_dpns_vote_operation(vec![
            crate::model::dpns_voting::DpnsVoteTarget {
                key: crate::model::dpns_voting::DpnsVoteTargetKey {
                    network: Network::Testnet,
                    voter_id,
                    vote_poll_id: Identifier::from([8; 32]),
                },
                voter_alias: Some("Eve".to_owned()),
                contested_name: "dominguez".to_owned(),
                requested_choice: dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Lock,
                current_choice: None,
                timing: VoteTiming::Scheduled(42),
            },
        ]);
        for status in [
            DpnsVoteTargetStatus::FailedBeforeSubmission,
            DpnsVoteTargetStatus::Rejected,
            DpnsVoteTargetStatus::NotApplied,
        ] {
            operation.targets[0].status = status;
            assert_eq!(
                scheduled_vote_journal_summary(
                    std::slice::from_ref(&operation),
                    voter_id,
                    &BTreeSet::new()
                ),
                (false, true),
                "{status:?} must remain actionable"
            );
            let dismissed =
                BTreeSet::from([(operation.id, operation.targets[0].target.key.clone())]);
            assert_eq!(
                scheduled_vote_journal_summary(
                    std::slice::from_ref(&operation),
                    voter_id,
                    &dismissed
                ),
                (false, false)
            );
        }
    }

    #[test]
    fn cancelled_schedule_is_not_reported_as_failed() {
        let voter_id = Identifier::from([7; 32]);
        let mut operation = AppContext::new_dpns_vote_operation(vec![
            crate::model::dpns_voting::DpnsVoteTarget {
                key: crate::model::dpns_voting::DpnsVoteTargetKey {
                    network: Network::Testnet,
                    voter_id,
                    vote_poll_id: Identifier::from([8; 32]),
                },
                voter_alias: Some("Eve".to_owned()),
                contested_name: "dominguez".to_owned(),
                requested_choice: dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Lock,
                current_choice: None,
                timing: VoteTiming::Scheduled(42),
            },
        ]);
        operation.targets[0].status = DpnsVoteTargetStatus::Cancelled;

        assert_eq!(
            scheduled_vote_journal_summary(&[operation], voter_id, &BTreeSet::new()),
            (false, false)
        );
    }

    #[test]
    fn later_success_supersedes_historical_schedule_failure() {
        let voter_id = Identifier::from([7; 32]);
        let target = crate::model::dpns_voting::DpnsVoteTarget {
            key: crate::model::dpns_voting::DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id,
                vote_poll_id: Identifier::from([8; 32]),
            },
            voter_alias: Some("Eve".to_owned()),
            contested_name: "dominguez".to_owned(),
            requested_choice:
                dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(42),
        };
        let mut failed = AppContext::new_dpns_vote_operation(vec![target.clone()]);
        failed.created_at = 1;
        failed.targets[0].status = DpnsVoteTargetStatus::Rejected;
        let mut confirmed = AppContext::new_dpns_vote_operation(vec![target]);
        confirmed.created_at = 2;
        confirmed.targets[0].status = DpnsVoteTargetStatus::Confirmed;

        assert_eq!(
            scheduled_vote_journal_summary(&[failed, confirmed], voter_id, &BTreeSet::new()),
            (false, false)
        );
    }

    /// Counts reads of the keys a summary pass touches, so a test can prove how
    /// many loads a caller performs. `snapshot_reads` counts the per-node
    /// current-vote snapshot (`current_votes_key`); `shared_reads` counts the
    /// contest cache and operation journal, which do not depend on the node.
    #[derive(Default)]
    struct CurrentVotesReadCounter {
        inner: InMemoryKv,
        snapshot_reads: AtomicUsize,
        shared_reads: AtomicUsize,
    }

    impl KvStore for CurrentVotesReadCounter {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            if key.starts_with("det:dpns_current_votes:v3:") {
                self.snapshot_reads.fetch_add(1, Ordering::Relaxed);
            }
            if key.starts_with(CONTESTED_NAME_KEY_PREFIX)
                || key.starts_with("det:dpns_vote_operation:v2:")
            {
                self.shared_reads.fetch_add(1, Ordering::Relaxed);
            }
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

    /// VOTE-NFR-007 / VOTE-TC-005: the masternode-card summary reads a node's
    /// proved current-vote snapshot once, not once per open contest.
    #[test]
    fn masternode_summary_reads_current_votes_once_per_node() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(CurrentVotesReadCounter::default());
        let kv = DetKv::from_store(store.clone());
        let context = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv.clone());

        let voter = Identifier::from([1; 32]);

        // Seed several open (Ongoing) contests: a contestant dated in the deep
        // past pushes each contest past the joinable half-window, and a missing
        // `end_time` keeps it in the ongoing set.
        for name in ["alice", "bob", "carol", "dave", "erin"] {
            let stored = StoredContestedName {
                normalized_contested_name: name.to_string(),
                contestants: vec![contestant(1, Some(1))],
                ..Default::default()
            };
            kv.put(DetScope::Global, &contested_name_key(name), &stored)
                .unwrap();
        }

        // Seed a proved snapshot so each summary read decodes a real record.
        context
            .cache_confirmed_dpns_vote(
                voter,
                Identifier::from([9; 32]),
                dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Abstain,
            )
            .expect("seed current-vote snapshot");

        let before = store.snapshot_reads.load(Ordering::Relaxed);
        let summary = context
            .masternode_contest_summary(Some(voter))
            .expect("summary");
        let reads = store.snapshot_reads.load(Ordering::Relaxed) - before;

        assert_eq!(summary.open_contest_count, 5);
        assert_eq!(
            reads, 1,
            "expected one snapshot read per node, got {reads} for 5 open contests"
        );
    }

    /// The contest cache and the operation journal do not depend on the node, so
    /// summarising a fleet must read them once — not once per card.
    #[test]
    fn masternode_summaries_read_node_independent_state_once_for_the_whole_fleet() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(CurrentVotesReadCounter::default());
        let kv = DetKv::from_store(store.clone());
        let context = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv.clone());

        for name in ["alice", "bob", "carol"] {
            let stored = StoredContestedName {
                normalized_contested_name: name.to_string(),
                contestants: vec![contestant(1, Some(1))],
                ..Default::default()
            };
            kv.put(DetScope::Global, &contested_name_key(name), &stored)
                .unwrap();
        }
        let mut operation =
            AppContext::new_dpns_vote_operation(vec![crate::model::dpns_voting::DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: context.dpns_vote_poll_id("alice").unwrap(),
            },
            voter_alias: None,
            contested_name: "alice".into(),
            requested_choice:
                dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(42),
        }]);
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();

        let voters: Vec<Identifier> = (1..=8).map(|byte| Identifier::from([byte; 32])).collect();
        let before = store.shared_reads.load(Ordering::Relaxed);
        let summaries = context
            .masternode_contest_summaries(&voters)
            .expect("fleet summaries");
        let reads = store.shared_reads.load(Ordering::Relaxed) - before;

        assert_eq!(summaries.len(), voters.len());
        assert_eq!(
            reads,
            4,
            "expected one read of each of the 3 contests and 1 journal record for the whole \
             fleet, got {reads} for {} nodes",
            voters.len()
        );
    }
}
