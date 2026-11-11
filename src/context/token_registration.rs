//! Network-bound public token registration journal and cross-process operation ownership.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::token_creator::registration::RegistrationRecord;
use crate::wallet_backend::network_prefix;
use crate::wallet_backend::{DetKv, DetScope};
use std::{
    fs::{File, OpenOptions},
    sync::Arc,
};

pub(crate) struct TokenRegistrationJournal {
    kv: Arc<DetKv>,
    key: String,
    // A dedicated advisory operation lock, not a database transaction. Dropping releases it.
    _operation: File,
}

impl AppContext {
    /// Reads the current public creation receipt or unresolved attempt without changing it.
    pub fn token_registration_record(&self) -> Result<Option<RegistrationRecord>, TaskError> {
        self.app_kv()
            .get(DetScope::Global, &record_key(self.network))
            .map_err(|source| TaskError::TokenCreatorJournal { source })
    }
    /// Acquires exclusive operation ownership across windows and processes for this network.
    pub(crate) fn token_registration_journal(&self) -> Result<TokenRegistrationJournal, TaskError> {
        let path = self.data_dir().join(format!(
            "token-registration-{}.lock",
            network_prefix(self.network)
        ));
        let operation = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| TaskError::TokenCreatorLock { source })?;
        operation.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => TaskError::TokenCreatorBusy,
            std::fs::TryLockError::Error(source) => TaskError::TokenCreatorLock { source },
        })?;
        Ok(TokenRegistrationJournal {
            kv: self.app_kv(),
            key: record_key(self.network),
            _operation: operation,
        })
    }
}
fn record_key(network: dash_sdk::dpp::dashcore::Network) -> String {
    format!("{}:token-registration:v1", network_prefix(network))
}
impl TokenRegistrationJournal {
    /// Reads the record while this operation owns the journal.
    pub fn read(&self) -> Result<Option<RegistrationRecord>, TaskError> {
        self.kv
            .get(DetScope::Global, &self.key)
            .map_err(|source| TaskError::TokenCreatorJournal { source })
    }
    /// Persists before broadcast, refusing any unresolved or repeated attempt.
    pub fn claim(&self, record: &RegistrationRecord) -> Result<(), TaskError> {
        if self.read()?.is_some_and(|current| {
            current.blocks_registration() || current.attempt_id == record.attempt_id
        }) {
            return Err(TaskError::TokenCreatorBusy);
        }
        self.kv
            .put(DetScope::Global, &self.key, record)
            .map_err(|source| TaskError::TokenCreatorJournal { source })
    }
    /// Settles only the attempt whose immutable public payload is still current.
    pub fn update(&self, record: &RegistrationRecord) -> Result<(), TaskError> {
        let current = self.read()?.ok_or(TaskError::TokenCreatorChanged)?;
        if current.attempt_id != record.attempt_id
            || current.contract_bytes != record.contract_bytes
            || current.network != record.network
        {
            return Err(TaskError::TokenCreatorChanged);
        }
        self.kv
            .put(DetScope::Global, &self.key, record)
            .map_err(|source| TaskError::TokenCreatorJournal { source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::token_creator::registration::RegistrationStatus;
    use dash_sdk::platform::Identifier;
    fn record(attempt: u8) -> RegistrationRecord {
        RegistrationRecord {
            attempt_id: [attempt; 32],
            network: dash_sdk::dpp::dashcore::Network::Testnet,
            owner: Identifier::from([2; 32]),
            contract_id: Identifier::from([3; 32]),
            token_id: Identifier::from([4; 32]),
            nonce: 1,
            key_id: 1,
            protocol_version: 14,
            contract_bytes: vec![],
            alias: "Public test".into(),
            status: RegistrationStatus::Unknown,
        }
    }
    #[test]
    fn token_registration_pending_attempt_cannot_be_overwritten_or_settled_by_another() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let journal = context.token_registration_journal().unwrap();
        let first = record(1);
        journal.claim(&first).unwrap();
        let mut other = record(2);
        assert!(journal.claim(&other).is_err());
        other.status = RegistrationStatus::Confirmed;
        assert!(journal.update(&other).is_err());
        assert_eq!(journal.read().unwrap(), Some(first.clone()));
        drop(journal);
        assert_eq!(context.token_registration_record().unwrap(), Some(first));
    }
    #[test]
    fn token_registration_operation_lock_excludes_another_context() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let first = context.token_registration_journal().unwrap();
        assert!(context.token_registration_journal().is_err());
        drop(first);
        assert!(context.token_registration_journal().is_ok());
    }
}
