//! Restart-safe record of a "Max" transfer from the Core wallet to the
//! shielded balance.
//!
//! One record per wallet and network, stored in the DET k/v sidecar under
//!
//! ```text
//! <network>:shielded_max_transfer:<seed_hash_base58>
//! ```
//!
//! scoped to [`DetScope::Wallet`](crate::wallet_backend::DetScope::Wallet) so
//! it cascades with the wallet. The record holds the upstream operation id and
//! is written BEFORE the transfer starts, so a crash or restart at any point
//! still leaves the id to query or finish the transfer with. It is removed only
//! once the transfer has an outcome.
//!
//! Unlike the cache sidecars, a record that cannot be read is an error, never
//! "absent": reading it as absent would let a second transfer start and lose
//! the id of the first.

use std::sync::{Arc, Mutex, PoisonError};

use dash_sdk::dpp::dashcore::Network;

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::shielded_max_transfer::PendingShieldedMaxTransfer;
use crate::wallet_backend::DetKv;
use crate::wallet_backend::kv::{KvAdapterError, map_kv_storage_error};
use crate::wallet_backend::sidecar::{SidecarScope, SidecarValue, SidecarView};

/// Colon-separated namespace of the per-wallet record.
const KEY_INFIX: &str = ":shielded_max_transfer:";

impl SidecarValue for PendingShieldedMaxTransfer {}

/// Outcome of [`ShieldedMaxTransferRecords::claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaxTransferClaim {
    /// The record was written; the caller owns the transfer.
    Claimed,
    /// An earlier transfer is still unsettled; nothing was written.
    AlreadyPending(PendingShieldedMaxTransfer),
}

/// Typed view over the Max transfer records.
///
/// `claim` and `release` are read-then-write cycles, so both run under
/// `write_lock`: two concurrent claims cannot both win, and a release cannot
/// delete a record another transfer claimed in between. The lock is a `std`
/// mutex held only for the synchronous k/v calls, never across an `.await`.
pub(crate) struct ShieldedMaxTransferRecords<'a> {
    sidecar: SidecarView<'a, PendingShieldedMaxTransfer>,
    write_lock: &'a Mutex<()>,
}

impl<'a> ShieldedMaxTransferRecords<'a> {
    pub(crate) fn new(kv: &'a Arc<DetKv>, write_lock: &'a Mutex<()>) -> Self {
        Self {
            sidecar: SidecarView::new(
                kv,
                KEY_INFIX,
                SidecarScope::WalletById,
                map_kv_error_to_task_error,
            ),
            write_lock,
        }
    }

    /// The wallet's unsettled transfer, if any.
    pub(crate) fn pending(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
    ) -> Result<Option<PendingShieldedMaxTransfer>, TaskError> {
        self.sidecar.try_get(network, seed_hash)
    }

    /// Store `record` unless the wallet already has an unsettled transfer.
    pub(crate) fn claim(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        record: PendingShieldedMaxTransfer,
    ) -> Result<MaxTransferClaim, TaskError> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = self.sidecar.try_get(network, seed_hash)? {
            return Ok(MaxTransferClaim::AlreadyPending(existing));
        }
        self.sidecar.set(network, seed_hash, &record)?;
        Ok(MaxTransferClaim::Claimed)
    }

    /// Remove the record of the settled transfer `operation_id`. A record of
    /// any other transfer is kept; a missing record is not an error.
    pub(crate) fn release(
        &self,
        network: Network,
        seed_hash: &WalletSeedHash,
        operation_id: [u8; 16],
    ) -> Result<(), TaskError> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match self.sidecar.try_get(network, seed_hash)? {
            Some(existing) if existing.operation_id == operation_id => {
                self.sidecar.delete(network, seed_hash)
            }
            _ => Ok(()),
        }
    }
}

fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    map_kv_storage_error(e, |source| TaskError::KvSidecarStorage {
        sidecar: "shielded_max_transfer",
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::kv_test_support::InMemoryKv;

    fn kv() -> Arc<DetKv> {
        Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    fn record(byte: u8) -> PendingShieldedMaxTransfer {
        PendingShieldedMaxTransfer {
            operation_id: [byte; 16],
        }
    }

    const SEED: WalletSeedHash = [0x5A; 32];

    #[test]
    fn a_claimed_transfer_blocks_a_second_one_until_released() {
        let kv = kv();
        let lock = Mutex::new(());
        let records = ShieldedMaxTransferRecords::new(&kv, &lock);

        assert_eq!(records.pending(Network::Testnet, &SEED).unwrap(), None);
        assert_eq!(
            records.claim(Network::Testnet, &SEED, record(1)).unwrap(),
            MaxTransferClaim::Claimed
        );
        assert_eq!(
            records.claim(Network::Testnet, &SEED, record(2)).unwrap(),
            MaxTransferClaim::AlreadyPending(record(1)),
            "a second transfer must never overwrite the first one's id"
        );
        assert_eq!(
            records.pending(Network::Testnet, &SEED).unwrap(),
            Some(record(1))
        );

        records
            .release(Network::Testnet, &SEED, record(1).operation_id)
            .unwrap();
        assert_eq!(records.pending(Network::Testnet, &SEED).unwrap(), None);
        assert_eq!(
            records.claim(Network::Testnet, &SEED, record(2)).unwrap(),
            MaxTransferClaim::Claimed
        );
    }

    #[test]
    fn releasing_another_transfer_keeps_the_record() {
        let kv = kv();
        let lock = Mutex::new(());
        let records = ShieldedMaxTransferRecords::new(&kv, &lock);
        records.claim(Network::Testnet, &SEED, record(1)).unwrap();

        records
            .release(Network::Testnet, &SEED, record(9).operation_id)
            .unwrap();
        assert_eq!(
            records.pending(Network::Testnet, &SEED).unwrap(),
            Some(record(1))
        );
        // Releasing with nothing stored is a no-op.
        records
            .release(Network::Mainnet, &SEED, record(1).operation_id)
            .unwrap();
    }

    #[test]
    fn records_are_separate_per_network_and_wallet() {
        let kv = kv();
        let lock = Mutex::new(());
        let records = ShieldedMaxTransferRecords::new(&kv, &lock);
        records.claim(Network::Testnet, &SEED, record(1)).unwrap();

        assert_eq!(records.pending(Network::Mainnet, &SEED).unwrap(), None);
        assert_eq!(
            records.pending(Network::Testnet, &[0x01; 32]).unwrap(),
            None
        );
        assert_eq!(
            records.claim(Network::Mainnet, &SEED, record(2)).unwrap(),
            MaxTransferClaim::Claimed
        );
    }

    #[test]
    fn an_unreadable_record_is_an_error_not_absent() {
        use platform_wallet_storage::{KvStore, ObjectId};
        let store = Arc::new(InMemoryKv::default());
        let key = crate::wallet_backend::sidecar::sidecar_key(Network::Testnet, KEY_INFIX, &SEED);
        // A blob from a future schema version, planted past the adapter.
        store
            .put(&ObjectId::Wallet(SEED), &key, &[0xFF, 1])
            .unwrap();
        let kv = Arc::new(DetKv::from_store(store));
        let lock = Mutex::new(());
        let records = ShieldedMaxTransferRecords::new(&kv, &lock);

        assert!(matches!(
            records.pending(Network::Testnet, &SEED),
            Err(TaskError::KvSidecarStorage { .. })
        ));
        assert!(
            records.claim(Network::Testnet, &SEED, record(2)).is_err(),
            "an unreadable record must not be replaced by a new claim"
        );
    }
}
