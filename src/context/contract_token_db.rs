use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::wallet::WalletId;
use crate::ui::tokens::tokens_screen::{IdentityTokenBalance, IdentityTokenIdentifier};
use bincode::config;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::query_types::IndexMap;
use rusqlite::Result;
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl AppContext {
    /// Retrieves all contracts from the database plus the system contracts from app context.
    pub fn get_contracts(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<QualifiedContract>> {
        // Get contracts from the database
        let mut contracts = self.db.get_contracts(self, limit, offset)?;

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

    pub fn get_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Result<Option<QualifiedContract>> {
        // Get the contract from the database
        self.db.get_contract_by_id(*contract_id, self)
    }

    pub fn get_unqualified_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Result<Option<DataContract>> {
        // Get the contract from the database
        self.db.get_unqualified_contract_by_id(*contract_id, self)
    }

    // Remove contract from the database by ID
    pub fn remove_contract(&self, contract_id: &Identifier) -> Result<()> {
        self.db.remove_contract(contract_id.as_bytes(), self)
    }

    pub fn replace_contract(
        &self,
        contract_id: Identifier,
        new_contract: &DataContract,
    ) -> Result<()> {
        self.db.replace_contract(contract_id, new_contract, self)
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

    pub fn remove_wallet(&self, seed_hash: &WalletId) -> Result<(), TaskError> {
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

    pub fn get_contract_by_token_id(
        &self,
        token_id: &Identifier,
    ) -> Result<Option<QualifiedContract>> {
        let Some(contract_id) = self.db.get_contract_id_by_token_id(token_id, self)? else {
            return Ok(None);
        };
        self.db.get_contract_by_id(contract_id, self)
    }
}
