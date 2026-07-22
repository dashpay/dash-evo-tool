//! DET adapter for upstream wallet persistence capabilities.
//!
//! The upstream SQLite persister owns wallet state, while DET adds the one
//! host-specific persistence path it needs for seedless shielded restart:
//! wallet-scoped Orchard full-viewing keys stored in the same SQLite file's
//! metadata table.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use platform_wallet::changeset::{
    ClientStartState, PersistenceCapabilities, PersistenceError, PlatformWalletChangeSet,
    PlatformWalletPersistence, ShieldedChangeSet,
};
use platform_wallet::wallet::platform_wallet::WalletId;
use platform_wallet::wallet::shielded::SubwalletId;
use platform_wallet_storage::{KvError, SqlitePersister};

use super::{DetKv, DetScope, KvAdapterError};

const SHIELDED_FVK_KEY: &str = "shielded:fvks:v1";

/// Delete one wallet's persisted shielded full-viewing keys.
pub(super) fn forget_wallet_viewing_keys(
    kv: &DetKv,
    wallet_id: WalletId,
) -> Result<(), KvAdapterError> {
    kv.delete(DetScope::Wallet(&wallet_id), SHIELDED_FVK_KEY)
}

/// Upstream wallet persistence plus DET's wallet-scoped shielded FVK rows.
pub(super) struct DetPersister {
    inner: Arc<SqlitePersister>,
    kv: DetKv,
    shielded_fvk_lock: Mutex<()>,
}

impl DetPersister {
    pub(super) fn new(inner: Arc<SqlitePersister>) -> Self {
        Self {
            kv: DetKv::new(Arc::clone(&inner)),
            inner,
            shielded_fvk_lock: Mutex::new(()),
        }
    }

    fn store_viewing_keys(
        &self,
        wallet_id: WalletId,
        incoming: &BTreeMap<SubwalletId, Vec<u8>>,
    ) -> Result<(), PersistenceError> {
        let _guard = self
            .shielded_fvk_lock
            .lock()
            .map_err(|_| PersistenceError::LockPoisoned)?;
        let mut stored: BTreeMap<u32, Vec<u8>> = self
            .kv
            .get(DetScope::Wallet(&wallet_id), SHIELDED_FVK_KEY)
            .map_err(PersistenceError::backend)?
            .unwrap_or_default();

        for (subwallet_id, fvk) in incoming {
            if subwallet_id.wallet_id != wallet_id {
                return Err(PersistenceError::backend(std::io::Error::other(
                    "shielded viewing-key wallet id does not match changeset wallet id",
                )));
            }
            stored.insert(subwallet_id.account_index, fvk.clone());
        }

        self.kv
            .put(DetScope::Wallet(&wallet_id), SHIELDED_FVK_KEY, &stored)
            .map_err(PersistenceError::backend)
    }

    fn load_viewing_keys(
        &self,
        wallet_id: WalletId,
    ) -> Result<BTreeMap<SubwalletId, Vec<u8>>, PersistenceError> {
        let stored: BTreeMap<u32, Vec<u8>> =
            match self.kv.get(DetScope::Wallet(&wallet_id), SHIELDED_FVK_KEY) {
                Ok(stored) => stored.unwrap_or_default(),
                Err(
                    error @ (KvAdapterError::SchemaVersion { .. }
                    | KvAdapterError::Truncated
                    | KvAdapterError::Decode(_)
                    | KvAdapterError::Store(KvError::ValueTooLarge { .. })),
                ) => {
                    tracing::warn!(
                        target: "wallet_backend::persister",
                        wallet_id = %hex::encode(wallet_id),
                        key = SHIELDED_FVK_KEY,
                        error = ?error,
                        "Skipping unreadable shielded viewing-key row",
                    );
                    BTreeMap::new()
                }
                Err(error) => return Err(PersistenceError::backend(error)),
            };
        Ok(stored
            .into_iter()
            .map(|(account, fvk)| (SubwalletId::new(wallet_id, account), fvk))
            .collect())
    }

    /// The FVK adapter owns only viewing-key deltas. Reject a mixed changeset
    /// rather than splitting one logical commit across the typed SQLite tables
    /// and the metadata table, which would violate `ATOMIC_CHANGESETS`.
    fn is_viewing_key_only(changeset: &PlatformWalletChangeSet) -> bool {
        let PlatformWalletChangeSet {
            core,
            identities,
            identity_keys,
            contacts,
            platform_addresses,
            asset_locks,
            invitations,
            token_balances,
            dashpay_profiles,
            dashpay_payments_overlay,
            wallet_metadata,
            account_registrations,
            provider_key_account_registrations,
            account_address_pools,
            pending_contact_crypto_added,
            pending_contact_crypto_cleared,
            shielded,
        } = changeset;
        let Some(shielded) = shielded else {
            return false;
        };
        let ShieldedChangeSet {
            notes_saved,
            nullifiers_spent,
            outgoing_notes,
            synced_indices,
            activity_entries,
            viewing_keys,
        } = shielded;

        core.is_none()
            && identities.is_none()
            && identity_keys.is_none()
            && contacts.is_none()
            && platform_addresses.is_none()
            && asset_locks.is_none()
            && invitations.is_none()
            && token_balances.is_none()
            && dashpay_profiles.is_none()
            && dashpay_payments_overlay.is_none()
            && wallet_metadata.is_none()
            && account_registrations.is_empty()
            && provider_key_account_registrations.is_empty()
            && account_address_pools.is_empty()
            && pending_contact_crypto_added.is_empty()
            && pending_contact_crypto_cleared.is_empty()
            && notes_saved.is_empty()
            && nullifiers_spent.is_empty()
            && outgoing_notes.is_empty()
            && synced_indices.is_empty()
            && activity_entries.is_empty()
            && !viewing_keys.is_empty()
    }
}

impl PlatformWalletPersistence for DetPersister {
    fn persistence_capabilities(&self) -> PersistenceCapabilities {
        self.inner
            .persistence_capabilities()
            .union(PersistenceCapabilities::SHIELDED_VIEWING_KEYS)
    }

    fn store(
        &self,
        wallet_id: WalletId,
        changeset: PlatformWalletChangeSet,
    ) -> Result<(), PersistenceError> {
        if let Some(shielded) = changeset.shielded.as_ref()
            && !shielded.viewing_keys.is_empty()
        {
            if !Self::is_viewing_key_only(&changeset) {
                return Err(PersistenceError::backend(std::io::Error::other(
                    "shielded viewing keys must be persisted in a dedicated changeset",
                )));
            }
            self.store_viewing_keys(wallet_id, &shielded.viewing_keys)?;
            return Ok(());
        }
        self.inner.store(wallet_id, changeset)
    }

    fn flush(&self, wallet_id: WalletId) -> Result<(), PersistenceError> {
        self.inner.flush(wallet_id)
    }

    fn load(&self) -> Result<ClientStartState, PersistenceError> {
        let mut start = self.inner.load()?;
        for wallet_id in start.wallets.keys().copied() {
            start
                .shielded
                .viewing_keys
                .extend(self.load_viewing_keys(wallet_id)?);
        }
        Ok(start)
    }

    fn delete_wallet(&self, wallet_id: WalletId) -> Result<(), PersistenceError> {
        PlatformWalletPersistence::delete_wallet(self.inner.as_ref(), wallet_id)
    }

    fn get_core_tx_record(
        &self,
        wallet_id: WalletId,
        txid: &dash_sdk::dpp::dashcore::Txid,
    ) -> Result<
        Option<dash_sdk::dpp::key_wallet::managed_account::transaction_record::TransactionRecord>,
        PersistenceError,
    > {
        self.inner.get_core_tx_record(wallet_id, txid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use platform_wallet::changeset::{
        PersistenceErrorKind, PlatformWalletChangeSet, ShieldedChangeSet,
    };
    use platform_wallet::wallet::shielded::ShieldedNote;
    use platform_wallet_storage::{KvStore, ObjectId, SqlitePersisterConfig};

    struct FailingReadKv {
        oversized: bool,
    }

    impl KvStore for FailingReadKv {
        fn get(&self, _scope: &ObjectId, _key: &str) -> Result<Option<Vec<u8>>, KvError> {
            if self.oversized {
                Err(KvError::ValueTooLarge { found: 2, max: 1 })
            } else {
                Err(KvError::LockPoisoned)
            }
        }

        fn put(&self, _scope: &ObjectId, _key: &str, _value: &[u8]) -> Result<(), KvError> {
            Ok(())
        }

        fn delete(&self, _scope: &ObjectId, _key: &str) -> Result<(), KvError> {
            Ok(())
        }

        fn list_keys(
            &self,
            _scope: &ObjectId,
            _prefix: Option<&str>,
        ) -> Result<Vec<String>, KvError> {
            Ok(Vec::new())
        }
    }

    fn test_persister() -> (tempfile::TempDir, DetPersister) {
        let dir = tempfile::tempdir().expect("persister tempdir");
        crate::app_dir::ensure_data_dir_exists(dir.path())
            .expect("restrict persister tempdir permissions");
        let inner = Arc::new(
            SqlitePersister::open(SqlitePersisterConfig::new(
                dir.path().join("platform-wallet.sqlite"),
            ))
            .expect("open test persister"),
        );
        (dir, DetPersister::new(inner))
    }

    fn viewing_key_changeset(
        wallet_id: WalletId,
        account: u32,
        fvk: Vec<u8>,
    ) -> PlatformWalletChangeSet {
        let mut shielded = ShieldedChangeSet::default();
        shielded
            .viewing_keys
            .insert(SubwalletId::new(wallet_id, account), fvk);
        PlatformWalletChangeSet {
            shielded: Some(shielded),
            ..Default::default()
        }
    }

    #[test]
    fn is_viewing_key_only_classifies_supported_shapes() {
        let wallet_id = [0x11; 32];
        let subwallet_id = SubwalletId::new(wallet_id, 0);

        let viewing_only = viewing_key_changeset(wallet_id, 0, vec![0xA1; 96]);

        let mut viewing_and_synced = viewing_key_changeset(wallet_id, 0, vec![0xA2; 96]);
        viewing_and_synced
            .shielded
            .as_mut()
            .expect("shielded changeset")
            .synced_indices
            .insert(subwallet_id, 7);

        let empty = PlatformWalletChangeSet::default();

        let mut synced_without_viewing = ShieldedChangeSet::default();
        synced_without_viewing
            .synced_indices
            .insert(subwallet_id, 8);
        let synced_without_viewing = PlatformWalletChangeSet {
            shielded: Some(synced_without_viewing),
            ..Default::default()
        };

        for (case, changeset, expected) in [
            ("viewing keys only", viewing_only, true),
            ("viewing keys plus another field", viewing_and_synced, false),
            ("no shielded changeset", empty, false),
            (
                "shielded delta without viewing keys",
                synced_without_viewing,
                false,
            ),
        ] {
            assert_eq!(
                DetPersister::is_viewing_key_only(&changeset),
                expected,
                "case: {case}"
            );
        }
    }

    #[test]
    fn viewing_keys_round_trip_through_store() {
        let (_dir, persister) = test_persister();
        let wallet_id = [0x21; 32];
        let subwallet_id = SubwalletId::new(wallet_id, 3);
        let fvk = vec![0xB1; 96];

        persister
            .store(
                wallet_id,
                viewing_key_changeset(wallet_id, subwallet_id.account_index, fvk.clone()),
            )
            .expect("store viewing key");

        assert_eq!(
            persister
                .load_viewing_keys(wallet_id)
                .expect("load viewing key"),
            BTreeMap::from([(subwallet_id, fvk)])
        );
    }

    #[test]
    fn delete_wallet_delegates_to_inner_persister() {
        let (_dir, persister) = test_persister();
        let error = persister
            .delete_wallet([0x25; 32])
            .expect_err("an unknown wallet must reach the inner persister");

        assert!(
            matches!(error, PersistenceError::Backend { .. }),
            "the wrapper must not fall back to UnsupportedOperation: {error:?}"
        );
    }

    #[test]
    fn load_viewing_keys_skips_oversized_values_but_propagates_backend_failures() {
        let wallet_id = [0x29; 32];
        let (_dir, mut persister) = test_persister();
        persister.kv = DetKv::from_store(Arc::new(FailingReadKv { oversized: true }));
        assert!(
            persister
                .load_viewing_keys(wallet_id)
                .expect("oversized row must degrade to absent")
                .is_empty()
        );

        let (_dir, mut persister) = test_persister();
        persister.kv = DetKv::from_store(Arc::new(FailingReadKv { oversized: false }));
        let error = persister
            .load_viewing_keys(wallet_id)
            .expect_err("backend failure must remain fatal");
        match error {
            PersistenceError::Backend { kind, source } => {
                assert_eq!(kind, PersistenceErrorKind::Fatal);
                assert!(matches!(
                    source.downcast_ref::<KvAdapterError>(),
                    Some(KvAdapterError::Store(KvError::LockPoisoned))
                ));
            }
            other => panic!("unexpected persistence error: {other:?}"),
        }
    }

    #[test]
    fn mixed_viewing_key_and_note_changeset_is_rejected_without_partial_write() {
        let (_dir, persister) = test_persister();
        let wallet_id = [0x31; 32];
        let subwallet_id = SubwalletId::new(wallet_id, 0);
        let mut changeset = viewing_key_changeset(wallet_id, 0, vec![0xC1; 96]);
        changeset
            .shielded
            .as_mut()
            .expect("shielded changeset")
            .notes_saved
            .insert(
                subwallet_id,
                vec![ShieldedNote {
                    position: 1,
                    cmx: [0xC2; 32],
                    nullifier: [0xC3; 32],
                    block_height: 2,
                    is_spent: false,
                    value: 3,
                    note_data: vec![0xC4; 115],
                }],
            );

        let error = persister
            .store(wallet_id, changeset)
            .expect_err("mixed changeset must be rejected");
        match error {
            PersistenceError::Backend { kind, source } => {
                assert_eq!(kind, PersistenceErrorKind::Fatal);
                assert_eq!(
                    source.to_string(),
                    "shielded viewing keys must be persisted in a dedicated changeset"
                );
            }
            other => panic!("unexpected persistence error: {other:?}"),
        }
        assert!(
            persister
                .load_viewing_keys(wallet_id)
                .expect("load after rejection")
                .is_empty(),
            "a rejected mixed changeset must not partially persist viewing keys"
        );
    }

    #[test]
    fn viewing_key_for_another_wallet_is_rejected_without_partial_write() {
        let (_dir, persister) = test_persister();
        let wallet_id = [0x41; 32];
        let other_wallet_id = [0x42; 32];
        let changeset = viewing_key_changeset(other_wallet_id, 0, vec![0xD1; 96]);

        let error = persister
            .store(wallet_id, changeset)
            .expect_err("cross-wallet viewing key must be rejected");
        match error {
            PersistenceError::Backend { kind, source } => {
                assert_eq!(kind, PersistenceErrorKind::Fatal);
                assert_eq!(
                    source.to_string(),
                    "shielded viewing-key wallet id does not match changeset wallet id"
                );
            }
            other => panic!("unexpected persistence error: {other:?}"),
        }
        assert!(
            persister
                .load_viewing_keys(wallet_id)
                .expect("load after rejection")
                .is_empty(),
            "a rejected cross-wallet changeset must not persist viewing keys"
        );
    }
}
