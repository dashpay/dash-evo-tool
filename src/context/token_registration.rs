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

pub(crate) struct TokenRegistrationJournal<'a> {
    context: &'a AppContext,
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
    pub(crate) fn token_registration_journal(
        &self,
    ) -> Result<TokenRegistrationJournal<'_>, TaskError> {
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
            context: self,
            kv: self.app_kv(),
            key: record_key(self.network),
            _operation: operation,
        })
    }
}
fn record_key(network: dash_sdk::dpp::dashcore::Network) -> String {
    format!("{}:token-registration:v1", network_prefix(network))
}
impl TokenRegistrationJournal<'_> {
    /// Reads the record while this operation owns the journal.
    pub fn read(&self) -> Result<Option<RegistrationRecord>, TaskError> {
        self.kv
            .get(DetScope::Global, &self.key)
            .map_err(|source| TaskError::TokenCreatorJournal { source })
    }
    /// Persists before broadcast, refusing any unresolved or repeated attempt.
    pub fn claim(&self, record: &RegistrationRecord) -> Result<(), TaskError> {
        if self.key != record_key(record.network) {
            return Err(TaskError::TokenCreatorChanged);
        }
        if self.read()?.is_some_and(|current| {
            current.blocks_registration() || (current.attempt_id == record.attempt_id && current.status != crate::model::token_creator::registration::RegistrationStatus::NotSubmitted)
        }) {
            return Err(TaskError::TokenCreatorBusy);
        }
        if let Some(current) = self.read()?
            && current.status
                == crate::model::token_creator::registration::RegistrationStatus::Confirmed
        {
            self.context
                .repair_token_creation_receipt(&current)
                .map_err(|source| TaskError::TokenCreatorRepairRequired {
                    source: Box::new(source),
                })?;
        }
        self.kv
            .put(DetScope::Global, &self.key, record)
            .map_err(|source| TaskError::TokenCreatorJournal { source })
    }
    /// Settles only the attempt whose immutable public payload is still current.
    pub fn update(&self, record: &RegistrationRecord) -> Result<(), TaskError> {
        let current = self.read()?.ok_or(TaskError::TokenCreatorChanged)?;
        if current == *record {
            return Ok(());
        }
        let mut expected = current;
        expected.status = record.status;
        if expected != *record {
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
    fn identical_confirmed_update_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        let context = crate::context::test_support::test_app_context_with_kv(
            dir.path(),
            Arc::new(crate::wallet_backend::DetKv::from_store(store.clone())),
        );
        let journal = context.token_registration_journal().unwrap();
        let mut confirmed = record(1);
        confirmed.status = RegistrationStatus::Confirmed;
        journal.claim(&confirmed).unwrap();
        store.fail_next_puts_containing("token-registration", 1);
        assert!(journal.update(&confirmed).is_ok());
        assert_eq!(journal.read().unwrap(), Some(confirmed));
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

    #[test]
    fn token_registration_restart_fixture_writer() {
        let Some(path) = std::env::var_os("TOKEN_CREATOR_RESTART_FIXTURE") else {
            return;
        };
        let context = crate::context::test_support::test_app_context(std::path::Path::new(&path));
        context
            .token_registration_journal()
            .unwrap()
            .claim(&record(1))
            .unwrap();
    }

    #[test]
    fn token_registration_restart_preserves_network_bound_pending_metadata() {
        let dir = tempfile::tempdir().unwrap();
        // AppContext owns runtime services. A separate process proves the durable bytes
        // survive their complete shutdown, rather than reusing a still-open in-process store.
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "context::token_registration::tests::token_registration_restart_fixture_writer",
                "--test-threads=1",
            ])
            .env("TOKEN_CREATOR_RESTART_FIXTURE", dir.path())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "isolated writer failed: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let reopened = crate::context::test_support::test_app_context(dir.path());
        assert_eq!(
            reopened.token_registration_record().unwrap(),
            Some(record(1))
        );
        assert!(
            reopened
                .token_registration_journal()
                .unwrap()
                .claim(&record(2))
                .is_err()
        );
        assert_eq!(
            reopened
                .app_kv()
                .get::<RegistrationRecord>(
                    DetScope::Global,
                    &record_key(dash_sdk::dpp::dashcore::Network::Mainnet)
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn token_registration_settlement_cannot_change_public_binding_fields() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let journal = context.token_registration_journal().unwrap();
        let first = record(1);
        journal.claim(&first).unwrap();
        let mut changed = first.clone();
        changed.owner = Identifier::from([99; 32]);
        changed.status = RegistrationStatus::Confirmed;
        assert!(journal.update(&changed).is_err());
        assert_eq!(journal.read().unwrap(), Some(first));
    }
}
