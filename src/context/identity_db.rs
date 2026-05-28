use super::AppContext;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use rusqlite::Result;
use serde::{Deserialize, Serialize};

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
    /// Inserts a local qualified identity into the database
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletSeedHash, u32)>,
    ) -> Result<()> {
        self.db.insert_local_qualified_identity(
            qualified_identity,
            wallet_and_identity_id_info,
            self,
        )
    }

    /// Updates a local qualified identity in the database
    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> Result<()> {
        self.db
            .update_local_qualified_identity(qualified_identity, self)
    }

    /// Sets the alias for an identity
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> Result<()> {
        self.db.set_identity_alias(identifier, new_alias)
    }

    /// Gets the alias for an identity
    pub fn get_identity_alias(&self, identifier: &Identifier) -> Result<Option<String>> {
        self.db.get_identity_alias(identifier)
    }

    /// Fetches all local qualified identities from the database, then
    /// hydrates each identity's top-up history from the per-network
    /// wallet k/v store.
    pub fn load_local_qualified_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut identities = self.db.get_local_qualified_identities(self, &wallets)?;
        for identity in &mut identities {
            self.hydrate_top_ups(identity);
        }
        Ok(identities)
    }

    /// Same as [`Self::load_local_qualified_identities`] but filters to
    /// identities associated with a wallet.
    #[allow(dead_code)] // May be used for loading identities in wallets
    pub fn load_local_qualified_identities_in_wallets(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut identities = self
            .db
            .get_local_qualified_identities_in_wallets(self, &wallets)?;
        for identity in &mut identities {
            self.hydrate_top_ups(identity);
        }
        Ok(identities)
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
    ) -> std::result::Result<(), crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        backend
            .kv()
            .put(None, &top_ups_key(identity_id), top_ups)
            .map_err(|source| crate::backend_task::error::TaskError::TopUpHistoryStorage { source })
    }

    pub fn get_identity_by_id(
        &self,
        identity_id: &Identifier,
    ) -> Result<Option<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let mut result = self.db.get_identity_by_id(identity_id, self, &wallets)?;
        if let Some(ref mut identity) = result {
            self.hydrate_top_ups(identity);
        }
        Ok(result)
    }

    /// Fetches all voting identities from the database
    pub fn load_local_voting_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        self.db.get_local_voting_identities(self)
    }

    /// Fetches all local user identities from the database
    pub fn load_local_user_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let identities = self.db.get_local_user_identities(self)?;

        Ok(identities
            .into_iter()
            .map(|(mut identity, wallet_hash)| {
                if let Some(wallet_id) = wallet_hash {
                    // Load wallets for each identity
                    self.load_wallet_for_identity(
                        &mut identity,
                        &[wallet_id],
                    )
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            identity = %identity.identity.id(),
                            error = ?e,
                            "cannot load wallet for identity when loading local user identities",
                        )
                    })
                } else {
                    tracing::debug!(
                        identity = %identity.identity.id(),
                        "no wallet hash found for identity when loading local user identities",
                    );
                }
                identity
            })
            .collect())
    }

    fn load_wallet_for_identity(
        &self,
        identity: &mut QualifiedIdentity,
        wallet_hashes: &[WalletSeedHash],
    ) -> Result<()> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        for wallet_hash in wallet_hashes {
            if let Some(wallet) = wallets.get(wallet_hash) {
                identity
                    .associated_wallets
                    .insert(*wallet_hash, wallet.clone());
            } else {
                tracing::warn!(
                    wallet = %hex::encode(wallet_hash),
                    identity = %identity.identity.id(),
                    "wallet not found for identity when loading local user identities",
                );
            }
        }

        Ok(())
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

    /// Fetches the local identities from the database and then maps them to their DPNS names.
    pub fn local_dpns_names(&self) -> Result<Vec<(Identifier, DPNSNameInfo)>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let qualified_identities = self.db.get_local_qualified_identities(self, &wallets)?;

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
