use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::qualified_contract::{InsertTokensToo, QualifiedContract};
use crate::model::wallet::WalletSeedHash;
use crate::ui::tokens::tokens_screen::{IdentityTokenBalance, IdentityTokenIdentifier};
use bincode::config;
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
use rusqlite::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Key prefix for user-registered contract entries in the per-network
/// wallet k/v store. The full key is `det:contract:<contract_id_base58>`
/// and lives in the global (`None`) scope — contracts are a
/// network-scoped concept inside the per-network k/v store.
const CONTRACT_KEY_PREFIX: &str = "det:contract:";

fn contract_key(contract_id: &Identifier) -> String {
    format!(
        "{}{}",
        CONTRACT_KEY_PREFIX,
        contract_id.to_string(Encoding::Base58)
    )
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

impl AppContext {
    /// Retrieves all user-registered contracts from the per-network k/v
    /// store, prepended with the system contracts (DPNS, token history,
    /// withdrawals, keyword search, DashPay).
    pub fn get_contracts(
        &self,
        _limit: Option<u32>,
        _offset: Option<u32>,
    ) -> std::result::Result<Vec<QualifiedContract>, TaskError> {
        let mut contracts = self.load_user_contracts()?;

        // Add the DPNS contract to the list
        let dpns_contract = QualifiedContract {
            contract: Arc::clone(&self.dpns_contract).as_ref().clone(),
            alias: Some("dpns".to_string()),
        };

        // Insert the DPNS contract at 0
        contracts.insert(0, dpns_contract);

        // Add the token history contract to the list
        let token_history_contract = QualifiedContract {
            contract: Arc::clone(&self.token_history_contract).as_ref().clone(),
            alias: Some("token_history".to_string()),
        };

        // Insert the token history contract at 1
        contracts.insert(1, token_history_contract);

        // Add the withdrawal contract to the list
        let withdraws_contract = QualifiedContract {
            contract: Arc::clone(&self.withdraws_contract).as_ref().clone(),
            alias: Some("withdrawals".to_string()),
        };

        // Insert the withdrawal contract at 2
        contracts.insert(2, withdraws_contract);

        // Add the keyword search contract to the list
        let keyword_search_contract = QualifiedContract {
            contract: Arc::clone(&self.keyword_search_contract).as_ref().clone(),
            alias: Some("keyword_search".to_string()),
        };

        // Insert the keyword search contract at 3
        contracts.insert(3, keyword_search_contract);

        // Add the DashPay contract to the list
        let dashpay_contract = QualifiedContract {
            contract: Arc::clone(&self.dashpay_contract).as_ref().clone(),
            alias: Some("dashpay".to_string()),
        };

        // Insert the DashPay contract at 4
        contracts.insert(4, dashpay_contract);

        Ok(contracts)
    }

    /// Read every user-registered contract from the per-network k/v
    /// store. Entries that fail to decode are skipped with a warning
    /// rather than aborting the whole listing.
    fn load_user_contracts(&self) -> std::result::Result<Vec<QualifiedContract>, TaskError> {
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(None, Some(CONTRACT_KEY_PREFIX))
            .map_err(|source| TaskError::ContractStorage { source })?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            match kv.get::<StoredContract>(None, &key) {
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
            .get(None, &key)
            .map_err(|source| TaskError::ContractStorage { source })?;
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
        let existing: Option<StoredContract> = kv
            .get(None, &key)
            .map_err(|source| TaskError::ContractStorage { source })?;
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
            kv.put(None, &key, &stored)
                .map_err(|source| TaskError::ContractStorage { source })?;
        }

        // Token metadata still lives in the local `token` table (untouched by C6).
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
                    let config = config::standard();
                    let Some(serialized_token_configuration) =
                        bincode::encode_to_vec(token_configuration, config).ok()
                    else {
                        return Ok(());
                    };
                    let token_name = token_configuration
                        .conventions()
                        .singular_form_by_language_code_or_default("en");
                    self.db.insert_token(
                        &token_id,
                        token_name,
                        serialized_token_configuration.as_slice(),
                        &data_contract.id(),
                        token_contract_position,
                        self,
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
            .delete(None, &contract_key(contract_id))
            .map_err(|source| TaskError::ContractStorage { source })
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
            .get::<StoredContract>(None, &key)
            .map_err(|source| TaskError::ContractStorage { source })?
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
        kv.put(None, &key, &stored)
            .map_err(|source| TaskError::ContractStorage { source })
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
            .get::<StoredContract>(None, &key)
            .map_err(|source| TaskError::ContractStorage { source })?
        else {
            // No entry — nothing to rename. UI handles "missing" elsewhere.
            return Ok(());
        };
        stored.alias = new_alias.map(str::to_string);
        kv.put(None, &key, &stored)
            .map_err(|source| TaskError::ContractStorage { source })
    }

    pub fn identity_token_balances(
        &self,
    ) -> Result<IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>> {
        self.db.get_identity_token_balances(self)
    }

    pub fn remove_token_balance(
        &self,
        token_id: Identifier,
        identity_id: Identifier,
    ) -> Result<()> {
        self.db.remove_token_balance(&token_id, &identity_id, self)
    }

    pub fn insert_token(
        &self,
        token_id: &Identifier,
        token_name: &str,
        token_configuration: TokenConfiguration,
        contract_id: &Identifier,
        token_position: u16,
    ) -> Result<()> {
        let config = config::standard();
        let Some(serialized_token_configuration) =
            bincode::encode_to_vec(&token_configuration, config).ok()
        else {
            // We should always be able to serialize
            return Ok(());
        };

        self.db.insert_token(
            token_id,
            token_name,
            serialized_token_configuration.as_slice(),
            contract_id,
            token_position,
            self,
        )?;

        Ok(())
    }

    pub fn remove_token(&self, token_id: &Identifier) -> Result<()> {
        self.db.remove_token(token_id, self)
    }

    pub fn remove_wallet(&self, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        // Acquire write lock first to ensure atomicity — if the lock fails,
        // no changes have been made to the database.
        let mut wallets = self.wallets.write()?;
        if !wallets.contains_key(seed_hash) {
            return Err(TaskError::WalletNotFound);
        }

        self.db.remove_wallet(seed_hash, &self.network)?;

        wallets.remove(seed_hash);
        let has_wallet = !wallets.is_empty();
        drop(wallets);

        self.has_wallet.store(has_wallet, Ordering::Relaxed);

        Ok(())
    }

    #[allow(dead_code)] // May be used for storing token balances
    pub fn insert_token_identity_balance(
        &self,
        token_id: &Identifier,
        identity_id: &Identifier,
        balance: u64,
    ) -> Result<()> {
        self.db
            .insert_identity_token_balance(token_id, identity_id, balance, self)?;

        Ok(())
    }

    /// Drop every user-registered contract entry for this network. Only
    /// applies to devnet contexts — guarded to match the pre-C6
    /// [`Database::remove_all_contracts_in_devnet`] behaviour.
    pub fn clear_user_contracts(&self) -> std::result::Result<(), TaskError> {
        use dash_sdk::dpp::dashcore::Network;
        if self.network != Network::Devnet {
            return Ok(());
        }
        let backend = self.wallet_backend()?;
        let kv = backend.kv();
        let keys = kv
            .list(None, Some(CONTRACT_KEY_PREFIX))
            .map_err(|source| TaskError::ContractStorage { source })?;
        for key in keys {
            kv.delete(None, &key)
                .map_err(|source| TaskError::ContractStorage { source })?;
        }
        Ok(())
    }

    pub fn get_contract_by_token_id(
        &self,
        token_id: &Identifier,
    ) -> std::result::Result<Option<QualifiedContract>, TaskError> {
        let Some(contract_id) = self.db.get_contract_id_by_token_id(token_id, self)? else {
            return Ok(None);
        };
        self.get_contract_by_id(&contract_id)
    }
}
