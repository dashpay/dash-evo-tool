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
    PlatformWalletPersistence,
};
use platform_wallet::wallet::platform_wallet::WalletId;
use platform_wallet::wallet::shielded::SubwalletId;
use platform_wallet_storage::SqlitePersister;

use super::{DetKv, DetScope};

const SHIELDED_FVK_KEY: &str = "shielded_fvks.v1";

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
        let stored: BTreeMap<u32, Vec<u8>> = self
            .kv
            .get(DetScope::Wallet(&wallet_id), SHIELDED_FVK_KEY)
            .map_err(PersistenceError::backend)?
            .unwrap_or_default();
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
            && shielded.notes_saved.is_empty()
            && shielded.nullifiers_spent.is_empty()
            && shielded.outgoing_notes.is_empty()
            && shielded.synced_indices.is_empty()
            && shielded.activity_entries.is_empty()
            && !shielded.viewing_keys.is_empty()
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
