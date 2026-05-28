use super::AppContext;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::{DPNSNameInfo, IdentityStatus, QualifiedIdentity};
use crate::model::wallet::{Wallet, WalletSeedHash};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Key prefix for local identity entries in the per-network wallet k/v
/// store. The full key is `det:identity:<base58_identity_id>` and lives
/// in the global (`None`) scope — identities are a network-scoped concept
/// inside the per-network k/v store.
const IDENTITY_KEY_PREFIX: &str = "det:identity:";

/// Versioned key for the user's custom identity ordering. Holds a single
/// `Vec<[u8; 32]>` of identity IDs in display order; bumping the version
/// suffix is a deliberate breaking change.
const IDENTITY_ORDER_KEY: &str = "det:identity_order:v1";

/// Key prefix for scheduled-vote entries in the per-network wallet k/v
/// store. The full key is `det:scheduled_vote:<base58_voter>:<contested_name>`
/// and lives in the global (`None`) scope — scheduling is a network-level
/// queue, not per-wallet.
const SCHEDULED_VOTE_KEY_PREFIX: &str = "det:scheduled_vote:";

/// Key prefix for top-up history blobs. The full key is
/// `det:top_ups:<base58_identity_id>` and lives in the global (`None`)
/// scope — top-up history is keyed by identity, which is itself a
/// network-scoped concept inside the per-network k/v store.
const TOP_UPS_KEY_PREFIX: &str = "det:top_ups:";

fn top_ups_key(identity_id: &Identifier) -> String {
    format!(
        "{}{}",
        TOP_UPS_KEY_PREFIX,
        identity_id.to_string(Encoding::Base58)
    )
}

fn identity_key(identity_id: &Identifier) -> String {
    format!(
        "{}{}",
        IDENTITY_KEY_PREFIX,
        identity_id.to_string(Encoding::Base58)
    )
}

/// Decode a stored bincode'd [`QualifiedIdentity`] blob, attaching the
/// active network. The encoder skips `status` / `network` (rehydrated by
/// callers) and `associated_wallets` / `top_ups` (filled by callers from
/// the wallet map and the top-up k/v entry).
fn decode_stored_identity(
    bytes: &[u8],
    network: Network,
) -> std::result::Result<QualifiedIdentity, TaskError> {
    let mut qi = QualifiedIdentity::from_bytes(bytes).map_err(|detail| {
        // `QualifiedIdentity::from_bytes` only fails when bincode decode
        // fails. Re-derive a typed `DecodeError` so the wrapper carries a
        // structured cause instead of a stringified one.
        TaskError::IdentityEncoding {
            source: bincode::error::DecodeError::OtherString(detail),
        }
    })?;
    qi.network = network;
    Ok(qi)
}

/// Persisted shape of a local identity entry. The [`QualifiedIdentity`]
/// itself is stored as its own bincode encoding (`to_bytes()`), with the
/// network-scoped metadata that the pre-C7 SQLite schema kept in dedicated
/// columns alongside. Fields that the encoder skips (status, network) are
/// rehydrated from the wrapper at read time.
#[derive(Debug, Serialize, Deserialize)]
struct StoredQualifiedIdentity {
    /// `QualifiedIdentity::to_bytes()` — bincode of the inner struct.
    /// Carries everything `Encode` writes: identity, alias, private keys,
    /// DPNS names, voter/operator associations.
    qi_bytes: Vec<u8>,
    /// Identity status (created/active/etc.). Held outside `qi_bytes`
    /// because the bincode shape deliberately omits status — matches the
    /// pre-C7 column-vs-blob split.
    status: u8,
    /// Identity type label (`User` / `Masternode` / `Evonode`). Stored
    /// alongside the blob so filter queries (voting, user-only) avoid a
    /// full decode pass.
    identity_type: String,
    /// Wallet seed-hash this identity was loaded from, if any. Mirrors
    /// the nullable `wallet` column the SQLite schema carried.
    wallet_hash: Option<[u8; 32]>,
    /// Account index within `wallet_hash`. `Some` iff `wallet_hash` is
    /// also `Some` (mirrors the pre-C7 `CHECK` constraint).
    wallet_index: Option<u32>,
}

fn scheduled_vote_key(voter_id: &Identifier, contested_name: &str) -> String {
    format!(
        "{}{}:{}",
        SCHEDULED_VOTE_KEY_PREFIX,
        voter_id.to_string(Encoding::Base58),
        contested_name
    )
}

/// Persisted shape of a scheduled DPNS vote. Mirrors
/// [`ScheduledDPNSVote`] but with a serde-friendly representation of
/// the SDK's [`ResourceVoteChoice`] (which only derives bincode under
/// this feature set).
#[derive(Debug, Serialize, Deserialize)]
struct StoredScheduledVote {
    voter_id: [u8; 32],
    contested_name: String,
    choice: StoredVoteChoice,
    unix_timestamp: u64,
    executed_successfully: bool,
}

#[derive(Debug, Serialize, Deserialize)]
enum StoredVoteChoice {
    TowardsIdentity([u8; 32]),
    Abstain,
    Lock,
}

impl From<&ScheduledDPNSVote> for StoredScheduledVote {
    fn from(v: &ScheduledDPNSVote) -> Self {
        Self {
            voter_id: v.voter_id.to_buffer(),
            contested_name: v.contested_name.clone(),
            choice: match v.choice {
                ResourceVoteChoice::TowardsIdentity(id) => {
                    StoredVoteChoice::TowardsIdentity(id.to_buffer())
                }
                ResourceVoteChoice::Abstain => StoredVoteChoice::Abstain,
                ResourceVoteChoice::Lock => StoredVoteChoice::Lock,
            },
            unix_timestamp: v.unix_timestamp,
            executed_successfully: v.executed_successfully,
        }
    }
}

impl From<StoredScheduledVote> for ScheduledDPNSVote {
    fn from(v: StoredScheduledVote) -> Self {
        ScheduledDPNSVote {
            voter_id: Identifier::from(v.voter_id),
            contested_name: v.contested_name,
            choice: match v.choice {
                StoredVoteChoice::TowardsIdentity(id) => {
                    ResourceVoteChoice::TowardsIdentity(Identifier::from(id))
                }
                StoredVoteChoice::Abstain => ResourceVoteChoice::Abstain,
                StoredVoteChoice::Lock => ResourceVoteChoice::Lock,
            },
            unix_timestamp: v.unix_timestamp,
            executed_successfully: v.executed_successfully,
        }
    }
}

impl AppContext {
    /// Insert (or replace) a local qualified identity in the per-network
    /// wallet k/v store at `det:identity:<base58_id>`. Mirrors pre-C7
    /// `INSERT OR REPLACE` semantics — wallet association is overwritten
    /// from the passed-in hint.
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletSeedHash, u32)>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.identity_kv()?;
        let (wallet_hash, wallet_index) = match wallet_and_identity_id_info {
            Some((seed, idx)) => (Some(*seed), Some(*idx)),
            None => {
                tracing::warn!(
                    identity_id = %qualified_identity.identity.id(),
                    alias = ?qualified_identity.alias,
                    "saving identity without wallet; this needs investigating",
                );
                (None, None)
            }
        };
        let stored = StoredQualifiedIdentity {
            qi_bytes: qualified_identity.to_bytes(),
            status: qualified_identity.status.as_u8(),
            identity_type: format!("{:?}", qualified_identity.identity_type),
            wallet_hash,
            wallet_index,
        };
        kv.put(
            None,
            &identity_key(&qualified_identity.identity.id()),
            &stored,
        )
        .map_err(|source| TaskError::IdentityStorage { source })
    }

    /// Update a local qualified identity in place. Wallet association
    /// (`wallet_hash` / `wallet_index`) is preserved from the existing
    /// record — pre-C7 `update_local_qualified_identity` had the same
    /// behaviour by virtue of omitting those columns from its `UPDATE`.
    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.identity_kv()?;
        let key = identity_key(&qualified_identity.identity.id());
        let existing: Option<StoredQualifiedIdentity> = kv
            .get(None, &key)
            .map_err(|source| TaskError::IdentityStorage { source })?;
        let (wallet_hash, wallet_index) = existing
            .as_ref()
            .map(|s| (s.wallet_hash, s.wallet_index))
            .unwrap_or((None, None));
        let stored = StoredQualifiedIdentity {
            qi_bytes: qualified_identity.to_bytes(),
            status: qualified_identity.status.as_u8(),
            identity_type: format!("{:?}", qualified_identity.identity_type),
            wallet_hash,
            wallet_index,
        };
        kv.put(None, &key, &stored)
            .map_err(|source| TaskError::IdentityStorage { source })
    }

    /// Update only the user-facing alias on a stored identity. Returns
    /// `Ok(())` when the identity is unknown — alias is metadata, not a
    /// load-bearing identifier.
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.identity_kv()?;
        let key = identity_key(identifier);
        let Some(mut stored) = kv
            .get::<StoredQualifiedIdentity>(None, &key)
            .map_err(|source| TaskError::IdentityStorage { source })?
        else {
            return Ok(());
        };
        let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        qi.alias = new_alias.map(str::to_string);
        stored.qi_bytes = qi.to_bytes();
        kv.put(None, &key, &stored)
            .map_err(|source| TaskError::IdentityStorage { source })
    }

    /// Read the user-facing alias for a stored identity, if any.
    pub fn get_identity_alias(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<Option<String>, TaskError> {
        let kv = self.identity_kv()?;
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(None, &identity_key(identifier))
            .map_err(|source| TaskError::IdentityStorage { source })?
        else {
            return Ok(None);
        };
        let qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        Ok(qi.alias)
    }

    /// Fetches all local qualified identities from the k/v store, then
    /// hydrates each identity's top-up history.
    ///
    /// Stops on the first corrupted identity blob and returns an error.
    /// This is intentional — identities hold private keys and balance data,
    /// so skipping a corrupted entry could cause loss of funds.
    pub fn load_local_qualified_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut identities = self.load_identities_filtered(&wallets, |_| true)?;
        for identity in &mut identities {
            self.hydrate_top_ups(identity);
        }
        Ok(identities)
    }

    /// Same as [`Self::load_local_qualified_identities`] but filters to
    /// identities associated with a wallet.
    #[allow(dead_code)] // May be used for loading identities in wallets
    pub fn load_local_qualified_identities_in_wallets(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut identities =
            self.load_identities_filtered(&wallets, |s| s.wallet_index.is_some())?;
        for identity in &mut identities {
            self.hydrate_top_ups(identity);
        }
        Ok(identities)
    }

    /// Internal: list every stored identity, decode it, rehydrate the
    /// metadata kept outside the bincode blob, and apply `keep` as a
    /// pre-decode filter on the wrapper. Sorted by identity ID for
    /// deterministic output — mirrors the pre-C7 `ORDER BY id`.
    fn load_identities_filtered<F>(
        &self,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
        keep: F,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError>
    where
        F: Fn(&StoredQualifiedIdentity) -> bool,
    {
        let kv = self.identity_kv()?;
        let mut keys = kv
            .list(None, Some(IDENTITY_KEY_PREFIX))
            .map_err(|source| TaskError::IdentityStorage { source })?;
        keys.sort();
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(stored) = kv
                .get::<StoredQualifiedIdentity>(None, &key)
                .map_err(|source| TaskError::IdentityStorage { source })?
            else {
                continue;
            };
            if !keep(&stored) {
                continue;
            }
            let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
            qi.status = IdentityStatus::from_u8(stored.status);
            qi.wallet_index = stored.wallet_index;
            qi.network = self.network;
            qi.associated_wallets = wallets.clone();
            qi.top_ups = BTreeMap::new();
            out.push(qi);
        }
        Ok(out)
    }

    /// Populate `identity.top_ups` from the per-network wallet k/v
    /// store. A missing or unreadable entry is logged and treated as an
    /// empty map; pre-C5 SQLite data is intentionally not migrated and
    /// surfaces as empty under the "empty start" policy.
    fn hydrate_top_ups(&self, identity: &mut QualifiedIdentity) {
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let key = top_ups_key(&identity.identity.id());
        match backend
            .kv()
            .get::<std::collections::BTreeMap<u32, u64>>(None, &key)
        {
            Ok(Some(map)) => identity.top_ups = map,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    identity = %identity.identity.id(),
                    error = ?e,
                    "Failed to load top-up history from wallet k/v"
                );
            }
        }
    }

    /// Persist the running top-up history for an identity into the
    /// per-network wallet k/v store.
    pub fn save_top_ups(
        &self,
        identity_id: &Identifier,
        top_ups: &std::collections::BTreeMap<u32, u64>,
    ) -> std::result::Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        backend
            .kv()
            .put(None, &top_ups_key(identity_id), top_ups)
            .map_err(|source| TaskError::TopUpHistoryStorage { source })
    }

    pub fn get_identity_by_id(
        &self,
        identity_id: &Identifier,
    ) -> std::result::Result<Option<QualifiedIdentity>, TaskError> {
        let kv = self.identity_kv()?;
        let Some(stored) = kv
            .get::<StoredQualifiedIdentity>(None, &identity_key(identity_id))
            .map_err(|source| TaskError::IdentityStorage { source })?
        else {
            return Ok(None);
        };
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
        qi.status = IdentityStatus::from_u8(stored.status);
        qi.wallet_index = stored.wallet_index;
        qi.network = self.network;
        qi.associated_wallets = wallets.clone();
        qi.top_ups = BTreeMap::new();
        self.hydrate_top_ups(&mut qi);
        Ok(Some(qi))
    }

    /// Fetches every locally-stored identity whose `identity_type` is
    /// not `User` — used by the DPNS contest voting flows.
    pub fn load_local_voting_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        self.load_identities_filtered(&wallets, |s| s.identity_type != "User")
    }

    /// Fetches every locally-stored identity whose `identity_type` is
    /// `User`. Top-up history is *not* loaded here — matching the
    /// pre-C7 query shape that the consumer screens depend on.
    pub fn load_local_user_identities(
        &self,
    ) -> std::result::Result<Vec<QualifiedIdentity>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        self.load_identities_filtered(&wallets, |s| s.identity_type == "User")
    }

    /// Remove a locally-stored identity. Returns `Ok(())` even when the
    /// identity is unknown — mirrors the pre-C7 `DELETE` which silently
    /// no-ops on missing rows.
    pub fn delete_local_qualified_identity(
        &self,
        identifier: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.identity_kv()?;
        kv.delete(None, &identity_key(identifier))
            .map_err(|source| TaskError::IdentityStorage { source })
    }

    /// Devnet-only sweep: drop every locally-stored identity for the
    /// current network. Matches the pre-C7
    /// `delete_all_local_qualified_identities_in_devnet` guard — no-op on
    /// non-devnet networks.
    pub fn delete_all_local_qualified_identities_in_devnet(
        &self,
    ) -> std::result::Result<(), TaskError> {
        if self.network != Network::Devnet {
            return Ok(());
        }
        let kv = self.identity_kv()?;
        let keys = kv
            .list(None, Some(IDENTITY_KEY_PREFIX))
            .map_err(|source| TaskError::IdentityStorage { source })?;
        for key in keys {
            kv.delete(None, &key)
                .map_err(|source| TaskError::IdentityStorage { source })?;
        }
        Ok(())
    }

    /// Persist the user-chosen identity ordering at `det:identity_order:v1`.
    /// Overwrites the previous list — matches pre-C7 semantics.
    pub fn save_identity_order(
        &self,
        all_ids: Vec<Identifier>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.identity_kv()?;
        let payload: Vec<[u8; 32]> = all_ids.iter().map(Identifier::to_buffer).collect();
        kv.put(None, IDENTITY_ORDER_KEY, &payload)
            .map_err(|source| TaskError::IdentityStorage { source })
    }

    /// Load the user-chosen identity ordering, dropping any references
    /// that no longer point at a stored identity.
    pub fn load_identity_order(&self) -> std::result::Result<Vec<Identifier>, TaskError> {
        let kv = self.identity_kv()?;
        let Some(payload): Option<Vec<[u8; 32]>> = kv
            .get(None, IDENTITY_ORDER_KEY)
            .map_err(|source| TaskError::IdentityStorage { source })?
        else {
            return Ok(Vec::new());
        };
        let mut kept = Vec::with_capacity(payload.len());
        let mut needs_rewrite = false;
        for buf in payload {
            let id = Identifier::from(buf);
            let exists = kv
                .get::<StoredQualifiedIdentity>(None, &identity_key(&id))
                .map_err(|source| TaskError::IdentityStorage { source })?
                .is_some();
            if exists {
                kept.push(id);
            } else {
                needs_rewrite = true;
            }
        }
        if needs_rewrite {
            let payload: Vec<[u8; 32]> = kept.iter().map(Identifier::to_buffer).collect();
            kv.put(None, IDENTITY_ORDER_KEY, &payload)
                .map_err(|source| TaskError::IdentityStorage { source })?;
        }
        Ok(kept)
    }

    fn identity_kv(&self) -> std::result::Result<crate::wallet_backend::DetKv, TaskError> {
        Ok(self.wallet_backend()?.kv())
    }

    /// Persist a batch of scheduled votes in the per-network wallet
    /// k/v store. Existing entries with the same `(voter_id,
    /// contested_name)` key are overwritten — matching the pre-C5
    /// `INSERT OR REPLACE` semantics.
    pub fn insert_scheduled_votes(
        &self,
        scheduled_votes: &Vec<ScheduledDPNSVote>,
    ) -> std::result::Result<(), crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        for vote in scheduled_votes {
            let stored = StoredScheduledVote::from(vote);
            kv.put(
                None,
                &scheduled_vote_key(&vote.voter_id, &vote.contested_name),
                &stored,
            )
            .map_err(|source| {
                crate::backend_task::error::TaskError::ScheduledVoteStorage { source }
            })?;
        }
        Ok(())
    }

    /// Fetch every scheduled vote queued for this network from the
    /// wallet k/v store.
    pub fn get_scheduled_votes(
        &self,
    ) -> std::result::Result<Vec<ScheduledDPNSVote>, crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(None, Some(SCHEDULED_VOTE_KEY_PREFIX))
            .map_err(
                |source| crate::backend_task::error::TaskError::ScheduledVoteStorage { source },
            )?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            match kv.get::<StoredScheduledVote>(None, &key) {
                Ok(Some(stored)) => out.push(stored.into()),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        key = %key,
                        error = ?e,
                        "Skipping unreadable scheduled vote entry"
                    );
                }
            }
        }
        Ok(out)
    }

    /// Drop every scheduled vote queued for this network.
    pub fn clear_all_scheduled_votes(
        &self,
    ) -> std::result::Result<(), crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(None, Some(SCHEDULED_VOTE_KEY_PREFIX))
            .map_err(
                |source| crate::backend_task::error::TaskError::ScheduledVoteStorage { source },
            )?;
        for key in keys {
            kv.delete(None, &key).map_err(|source| {
                crate::backend_task::error::TaskError::ScheduledVoteStorage { source }
            })?;
        }
        Ok(())
    }

    /// Drop every scheduled vote that has already been cast successfully.
    pub fn clear_executed_scheduled_votes(
        &self,
    ) -> std::result::Result<(), crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(None, Some(SCHEDULED_VOTE_KEY_PREFIX))
            .map_err(
                |source| crate::backend_task::error::TaskError::ScheduledVoteStorage { source },
            )?;
        for key in keys {
            match kv.get::<StoredScheduledVote>(None, &key) {
                Ok(Some(stored)) if stored.executed_successfully => {
                    kv.delete(None, &key).map_err(|source| {
                        crate::backend_task::error::TaskError::ScheduledVoteStorage { source }
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Drop a single scheduled vote keyed by `(voter_id, contested_name)`.
    #[allow(clippy::ptr_arg)]
    pub fn delete_scheduled_vote(
        &self,
        identity_id: &[u8],
        contested_name: &String,
    ) -> std::result::Result<(), crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let voter_id = Identifier::from_bytes(identity_id).map_err(|e| {
            crate::backend_task::error::TaskError::SerializationError {
                detail: format!("Invalid voter identifier in scheduled-vote operation: {e}"),
            }
        })?;
        backend
            .kv()
            .delete(None, &scheduled_vote_key(&voter_id, contested_name))
            .map_err(
                |source| crate::backend_task::error::TaskError::ScheduledVoteStorage { source },
            )
    }

    /// Mark a single scheduled vote as executed so future cast loops skip it.
    pub fn mark_vote_executed(
        &self,
        identity_id: &[u8],
        contested_name: String,
    ) -> std::result::Result<(), crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let voter_id = Identifier::from_bytes(identity_id).map_err(|e| {
            crate::backend_task::error::TaskError::SerializationError {
                detail: format!("Invalid voter identifier in scheduled-vote operation: {e}"),
            }
        })?;
        let key = scheduled_vote_key(&voter_id, &contested_name);
        let kv = backend.kv();
        let Some(mut stored): Option<StoredScheduledVote> =
            kv.get(None, &key).map_err(|source| {
                crate::backend_task::error::TaskError::ScheduledVoteStorage { source }
            })?
        else {
            return Ok(());
        };
        stored.executed_successfully = true;
        kv.put(None, &key, &stored).map_err(|source| {
            crate::backend_task::error::TaskError::ScheduledVoteStorage { source }
        })
    }

    /// Fetches the local identities from the k/v store and maps them to their DPNS names.
    pub fn local_dpns_names(
        &self,
    ) -> std::result::Result<Vec<(Identifier, DPNSNameInfo)>, TaskError> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let qualified_identities = self.load_identities_filtered(&wallets, |_| true)?;

        // Map each identity's DPNS names to (Identifier, DPNSNameInfo) tuples
        let dpns_names = qualified_identities
            .iter()
            .flat_map(|qualified_identity| {
                qualified_identity.dpns_names.iter().map(|dpns_name_info| {
                    (
                        qualified_identity.identity.id(),
                        DPNSNameInfo {
                            name: dpns_name_info.name.clone(),
                            acquired_at: dpns_name_info.acquired_at,
                        },
                    )
                })
            })
            .collect::<Vec<(Identifier, DPNSNameInfo)>>();

        Ok(dpns_names)
    }
}
