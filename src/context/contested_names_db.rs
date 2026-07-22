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
    ContestState, Contestant, ContestedName, PendingUsername, pending_usernames_in,
};
use crate::model::qualified_identity::QualifiedIdentity;
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
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// Key prefix for DPNS contest cache entries in the per-network wallet
/// k/v store. Each contest occupies one record at
/// `det:contested_name:<normalized_name>` in the global scope.
const CONTESTED_NAME_KEY_PREFIX: &str = "det:contested_name:";

fn contested_name_key(normalized_name: &str) -> String {
    format!("{CONTESTED_NAME_KEY_PREFIX}{normalized_name}")
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

/// How long a DPNS name contest stays open before it locks or resolves.
/// Mainnet runs the two-week production window; every other network uses a
/// 90-minute fast-cycle window for testing. Mirrors platform's DPNS
/// contested-name governance parameters (`ACTIVE_VOTE_DURATION`).
const MAINNET_CONTEST_DURATION: Duration = Duration::from_secs(60 * 60 * 24 * 14);
const NON_MAINNET_CONTEST_DURATION: Duration = Duration::from_secs(60 * 90);

fn contest_duration_for_network(network: Network) -> Duration {
    if network == Network::Mainnet {
        MAINNET_CONTEST_DURATION
    } else {
        NON_MAINNET_CONTEST_DURATION
    }
}

impl StoredContestedName {
    fn to_contested_name(&self, network: Network) -> ContestedName {
        let contest_duration = contest_duration_for_network(network);
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
            let elapsed = Duration::from_millis(
                (std::time::UNIX_EPOCH
                    .elapsed()
                    .unwrap_or_default()
                    .as_millis() as u64)
                    .saturating_sub(created_at),
            );
            // New contenders may join only during the first half of the
            // contest window; the second half is vote-only.
            if elapsed <= contest_duration / 2 {
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

impl AppContext {
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
        let current_timestamp = std::time::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_millis() as u64;
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

    /// Rebuild the frame-safe pending-name snapshot from the contest store.
    pub(crate) fn refresh_pending_dpns_usernames(&self) -> Result<(), TaskError> {
        let contests = self.ongoing_contested_names()?;
        let pending = pending_usernames_in(&contests);
        *self
            .pending_dpns_usernames
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = pending;
        Ok(())
    }

    /// The DPNS username `identity_id` has requested but not yet been awarded,
    /// if any — read from the frame-safe snapshot.
    ///
    /// Read-only; returns `Ok(None)` when nothing is pending. Lets the UI tell
    /// "requested but still being decided" apart from "no username requested".
    pub fn pending_dpns_username_for(
        &self,
        identity_id: &Identifier,
    ) -> std::result::Result<Option<PendingUsername>, TaskError> {
        let now_ms = std::time::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_millis() as u64;
        Ok(self
            .pending_dpns_usernames
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(identity_id)
            .filter(|pending| pending.decided_at.is_none_or(|end| end > now_ms))
            .cloned())
    }

    /// Map each of `identity_ids` to its pending DPNS username request, if any.
    ///
    /// Reads only the frame-safe snapshot. Identities with nothing pending are
    /// omitted.
    pub fn pending_dpns_usernames(
        &self,
        identity_ids: &[Identifier],
    ) -> std::result::Result<HashMap<Identifier, PendingUsername>, TaskError> {
        let now_ms = std::time::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_millis() as u64;
        let pending = self
            .pending_dpns_usernames
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(identity_ids
            .iter()
            .filter_map(|id| {
                pending
                    .get(id)
                    .filter(|name| name.decided_at.is_none_or(|end| end > now_ms))
                    .cloned()
                    .map(|name| (*id, name))
            })
            .collect())
    }

    /// Return a pending DPNS username only while `identity` owns no awarded name.
    pub fn pending_dpns_username_for_identity(
        &self,
        identity: &QualifiedIdentity,
    ) -> Option<PendingUsername> {
        if identity_owns_dpns_name(identity) {
            None
        } else {
            self.pending_dpns_username_for(&identity.identity.id())
                .ok()
                .flatten()
        }
    }

    /// Map identities without an awarded name to their pending DPNS usernames.
    pub fn pending_dpns_usernames_for_identities(
        &self,
        identities: &[QualifiedIdentity],
    ) -> HashMap<Identifier, PendingUsername> {
        let identity_ids = identities
            .iter()
            .filter(|identity| !identity_owns_dpns_name(identity))
            .map(|identity| identity.identity.id())
            .collect::<Vec<_>>();
        self.pending_dpns_usernames(&identity_ids)
            .unwrap_or_default()
    }

    /// Summarise a masternode/evonode node's DPNS voting position for its card.
    ///
    /// `voter_id` is the node's voter-identity id (`associated_voter_identity`);
    /// pass `None` for a node with no voting key loaded — it can vote on
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

        let open_contest_count = self
            .ongoing_contested_names()?
            .iter()
            .filter(|contest| contest.is_open_for_voter(&voter_id))
            .count();

        let has_scheduled_vote = self
            .get_scheduled_votes()?
            .iter()
            .any(|vote| vote.voter_id == voter_id && !vote.executed_successfully);

        Ok(crate::model::contested_name::MasternodeContestSummary {
            open_contest_count,
            has_scheduled_vote,
        })
    }

    /// Apply a batch of newly-seen normalized names. New names are stored
    /// as empty contest skeletons; existing names whose `last_updated` is
    /// older than 30 s are returned alongside new names for the caller to
    /// refresh — matching pre-C6 staleness gating.
    pub fn insert_name_contests_as_normalized_names(
        &self,
        name_contests: Vec<String>,
    ) -> std::result::Result<Vec<String>, TaskError> {
        let kv = self.det_kv()?;
        let stale_threshold = chrono::Utc::now().timestamp() - 30;
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
        stale.sort_by(|a, b| a.1.unwrap_or(0).cmp(&b.1.unwrap_or(0)));
        Ok(stale.into_iter().map(|(name, _)| name).collect())
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
    use crate::model::contested_name::pending_username_in;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{
        DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
    };
    use crate::wallet_backend::DetKv;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::sync::Arc;

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

    #[test]
    fn pending_dpns_usernames_for_identities_omits_owned_identity() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(temp_dir.path());
        let owned = qualified_identity(1, "alice");
        let unowned = qualified_identity(2, "   ");
        {
            let mut pending = context
                .pending_dpns_usernames
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pending.insert(
                owned.identity.id(),
                PendingUsername {
                    name: "alice".to_string(),
                    decided_at: None,
                },
            );
            pending.insert(
                unowned.identity.id(),
                PendingUsername {
                    name: "bob".to_string(),
                    decided_at: None,
                },
            );
        }

        let pending = context.pending_dpns_usernames_for_identities(&[owned, unowned]);

        assert!(
            !pending.contains_key(&Identifier::from([1; 32])),
            "an awarded DPNS name must suppress the stale pending indicator"
        );
        assert_eq!(
            pending
                .get(&Identifier::from([2; 32]))
                .map(|name| name.name.as_str()),
            Some("bob"),
            "a blank DPNS entry is not ownership and must keep the pending indicator"
        );
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

    // ----------------------------------------------------------------
    // Bridge: a stored contest decodes into a detectable pending username.
    // ----------------------------------------------------------------

    #[test]
    fn stored_undecided_contest_yields_pending_username() {
        // A contestant with a timestamp and no winner resolves to an active
        // (Ongoing) state, so the identity has a pending username request.
        let stored = StoredContestedName {
            normalized_contested_name: "det1".to_string(),
            end_time: Some(9_999),
            contestants: vec![contestant(5, Some(100))],
            ..Default::default()
        };
        let cn = stored.to_contested_name(Network::Testnet);
        let me = Identifier::from([5u8; 32]);
        let pending = pending_username_in(std::slice::from_ref(&cn), &me)
            .expect("undecided contender must surface a pending username");
        assert_eq!(pending.name, "name-5");
        assert_eq!(pending.decided_at, Some(9_999));
    }

    #[test]
    fn stored_awarded_contest_yields_no_pending_username() {
        let stored = StoredContestedName {
            normalized_contested_name: "det1".to_string(),
            awarded_to: Some([5u8; 32]),
            contestants: vec![contestant(5, Some(100))],
            ..Default::default()
        };
        let cn = stored.to_contested_name(Network::Testnet);
        let me = Identifier::from([5u8; 32]);
        assert!(pending_username_in(std::slice::from_ref(&cn), &me).is_none());
    }
}
