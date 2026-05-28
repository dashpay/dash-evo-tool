//! DPNS contest cache, persisted in the per-network wallet k/v store.
//!
//! One [`StoredContestedName`] entry per normalized name, keyed at
//! `det:contested_name:<normalized_name>` in the global scope (contests
//! are network-scoped, not per-wallet). Contender rows are nested inside
//! the record so a single k/v read returns the whole contest — there is
//! no relational join.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::contested_name::{ContestState, Contestant, ContestedName};
use crate::wallet_backend::DetKv;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::TimestampMillis;
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

fn contest_duration_for_network(network: Network) -> Duration {
    if network == Network::Mainnet {
        Duration::from_secs(60 * 60 * 24 * 14)
    } else {
        Duration::from_secs(60 * 90)
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

impl AppContext {
    /// Fetches every DPNS contest cached in the per-network k/v store.
    pub fn all_contested_names(&self) -> std::result::Result<Vec<ContestedName>, TaskError> {
        let kv = self.contest_kv()?;
        let keys = kv
            .list(None, Some(CONTESTED_NAME_KEY_PREFIX))
            .map_err(|source| TaskError::ContestStorage { source })?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            match kv.get::<StoredContestedName>(None, &key) {
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
        let kv = self.contest_kv()?;
        let keys = kv
            .list(None, Some(CONTESTED_NAME_KEY_PREFIX))
            .map_err(|source| TaskError::ContestStorage { source })?;
        let mut out = Vec::new();
        for key in keys {
            match kv.get::<StoredContestedName>(None, &key) {
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

    /// Apply a batch of newly-seen normalized names. New names are stored
    /// as empty contest skeletons; existing names whose `last_updated` is
    /// older than 30 s are returned alongside new names for the caller to
    /// refresh — matching pre-C6 staleness gating.
    pub fn insert_name_contests_as_normalized_names(
        &self,
        name_contests: Vec<String>,
    ) -> std::result::Result<Vec<String>, TaskError> {
        let kv = self.contest_kv()?;
        let stale_threshold = chrono::Utc::now().timestamp() - 30;
        let mut new_names: Vec<String> = Vec::new();
        let mut stale: Vec<(String, Option<i64>)> = Vec::new();

        for name in name_contests {
            let key = contested_name_key(&name);
            match kv
                .get::<StoredContestedName>(None, &key)
                .map_err(|source| TaskError::ContestStorage { source })?
            {
                None => {
                    let stored = StoredContestedName {
                        normalized_contested_name: name.clone(),
                        ..Default::default()
                    };
                    kv.put(None, &key, &stored)
                        .map_err(|source| TaskError::ContestStorage { source })?;
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
        let kv = self.contest_kv()?;
        let key = contested_name_key(normalized_contested_name);
        let last_updated = chrono::Utc::now().timestamp() as u64;

        let mut stored = kv
            .get::<StoredContestedName>(None, &key)
            .map_err(|source| TaskError::ContestStorage { source })?
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
                    kv.put(None, &key, &stored)
                        .map_err(|source| TaskError::ContestStorage { source })?;
                }
                ContestedDocumentVotePollWinnerInfo::Locked => {
                    stored.locked = true;
                    stored.last_updated = Some(last_updated);
                    stored.end_time = Some(block_info.time_ms);
                    kv.put(None, &key, &stored)
                        .map_err(|source| TaskError::ContestStorage { source })?;
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

        kv.put(None, &key, &stored)
            .map_err(|source| TaskError::ContestStorage { source })?;
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
        let kv = self.contest_kv()?;
        for (name, new_end_time) in name_contests {
            let key = contested_name_key(&name);
            let Some(mut stored) = kv
                .get::<StoredContestedName>(None, &key)
                .map_err(|source| TaskError::ContestStorage { source })?
            else {
                continue;
            };
            match stored.end_time {
                Some(t) if t >= new_end_time => continue,
                _ => {
                    stored.end_time = Some(new_end_time);
                    kv.put(None, &key, &stored)
                        .map_err(|source| TaskError::ContestStorage { source })?;
                }
            }
        }
        Ok(())
    }

    fn contest_kv(&self) -> std::result::Result<DetKv, TaskError> {
        let backend = self.wallet_backend()?;
        Ok(backend.kv())
    }
}
