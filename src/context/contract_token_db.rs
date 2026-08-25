use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::qualified_contract::{InsertTokensToo, QualifiedContract};
use crate::ui::tokens::tokens_screen::{
    IdentityTokenBalance, IdentityTokenIdentifier, TokenInfo, TokenInfoWithDataContract,
};
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError, UpstreamTokenBalances};
use bincode::config;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::serialization::{
    PlatformDeserializableWithPotentialValidationFromVersionedStructure,
    PlatformSerializableWithPlatformVersion,
};
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::query_types::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Key prefix for user-registered contract entries in the per-network
/// wallet k/v store. The full key is `det:contract:<contract_id_base58>`
/// and lives in the global (`None`) scope — contracts are a
/// network-scoped concept inside the per-network k/v store.
const CONTRACT_KEY_PREFIX: &str = "det:contract:";

/// Key prefix for the local token registry. One entry per known token at
/// `det:token:<base58_token_id>`. Lives in the global (`None`) scope —
/// tokens are network-scoped via the surrounding per-network k/v store.
const TOKEN_KEY_PREFIX: &str = "det:token:";

/// Versioned key for the user-chosen ordering of `(token_id, identity_id)`
/// pairs in the My Tokens screen. Per-network (each network has its own
/// k/v store, so each holds its own ordering).
const TOKEN_ORDER_KEY: &str = "det:token_order:v1";

/// Key prefix for the `(token_id, identity_id)` pairs the user stopped
/// tracking. Upstream's token watch set is in-memory only, so the dismissal is
/// persisted here and re-applied every time DET rebuilds a watch set.
///
/// One presence marker per pair at
/// `det:token_untracked:v2:<token_id_base58>:<identity_id_base58>`; the value is
/// empty, only the key carries meaning. Dismiss and re-track run as independent
/// backend tasks that can overlap, so a marker per pair (rather than one
/// set-valued blob) keeps each mutation a single self-contained write that no
/// concurrent mutation can read stale and clobber. Token id leads, so dropping
/// every dismissal of one token is a single prefix scan.
const TOKEN_UNTRACKED_PREFIX: &str = "det:token_untracked:v2:";

/// Marker key for one dismissed `(token, identity)` pair.
fn untracked_key(pair: &IdentityTokenIdentifier) -> String {
    format!(
        "{}{}",
        untracked_token_prefix(&pair.token_id),
        pair.identity_id.to_string(Encoding::Base58)
    )
}

/// Key prefix covering every identity that dismissed `token_id`.
fn untracked_token_prefix(token_id: &Identifier) -> String {
    format!(
        "{}{}:",
        TOKEN_UNTRACKED_PREFIX,
        token_id.to_string(Encoding::Base58)
    )
}

fn contract_key(contract_id: &Identifier) -> String {
    format!(
        "{}{}",
        CONTRACT_KEY_PREFIX,
        contract_id.to_string(Encoding::Base58)
    )
}

fn token_key(token_id: &Identifier) -> String {
    format!(
        "{}{}",
        TOKEN_KEY_PREFIX,
        token_id.to_string(Encoding::Base58)
    )
}

/// Persisted shape of a token registry entry. Token configuration is held
/// in its bincode form to keep the registry compact — decoded on demand
/// with `bincode::config::standard()` (matches the pre-C7 SQLite blob).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToken {
    /// Bincode-encoded `TokenConfiguration` (same shape as the pre-C7
    /// `token.token_config` BLOB column).
    config_bytes: Vec<u8>,
    /// Human-readable alias (e.g. the token's singular form in English).
    alias: String,
    /// Owning data-contract ID (base58 encoded in the key, raw bytes here).
    data_contract_id: [u8; 32],
    /// Token position within the contract's `tokens` map.
    position: u16,
}

/// Persisted shape of a DET-local contract entry. The contract itself is
/// stored as platform-serialized bytes (the canonical wire format), with
/// a user-chosen alias alongside. Decoded back into a [`DataContract`]
/// using the active SDK's [`platform_version`](AppContext::platform_version).
#[derive(Debug, Serialize, Deserialize)]
struct StoredContract {
    contract_bytes: Vec<u8>,
    alias: Option<String>,
}

/// Map a k/v adapter failure to the saved-contract storage error.
fn contract_err(source: KvAdapterError) -> TaskError {
    TaskError::ContractStorage { source }
}

/// Map a k/v adapter failure to the saved-token storage error.
fn token_err(source: KvAdapterError) -> TaskError {
    TaskError::TokenStorage { source }
}

impl AppContext {
    /// Retrieves all user-registered contracts from the per-network k/v
    /// store, prepended with the system contracts (DPNS, token history,
    /// withdrawals, keyword search, DashPay).
    pub fn get_contracts(&self) -> std::result::Result<Vec<QualifiedContract>, TaskError> {
        let mut contracts = self.load_user_contracts()?;

        // Pin the system contracts at the head of the list, in display order.
        let system_contracts = [
            (&self.dpns_contract, "dpns"),
            (&self.token_history_contract, "token_history"),
            (&self.withdraws_contract, "withdrawals"),
            (&self.keyword_search_contract, "keyword_search"),
            (&self.dashpay_contract, "dashpay"),
        ];
        for (index, (contract, alias)) in system_contracts.into_iter().enumerate() {
            contracts.insert(
                index,
                QualifiedContract {
                    contract: contract.as_ref().clone(),
                    alias: Some(alias.to_string()),
                },
            );
        }

        Ok(contracts)
    }

    /// Read every user-registered contract from the per-network k/v
    /// store. Entries that fail to decode are skipped with a warning
    /// rather than aborting the whole listing.
    fn load_user_contracts(&self) -> std::result::Result<Vec<QualifiedContract>, TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(DetScope::Global, Some(CONTRACT_KEY_PREFIX))
            .map_err(contract_err)?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            match kv.get::<StoredContract>(DetScope::Global, &key) {
                Ok(Some(stored)) => match self.decode_stored_contract(stored) {
                    Ok(qc) => out.push(qc),
                    Err(e) => tracing::warn!(
                        key = %key,
                        error = ?e,
                        "Skipping unreadable contract entry",
                    ),
                },
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    key = %key,
                    error = ?e,
                    "Skipping unreadable contract entry",
                ),
            }
        }
        Ok(out)
    }

    fn decode_stored_contract(
        &self,
        stored: StoredContract,
    ) -> std::result::Result<QualifiedContract, TaskError> {
        let contract = DataContract::versioned_deserialize(
            &stored.contract_bytes,
            false,
            self.platform_version(),
        )
        .map_err(|source| TaskError::ContractEncoding {
            source: Box::new(source),
        })?;
        Ok(QualifiedContract {
            contract,
            alias: stored.alias,
        })
    }

    pub fn get_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> std::result::Result<Option<QualifiedContract>, TaskError> {
        let backend = self.wallet_backend()?;
        let key = contract_key(contract_id);
        let stored: Option<StoredContract> = backend
            .kv()
            .get(DetScope::Global, &key)
            .map_err(contract_err)?;
        match stored {
            Some(stored) => Ok(Some(self.decode_stored_contract(stored)?)),
            None => Ok(None),
        }
    }

    pub fn get_unqualified_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> std::result::Result<Option<DataContract>, TaskError> {
        Ok(self.get_contract_by_id(contract_id)?.map(|qc| qc.contract))
    }

    /// Insert a contract entry if no record exists under its ID, mirroring
    /// the pre-C6 `INSERT OR IGNORE` semantics. Also persists token
    /// metadata for the requested positions in the local `token` table.
    pub fn insert_contract_if_not_exists(
        &self,
        data_contract: &DataContract,
        contract_alias: Option<&str>,
        insert_tokens_too: InsertTokensToo,
    ) -> std::result::Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let key = contract_key(&data_contract.id());

        // Only write the contract entry if none exists yet (INSERT OR IGNORE).
        let existing: Option<StoredContract> =
            kv.get(DetScope::Global, &key).map_err(contract_err)?;
        if existing.is_none() {
            let contract_bytes = data_contract
                .serialize_to_bytes_with_platform_version(self.platform_version())
                .map_err(|source| TaskError::ContractEncoding {
                    source: Box::new(source),
                })?;
            let stored = StoredContract {
                contract_bytes,
                alias: contract_alias.map(str::to_string),
            };
            kv.put(DetScope::Global, &key, &stored)
                .map_err(contract_err)?;
        }

        // Token metadata now also lives in the per-network k/v store (C7).
        if !data_contract.tokens().is_empty() {
            let positions: Vec<_> = match insert_tokens_too {
                InsertTokensToo::AllTokensShouldBeAdded => {
                    data_contract.tokens().keys().cloned().collect()
                }
                InsertTokensToo::NoTokensShouldBeAdded => return Ok(()),
                InsertTokensToo::SomeTokensShouldBeAdded(positions) => positions,
            };
            for token_contract_position in positions {
                if let Some(token_id) = data_contract.token_id(token_contract_position)
                    && let Ok(token_configuration) =
                        data_contract.expected_token_configuration(token_contract_position)
                {
                    let token_name = token_configuration
                        .conventions()
                        .singular_form_by_language_code_or_default("en")
                        .to_string();
                    self.insert_token(
                        &token_id,
                        &token_name,
                        token_configuration.clone(),
                        &data_contract.id(),
                        token_contract_position,
                    )?;
                }
            }
        }

        Ok(())
    }

    // Remove contract from the per-network k/v store by ID.
    pub fn remove_contract(&self, contract_id: &Identifier) -> std::result::Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        backend
            .kv()
            .delete(DetScope::Global, &contract_key(contract_id))
            .map_err(contract_err)
    }

    /// Replace a contract entry while preserving the existing alias, if any.
    pub fn replace_contract(
        &self,
        contract_id: Identifier,
        new_contract: &DataContract,
    ) -> std::result::Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let key = contract_key(&contract_id);

        let existing_alias = kv
            .get::<StoredContract>(DetScope::Global, &key)
            .map_err(contract_err)?
            .and_then(|s| s.alias);

        let contract_bytes = new_contract
            .serialize_to_bytes_with_platform_version(self.platform_version())
            .map_err(|source| TaskError::ContractEncoding {
                source: Box::new(source),
            })?;

        let stored = StoredContract {
            contract_bytes,
            alias: existing_alias,
        };
        kv.put(DetScope::Global, &key, &stored)
            .map_err(contract_err)
    }

    /// Update (or clear) the alias of an existing contract entry. Returns
    /// `Ok(())` even if the contract is unknown — matching the lenient
    /// "alias is metadata" UX the UI relies on.
    pub fn set_contract_alias(
        &self,
        contract_id: &Identifier,
        new_alias: Option<&str>,
    ) -> std::result::Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let key = contract_key(contract_id);
        let Some(mut stored) = kv
            .get::<StoredContract>(DetScope::Global, &key)
            .map_err(contract_err)?
        else {
            // No entry — nothing to rename. UI handles "missing" elsewhere.
            return Ok(());
        };
        stored.alias = new_alias.map(str::to_string);
        kv.put(DetScope::Global, &key, &stored)
            .map_err(contract_err)
    }

    /// List every synced `(identity, token)` balance pair from upstream,
    /// decorated with the token alias / config / data-contract fields the My
    /// Tokens screen expects. Identities that have not yet completed a sync
    /// pass are omitted (the UI renders them as "balance unknown").
    pub fn identity_token_balances(
        &self,
    ) -> std::result::Result<IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>, TaskError>
    {
        let kv = self.det_kv()?;
        let balances = self.token_balances_view()?.list();
        // Cache decoded token registry entries so repeated balances under
        // the same token only decode the configuration once.
        let mut token_cache: std::collections::HashMap<
            Identifier,
            (StoredToken, TokenConfiguration),
        > = std::collections::HashMap::new();

        let mut result = IndexMap::new();
        for (identity_id, token_id, balance) in balances {
            let entry = match token_cache.get(&token_id) {
                Some(cached) => cached.clone(),
                None => {
                    let token_key_s = token_key(&token_id);
                    let Some(stored) = kv
                        .get::<StoredToken>(DetScope::Global, &token_key_s)
                        .map_err(token_err)?
                    else {
                        tracing::debug!(
                            token = %token_id,
                            "Skipping balance for unknown token (registry entry missing)"
                        );
                        continue;
                    };
                    let cfg = decode_token_config(&stored.config_bytes)?;
                    let entry = (stored, cfg);
                    token_cache.insert(token_id, entry.clone());
                    entry
                }
            };
            let (stored, token_config) = entry;

            let data_contract_id = Identifier::from(stored.data_contract_id);
            result.insert(
                IdentityTokenIdentifier {
                    identity_id,
                    token_id,
                },
                IdentityTokenBalance {
                    token_id,
                    token_alias: stored.alias.clone(),
                    token_config,
                    identity_id,
                    balance,
                    estimated_unclaimed_rewards: None,
                    data_contract_id,
                    token_position: stored.position,
                },
            );
        }
        Ok(result)
    }

    /// Drop a single identity-token pair from the My Tokens ordering.
    /// Balances are owned upstream, so this only prunes DET's saved order
    /// list — see [`Self::mark_token_balance_untracked`] for the dismissal
    /// that keeps the pair out of future watch sets.
    pub fn remove_token_balance(
        &self,
        pair: IdentityTokenIdentifier,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        prune_token_order(&kv, |(t, i)| {
            !(*t == pair.token_id && *i == pair.identity_id)
        })?;
        Ok(())
    }

    /// Every identity-token pair the user stopped tracking.
    pub fn untracked_token_balances(
        &self,
    ) -> std::result::Result<BTreeSet<IdentityTokenIdentifier>, TaskError> {
        read_untracked(&self.det_kv()?)
    }

    /// Record an identity-token pair as no longer tracked, so balance
    /// refreshes stop re-watching it upstream. Idempotent.
    pub fn mark_token_balance_untracked(
        &self,
        pair: IdentityTokenIdentifier,
    ) -> std::result::Result<(), TaskError> {
        mark_untracked_in(&self.det_kv()?, pair)
    }

    /// Track a previously dismissed identity-token pair again.
    /// Idempotent — a pair that was never dismissed is left alone.
    pub fn clear_untracked_token_balance(
        &self,
        pair: IdentityTokenIdentifier,
    ) -> std::result::Result<(), TaskError> {
        clear_untracked_in(&self.det_kv()?, pair)
    }

    /// Track a token again for every identity that dismissed it. Idempotent.
    pub fn clear_untracked_token(
        &self,
        token_id: &Identifier,
    ) -> std::result::Result<(), TaskError> {
        clear_untracked_token_in(&self.det_kv()?, token_id)
    }

    /// Insert (or refresh) a token entry in the local registry. Balances are
    /// owned upstream now; the upstream sync loop fetches them once the
    /// identity's watch list is registered, so no balance row is seeded here.
    pub fn insert_token(
        &self,
        token_id: &Identifier,
        token_name: &str,
        token_configuration: TokenConfiguration,
        contract_id: &Identifier,
        token_position: u16,
    ) -> std::result::Result<(), TaskError> {
        let config = config::standard();
        let config_bytes = bincode::encode_to_vec(&token_configuration, config)
            .map_err(|source| TaskError::TokenConfigSerialization { source })?;
        let stored = StoredToken {
            config_bytes,
            alias: token_name.to_string(),
            data_contract_id: contract_id.to_buffer(),
            position: token_position,
        };
        let kv = self.det_kv()?;
        kv.put(DetScope::Global, &token_key(token_id), &stored)
            .map_err(token_err)
    }

    /// Reflect a proof-derived post-transaction balance for one
    /// `(identity, token)` pair in the upstream-fed snapshot immediately.
    /// Token mutation tasks (mint / transfer / burn / purchase / claim) call
    /// this with the authoritative balance from the state-transition result so
    /// the My Tokens screen updates before the next upstream sync pass.
    pub fn insert_token_identity_balance(
        &self,
        token_id: &Identifier,
        identity_id: &Identifier,
        balance: u64,
    ) -> std::result::Result<(), TaskError> {
        self.wallet_backend()?
            .apply_known_token_balance(*identity_id, *token_id, balance);
        Ok(())
    }

    /// Remove a token from the registry and drop it from the saved order and
    /// the dismissal list. Per-identity balances are owned upstream, so there
    /// is nothing local to cascade-delete.
    pub fn remove_token(&self, token_id: &Identifier) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        kv.delete(DetScope::Global, &token_key(token_id))
            .map_err(token_err)?;
        prune_token_order(&kv, |(t, _)| t != token_id)?;
        clear_untracked_token_in(&kv, token_id)?;
        Ok(())
    }

    /// Read the canonical [`TokenConfiguration`] stored at
    /// `det:token:<token_id>`, decoded with the pre-C7 bincode layout.
    pub fn get_token_config_for_id(
        &self,
        token_id: &Identifier,
    ) -> std::result::Result<Option<TokenConfiguration>, TaskError> {
        let kv = self.det_kv()?;
        let Some(stored) = kv
            .get::<StoredToken>(DetScope::Global, &token_key(token_id))
            .map_err(token_err)?
        else {
            return Ok(None);
        };
        Ok(Some(decode_token_config(&stored.config_bytes)?))
    }

    /// Reverse-lookup: find the data-contract ID that owns a given token.
    pub fn get_contract_id_by_token_id(
        &self,
        token_id: &Identifier,
    ) -> std::result::Result<Option<Identifier>, TaskError> {
        let kv = self.det_kv()?;
        let Some(stored) = kv
            .get::<StoredToken>(DetScope::Global, &token_key(token_id))
            .map_err(token_err)?
        else {
            return Ok(None);
        };
        Ok(Some(Identifier::from(stored.data_contract_id)))
    }

    /// Read every token registry entry, decode its configuration, and return
    /// the entries sorted by alias. Entries whose key does not decode back to a
    /// valid token id are skipped with a warning.
    fn load_sorted_tokens(
        &self,
    ) -> std::result::Result<Vec<(Identifier, StoredToken, TokenConfiguration)>, TaskError> {
        let kv = self.det_kv()?;
        let keys = kv
            .list(DetScope::Global, Some(TOKEN_KEY_PREFIX))
            .map_err(token_err)?;
        let mut sorted: Vec<(Identifier, StoredToken, TokenConfiguration)> = Vec::new();
        for key in keys {
            let Some(stored) = kv
                .get::<StoredToken>(DetScope::Global, &key)
                .map_err(token_err)?
            else {
                continue;
            };
            // Key has the base58 token-id appended directly after the
            // prefix — decode it back so we can use `Identifier` keys in
            // the resulting `IndexMap`.
            let suffix = match key.strip_prefix(TOKEN_KEY_PREFIX) {
                Some(s) => s,
                None => continue,
            };
            let Ok(token_id) = Identifier::from_string(suffix, Encoding::Base58) else {
                tracing::warn!(key = %key, "Skipping unparseable token key");
                continue;
            };
            let cfg = decode_token_config(&stored.config_bytes)?;
            sorted.push((token_id, stored, cfg));
        }
        sorted.sort_by(|a, b| a.1.alias.cmp(&b.1.alias));
        Ok(sorted)
    }

    /// List every token in the registry alongside its owning data
    /// contract. Falls back to the per-network k/v contract entry — the
    /// system-contract pinning lives in [`Self::get_contracts`].
    pub fn get_all_known_tokens_with_data_contract(
        &self,
    ) -> std::result::Result<IndexMap<Identifier, TokenInfoWithDataContract>, TaskError> {
        let mut result = IndexMap::new();
        for (token_id, stored, token_config) in self.load_sorted_tokens()? {
            let contract_id = Identifier::from(stored.data_contract_id);
            let Some(qc) = self.get_contract_by_id(&contract_id)? else {
                tracing::debug!(
                    token = %token_id,
                    contract = %contract_id,
                    "Skipping token: owning contract is not cached",
                );
                continue;
            };
            result.insert(
                token_id,
                TokenInfoWithDataContract {
                    token_id,
                    token_name: stored.alias,
                    data_contract: qc.contract,
                    token_position: stored.position,
                    token_configuration: token_config,
                    description: None,
                },
            );
        }
        Ok(result)
    }

    /// Lightweight variant of
    /// [`Self::get_all_known_tokens_with_data_contract`] that does not
    /// resolve the owning contract.
    pub fn get_all_known_tokens(
        &self,
    ) -> std::result::Result<IndexMap<Identifier, TokenInfo>, TaskError> {
        let mut result = IndexMap::new();
        for (token_id, stored, token_config) in self.load_sorted_tokens()? {
            result.insert(
                token_id,
                TokenInfo {
                    token_id,
                    token_name: stored.alias,
                    data_contract_id: Identifier::from(stored.data_contract_id),
                    token_position: stored.position,
                    token_configuration: token_config,
                    description: None,
                },
            );
        }
        Ok(result)
    }

    /// Persist the user-chosen `(token_id, identity_id)` ordering at
    /// `det:token_order:v1`. Mirrors pre-C7 semantics: the new list
    /// replaces the previous one wholesale.
    pub fn save_token_order(
        &self,
        all_ids: Vec<(Identifier, Identifier)>,
    ) -> std::result::Result<(), TaskError> {
        let kv = self.det_kv()?;
        let payload: Vec<([u8; 32], [u8; 32])> = all_ids
            .iter()
            .map(|(t, i)| (t.to_buffer(), i.to_buffer()))
            .collect();
        kv.put(DetScope::Global, TOKEN_ORDER_KEY, &payload)
            .map_err(token_err)
    }

    /// Load the user-chosen `(token_id, identity_id)` ordering. Returns
    /// an empty `Vec` when no ordering has been saved yet.
    pub fn load_token_order(
        &self,
    ) -> std::result::Result<Vec<(Identifier, Identifier)>, TaskError> {
        let kv = self.det_kv()?;
        let Some(payload): Option<Vec<([u8; 32], [u8; 32])>> = kv
            .get(DetScope::Global, TOKEN_ORDER_KEY)
            .map_err(token_err)?
        else {
            return Ok(Vec::new());
        };
        Ok(payload
            .into_iter()
            .map(|(t, i)| (Identifier::from(t), Identifier::from(i)))
            .collect())
    }

    /// Devnet-only sweep: drop the token registry, every balance entry,
    /// the saved order and the dismissal list. No-op on non-devnet networks.
    pub fn delete_all_local_tokens_in_devnet(&self) -> std::result::Result<(), TaskError> {
        if self.network != Network::Devnet {
            return Ok(());
        }
        let kv = self.det_kv()?;
        delete_by_prefix(&kv, TOKEN_KEY_PREFIX)?;
        delete_by_prefix(&kv, TOKEN_UNTRACKED_PREFIX)?;
        // Balances are owned upstream now — nothing local to wipe.
        kv.delete(DetScope::Global, TOKEN_ORDER_KEY)
            .map_err(token_err)?;
        Ok(())
    }

    /// Upstream-fed token-balance view (T6 seam). Reads the lock-free
    /// snapshot the wallet backend refreshes from the upstream identity-sync
    /// loop.
    fn token_balances_view(&self) -> std::result::Result<UpstreamTokenBalances, TaskError> {
        Ok(self.wallet_backend()?.token_balances())
    }

    /// Drop every user-registered contract entry for this network. Only
    /// applies to devnet contexts — guarded to match the pre-C6
    /// [`Database::remove_all_contracts_in_devnet`] behaviour.
    pub fn clear_user_contracts(&self) -> std::result::Result<(), TaskError> {
        if self.network != Network::Devnet {
            return Ok(());
        }
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(DetScope::Global, Some(CONTRACT_KEY_PREFIX))
            .map_err(contract_err)?;
        for key in keys {
            kv.delete(DetScope::Global, &key).map_err(contract_err)?;
        }
        Ok(())
    }

    pub fn get_contract_by_token_id(
        &self,
        token_id: &Identifier,
    ) -> std::result::Result<Option<QualifiedContract>, TaskError> {
        let Some(contract_id) = self.get_contract_id_by_token_id(token_id)? else {
            return Ok(None);
        };
        self.get_contract_by_id(&contract_id)
    }
}

/// Decode a stored bincode-encoded [`TokenConfiguration`] blob. Mirrors
/// the pre-C7 read path that wrapped the same bytes inside SQLite.
fn decode_token_config(bytes: &[u8]) -> std::result::Result<TokenConfiguration, TaskError> {
    bincode::decode_from_slice::<TokenConfiguration, _>(bytes, config::standard())
        .map(|(cfg, _)| cfg)
        .map_err(|source| TaskError::TokenConfigEncoding { source })
}

/// Read the dismissed pairs back off their marker keys. No markers means
/// nothing was ever dismissed. A key that does not decode back to a
/// `(token, identity)` pair is skipped with a warning rather than failing the
/// whole read — one unreadable marker must not hide every other dismissal.
fn read_untracked(kv: &DetKv) -> std::result::Result<BTreeSet<IdentityTokenIdentifier>, TaskError> {
    let keys = kv
        .list(DetScope::Global, Some(TOKEN_UNTRACKED_PREFIX))
        .map_err(token_err)?;
    let mut pairs = BTreeSet::new();
    for key in keys {
        match parse_untracked_key(&key) {
            Some(pair) => {
                pairs.insert(pair);
            }
            None => tracing::warn!(key = %key, "Skipping unparseable token dismissal key"),
        }
    }
    Ok(pairs)
}

/// Decode a marker key back into the pair it dismisses. The inverse of
/// [`untracked_key`] — the sole place the `<token>:<identity>` key layout is
/// mapped back onto the named fields.
fn parse_untracked_key(key: &str) -> Option<IdentityTokenIdentifier> {
    let (token, identity) = key.strip_prefix(TOKEN_UNTRACKED_PREFIX)?.split_once(':')?;
    Some(IdentityTokenIdentifier {
        token_id: Identifier::from_string(token, Encoding::Base58).ok()?,
        identity_id: Identifier::from_string(identity, Encoding::Base58).ok()?,
    })
}

/// Dismiss one pair. A single upsert of that pair's marker — idempotent, and
/// independent of every other pair's marker.
fn mark_untracked_in(
    kv: &DetKv,
    pair: IdentityTokenIdentifier,
) -> std::result::Result<(), TaskError> {
    kv.put(DetScope::Global, &untracked_key(&pair), &())
        .map_err(token_err)
}

/// Re-track one pair. A single delete of that pair's marker — idempotent, and
/// independent of every other pair's marker.
fn clear_untracked_in(
    kv: &DetKv,
    pair: IdentityTokenIdentifier,
) -> std::result::Result<(), TaskError> {
    kv.delete(DetScope::Global, &untracked_key(&pair))
        .map_err(token_err)
}

/// Drop the token-list state belonging to `identity_id`: every balance
/// dismissal it recorded, and its entries in the saved ordering.
///
/// The identity counterpart of [`clear_untracked_token_in`] and of the pruning
/// [`AppContext::remove_token`] does — a marker naming an identity that no
/// longer exists is invisible to the user, so left behind it silently re-hides
/// that token if the identity is ever loaded again.
///
/// The dismissal keys lead with the token id, so one identity's markers are not
/// a single prefix scan: the whole family is listed and matched on the decoded
/// pair, keeping [`parse_untracked_key`] the only reader of the key layout.
pub(super) fn forget_identity_token_state(
    kv: &DetKv,
    identity_id: &Identifier,
) -> std::result::Result<(), TaskError> {
    for key in kv
        .list(DetScope::Global, Some(TOKEN_UNTRACKED_PREFIX))
        .map_err(token_err)?
    {
        if parse_untracked_key(&key).is_some_and(|pair| pair.identity_id == *identity_id) {
            kv.delete(DetScope::Global, &key).map_err(token_err)?;
        }
    }
    prune_token_order(kv, |(_, identity)| identity != identity_id)
}

/// Drop every dismissal recorded for `token_id`, whichever identity made it.
fn clear_untracked_token_in(
    kv: &DetKv,
    token_id: &Identifier,
) -> std::result::Result<(), TaskError> {
    delete_by_prefix(kv, &untracked_token_prefix(token_id))
}

/// Delete every Global-scoped key under `prefix`.
fn delete_by_prefix(kv: &DetKv, prefix: &str) -> std::result::Result<(), TaskError> {
    for key in kv.list(DetScope::Global, Some(prefix)).map_err(token_err)? {
        kv.delete(DetScope::Global, &key).map_err(token_err)?;
    }
    Ok(())
}

/// Filter the stored token-order list and write it back when the filter
/// drops any entries. No-op when no order list exists yet.
fn prune_token_order<F>(kv: &DetKv, keep: F) -> std::result::Result<(), TaskError>
where
    F: Fn(&(Identifier, Identifier)) -> bool,
{
    let Some(payload): Option<Vec<([u8; 32], [u8; 32])>> = kv
        .get(DetScope::Global, TOKEN_ORDER_KEY)
        .map_err(token_err)?
    else {
        return Ok(());
    };
    let before = payload.len();
    let kept: Vec<_> = payload
        .into_iter()
        .map(|(t, i)| (Identifier::from(t), Identifier::from(i)))
        .filter(|pair| keep(pair))
        .collect();
    if kept.len() == before {
        return Ok(());
    }
    let serialized: Vec<([u8; 32], [u8; 32])> = kept
        .iter()
        .map(|(t, i)| (t.to_buffer(), i.to_buffer()))
        .collect();
    kv.put(DetScope::Global, TOKEN_ORDER_KEY, &serialized)
        .map_err(token_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use platform_wallet_storage::{KvError, KvStore, ObjectId};
    use std::sync::{Arc, Mutex};

    fn empty_kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    fn ident(b: u8) -> Identifier {
        Identifier::from([b; 32])
    }

    fn stored_token(alias: &str, contract: u8, position: u16) -> StoredToken {
        StoredToken {
            config_bytes: vec![0xAB, 0xCD, 0xEF],
            alias: alias.to_string(),
            data_contract_id: [contract; 32],
            position,
        }
    }

    // ----------------------------------------------------------------
    // Token registry: Global-scoped k/v round-trip.
    // ----------------------------------------------------------------

    #[test]
    fn token_registry_entry_round_trips_in_global_scope() {
        let kv = empty_kv();
        let token = ident(1);
        let key = token_key(&token);
        let stored = stored_token("MyToken", 9, 3);
        kv.put(DetScope::Global, &key, &stored).unwrap();

        let got: StoredToken = kv.get(DetScope::Global, &key).unwrap().unwrap();
        assert_eq!(got.alias, "MyToken");
        assert_eq!(got.position, 3);
        assert_eq!(got.data_contract_id, [9u8; 32]);
        assert_eq!(got.config_bytes, vec![0xAB, 0xCD, 0xEF]);
    }

    #[test]
    fn missing_token_registry_entry_reads_as_none() {
        let kv = empty_kv();
        let got: Option<StoredToken> = kv.get(DetScope::Global, &token_key(&ident(2))).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn token_registry_put_upserts_in_place() {
        let kv = empty_kv();
        let token = ident(1);
        let key = token_key(&token);
        kv.put(DetScope::Global, &key, &stored_token("Old", 1, 0))
            .unwrap();
        kv.put(DetScope::Global, &key, &stored_token("New", 2, 5))
            .unwrap();
        let got: StoredToken = kv.get(DetScope::Global, &key).unwrap().unwrap();
        assert_eq!(got.alias, "New");
        assert_eq!(got.position, 5);
        assert_eq!(got.data_contract_id, [2u8; 32]);
    }

    #[test]
    fn token_keys_carry_a_base58_suffix_that_round_trips_to_the_id() {
        // The listing readers strip the prefix and decode the base58 suffix
        // back into an `Identifier`; assert that contract holds.
        let token = ident(5);
        let key = token_key(&token);
        let suffix = key.strip_prefix(TOKEN_KEY_PREFIX).unwrap();
        let decoded = Identifier::from_string(suffix, Encoding::Base58).unwrap();
        assert_eq!(decoded, token);
    }

    #[test]
    fn contract_keys_carry_a_base58_suffix_that_round_trips_to_the_id() {
        let contract = ident(6);
        let key = contract_key(&contract);
        let suffix = key.strip_prefix(CONTRACT_KEY_PREFIX).unwrap();
        let decoded = Identifier::from_string(suffix, Encoding::Base58).unwrap();
        assert_eq!(decoded, contract);
    }

    // ----------------------------------------------------------------
    // Token order: prune helper rewrites the list only when it shrinks.
    // ----------------------------------------------------------------

    #[test]
    fn prune_token_order_drops_only_filtered_pairs() {
        let kv = empty_kv();
        let keep_pair = (ident(1), ident(10));
        let drop_pair = (ident(2), ident(20));
        let payload: Vec<([u8; 32], [u8; 32])> = vec![
            (keep_pair.0.to_buffer(), keep_pair.1.to_buffer()),
            (drop_pair.0.to_buffer(), drop_pair.1.to_buffer()),
        ];
        kv.put(DetScope::Global, TOKEN_ORDER_KEY, &payload).unwrap();

        prune_token_order(&kv, |(t, i)| !(*t == drop_pair.0 && *i == drop_pair.1)).unwrap();

        let got: Vec<([u8; 32], [u8; 32])> =
            kv.get(DetScope::Global, TOKEN_ORDER_KEY).unwrap().unwrap();
        assert_eq!(
            got,
            vec![(keep_pair.0.to_buffer(), keep_pair.1.to_buffer())]
        );
    }

    #[test]
    fn prune_token_order_is_noop_when_no_order_list_exists() {
        let kv = empty_kv();
        // No order list has ever been written — pruning must not create one.
        prune_token_order(&kv, |_| true).unwrap();
        let got: Option<Vec<([u8; 32], [u8; 32])>> =
            kv.get(DetScope::Global, TOKEN_ORDER_KEY).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn prune_token_order_keeps_list_intact_when_filter_drops_nothing() {
        let kv = empty_kv();
        let payload: Vec<([u8; 32], [u8; 32])> =
            vec![(ident(1).to_buffer(), ident(10).to_buffer())];
        kv.put(DetScope::Global, TOKEN_ORDER_KEY, &payload).unwrap();
        prune_token_order(&kv, |_| true).unwrap();
        let got: Vec<([u8; 32], [u8; 32])> =
            kv.get(DetScope::Global, TOKEN_ORDER_KEY).unwrap().unwrap();
        assert_eq!(got, payload);
    }

    // ----------------------------------------------------------------
    // Untracked pairs: dismissals persist so refreshes stop re-watching.
    // ----------------------------------------------------------------

    fn pair(token: u8, identity: u8) -> IdentityTokenIdentifier {
        IdentityTokenIdentifier {
            token_id: ident(token),
            identity_id: ident(identity),
        }
    }

    #[test]
    fn untracked_pairs_read_as_empty_before_anything_is_dismissed() {
        let kv = empty_kv();
        assert!(read_untracked(&kv).unwrap().is_empty());
    }

    /// Seed dismissals through the same path the backend tasks use.
    fn dismiss_all(kv: &DetKv, pairs: impl IntoIterator<Item = IdentityTokenIdentifier>) {
        for pair in pairs {
            mark_untracked_in(kv, pair).expect("dismiss pair");
        }
    }

    #[test]
    fn untracked_pairs_round_trip_through_the_kv_store() {
        let kv = empty_kv();
        let pairs = BTreeSet::from([pair(1, 10), pair(2, 20)]);
        dismiss_all(&kv, pairs.iter().copied());
        assert_eq!(read_untracked(&kv).unwrap(), pairs);
    }

    #[test]
    fn dismissing_the_same_pair_twice_is_idempotent() {
        let kv = empty_kv();
        dismiss_all(&kv, [pair(1, 10), pair(1, 10)]);
        assert_eq!(read_untracked(&kv).unwrap(), BTreeSet::from([pair(1, 10)]));
    }

    #[test]
    fn re_tracking_a_pair_that_was_never_dismissed_is_a_noop() {
        let kv = empty_kv();
        dismiss_all(&kv, [pair(1, 10)]);
        clear_untracked_in(&kv, pair(2, 20)).unwrap();
        assert_eq!(read_untracked(&kv).unwrap(), BTreeSet::from([pair(1, 10)]));
    }

    /// A marker key carries `<token>:<identity>` in that order. Decoding it the
    /// other way round would silently un-track a pair nobody dismissed.
    #[test]
    fn a_marker_key_round_trips_back_to_its_pair_token_id_first() {
        let dismissed = pair(1, 10);
        let key = untracked_key(&dismissed);

        assert_eq!(
            key,
            format!(
                "{TOKEN_UNTRACKED_PREFIX}{}:{}",
                ident(1).to_string(Encoding::Base58),
                ident(10).to_string(Encoding::Base58)
            ),
            "token_id is keyed first, identity_id second"
        );
        assert_eq!(parse_untracked_key(&key), Some(dismissed));
    }

    #[test]
    fn an_unparseable_marker_key_is_skipped_rather_than_failing_the_read() {
        let kv = empty_kv();
        dismiss_all(&kv, [pair(1, 10)]);
        kv.put(
            DetScope::Global,
            &format!("{TOKEN_UNTRACKED_PREFIX}not-base58:nonsense"),
            &(),
        )
        .unwrap();

        assert_eq!(read_untracked(&kv).unwrap(), BTreeSet::from([pair(1, 10)]));
    }

    #[test]
    fn clearing_a_token_drops_its_dismissals_for_every_identity() {
        let kv = empty_kv();
        let cleared = ident(1);
        let kept = pair(2, 10);
        dismiss_all(&kv, [pair(1, 10), pair(1, 11), kept]);

        clear_untracked_token_in(&kv, &cleared).unwrap();

        assert_eq!(read_untracked(&kv).unwrap(), BTreeSet::from([kept]));
    }

    #[test]
    fn clearing_an_undismissed_token_is_a_noop() {
        let kv = empty_kv();
        let pairs = BTreeSet::from([pair(1, 10)]);
        dismiss_all(&kv, pairs.iter().copied());
        clear_untracked_token_in(&kv, &ident(9)).unwrap();
        assert_eq!(read_untracked(&kv).unwrap(), pairs);
    }

    // ----------------------------------------------------------------
    // Untracked pairs: concurrent dismissals must not clobber each other.
    //
    // Dismiss / re-track are independent backend tasks, each `tokio::spawn`ed,
    // so two of them can overlap. A mutation that reads the whole dismissal
    // set and writes it back has a window in which a concurrent mutation is
    // read-before / written-over — the lost update surfaces as a dismissed
    // token reappearing on the next refresh.
    // ----------------------------------------------------------------

    /// Delegates to [`InMemoryKv`], stalling *after* each read has taken its
    /// snapshot. Two concurrent mutations therefore both observe the
    /// pre-mutation state and write back late: a mutation that reads before it
    /// writes loses its peer's update, while a mutation that only writes never
    /// reads, never stalls, and cannot be clobbered.
    #[derive(Default)]
    struct StallingReadKv {
        inner: InMemoryKv,
    }

    impl KvStore for StallingReadKv {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            let value = self.inner.get(scope, key);
            std::thread::sleep(std::time::Duration::from_millis(200));
            value
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

    fn stalling_kv() -> DetKv {
        DetKv::from_store(Arc::new(StallingReadKv::default()))
    }

    #[test]
    fn concurrent_dismissals_of_different_pairs_both_survive() {
        let kv = stalling_kv();
        let (first, second) = (pair(1, 10), pair(2, 20));

        std::thread::scope(|scope| {
            scope.spawn(|| mark_untracked_in(&kv, first).expect("dismiss first"));
            scope.spawn(|| mark_untracked_in(&kv, second).expect("dismiss second"));
        });

        assert_eq!(
            read_untracked(&kv).unwrap(),
            BTreeSet::from([first, second]),
            "each dismissal must be an independent write — neither may clobber the other"
        );
    }

    #[test]
    fn a_dismissal_racing_a_re_track_of_another_pair_keeps_both_mutations() {
        let kv = stalling_kv();
        let (dismissed, kept, retracked) = (pair(1, 10), pair(2, 20), pair(3, 30));
        for seed in [retracked, kept] {
            mark_untracked_in(&kv, seed).expect("seed dismissal");
        }

        std::thread::scope(|scope| {
            scope.spawn(|| mark_untracked_in(&kv, dismissed).expect("dismiss"));
            scope.spawn(|| clear_untracked_in(&kv, retracked).expect("re-track"));
        });

        assert_eq!(
            read_untracked(&kv).unwrap(),
            BTreeSet::from([dismissed, kept]),
            "a dismissal and a re-track of different pairs must both land"
        );
    }

    /// Records every store operation so a read-modify-write can be spotted
    /// structurally, without relying on a thread interleaving.
    #[derive(Default)]
    struct RecordingKv {
        inner: InMemoryKv,
        ops: Mutex<Vec<String>>,
    }

    impl KvStore for RecordingKv {
        fn get(&self, scope: &ObjectId, key: &str) -> Result<Option<Vec<u8>>, KvError> {
            self.ops.lock().unwrap().push(format!("get {key}"));
            self.inner.get(scope, key)
        }

        fn put(&self, scope: &ObjectId, key: &str, value: &[u8]) -> Result<(), KvError> {
            self.ops.lock().unwrap().push(format!("put {key}"));
            self.inner.put(scope, key, value)
        }

        fn delete(&self, scope: &ObjectId, key: &str) -> Result<(), KvError> {
            self.ops.lock().unwrap().push(format!("delete {key}"));
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

    /// The structural invariant behind the two race tests above: a dismissal
    /// and a re-track each touch exactly their own pair's key, and neither
    /// reads the dismissal set first — so there is no window to lose.
    #[test]
    fn dismissing_and_re_tracking_a_pair_are_single_writes_with_no_read_back() {
        let store = Arc::new(RecordingKv::default());
        let kv = DetKv::from_store(store.clone());
        let dismissed = pair(1, 10);
        let key = untracked_key(&dismissed);

        mark_untracked_in(&kv, dismissed).unwrap();
        clear_untracked_in(&kv, dismissed).unwrap();

        assert_eq!(
            *store.ops.lock().unwrap(),
            vec![format!("put {key}"), format!("delete {key}")],
            "a mutation that reads the dismissal set before writing it back would race a concurrent one"
        );
    }
}
