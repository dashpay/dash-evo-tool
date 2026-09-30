//! Wallet lifecycle orchestration: the thin [`AppContext`] delegation layer.
//!
//! Each method here coordinates DET-side state (`wallets`, databases, subtasks,
//! connection status) around the wallet seam. Pure upstream-crate orchestration
//! lives in [`wallet_backend`](crate::wallet_backend) — the size here is
//! coordination surface.
//!
//! The `impl AppContext` methods are grouped by responsibility across
//! submodules, mirroring the multi-impl-of-one-struct layout `wallet_backend`
//! uses: [`prepare`] (the storage-preparation gate), [`spv`] (backend wiring /
//! chain-storage), [`registration`], [`removal`], [`bootstrap`] (address
//! derivation + post-unlock warmup), and [`unlock`] (lock/unlock handling). Shared imports, constants, the free
//! helpers, and the [`AppContext::wallet_arc`] lookup live here in `mod.rs`.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dashpay::ContactStatus;
use crate::model::spv_status::SpvStatus;
use crate::model::wallet::birth_height::{WalletOrigin, registration_birth_height};
use crate::model::wallet::meta::WalletMeta;
use crate::model::wallet::seed_envelope::StoredSeedEnvelope;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::utils::file_deletion::{DeletionIntent, delete_file, delete_tree};
use crate::wallet_backend::poison::RwLockRecover;
use crate::wallet_backend::{
    ClearAllOutcome, DetScope, WalletBackend, WalletMetaView, WalletSeedView, spv_storage_dir,
};
use dash_sdk::dpp::dashcore::Network;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

/// Number of identity-authentication keys warmed per known identity index
/// during the JIT bootstrap (D4b). Matches the readers' auth-key lookup
/// window so the common identity-load path serves entirely from cache.
const AUTH_PUBKEY_WARM_KEY_COUNT: u32 = 12;

/// The upstream `dash-spv` `DiskStorageManager` chain-cache entries under the
/// per-network SPV directory. Each is a subfolder except `peers.dat`. Only
/// these resyncable entries are ever cleared — the durable wallet databases
/// live outside this directory (see
/// [`wallet_database_path`](crate::wallet_backend::wallet_database_path)) so
/// clearing the chain cache cannot touch funds or secrets.
const SPV_CHAIN_STORAGE_ENTRIES: [&str; 7] = [
    "block_headers",
    "filter_headers",
    "filters",
    "blocks",
    "metadata",
    "masternodestate",
    "peers.dat",
];

/// How long an explicitly unlocked wallet may remain in the secret session cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletUnlockRetention {
    /// Keep the seed only until unlock-triggered registration finishes.
    OperationOnly,
    /// Keep the seed until the storage update finishes. The update's own
    /// bootstrap pass re-enters the seed scope for the wallet it just prompted
    /// for, so the unlock's reconciliation subtask must not be the sole owner of
    /// the seed's lifetime — whichever of the two finishes first would otherwise
    /// evict the seed the other still needs, and the loser re-prompts.
    UntilStorageUpdateComplete,
    /// Keep the seed available until the application closes.
    UntilAppClose,
}

/// Remove the upstream chain-sync cache files under `spv_dir`, leaving the
/// legacy shielded sidecars in that directory untouched (the durable wallet
/// databases are not in it at all — see [`SPV_CHAIN_STORAGE_ENTRIES`]). The
/// `DiskStorageManager` lock lives at `<spv_dir>.lock` (a sibling of the
/// directory); it is removed too so a stale lock cannot block the next sync.
/// A missing entry is the expected fresh/never-synced state and is tolerated.
/// Every unlink goes through the [`delete_file`] chokepoint, confined to this
/// network's SPV data by [`DeletionIntent::NetworkClear`].
fn clear_spv_chain_storage(data_dir: &Path, network: Network) -> Result<(), TaskError> {
    let intent = DeletionIntent::NetworkClear { data_dir, network };
    let spv_dir = spv_storage_dir(data_dir, network);
    for entry in SPV_CHAIN_STORAGE_ENTRIES {
        let path = spv_dir.join(entry);
        let result = match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => delete_tree(&path, intent),
            Ok(_) => delete_file(&path, intent),
            Err(error) => Err(error),
        };
        ignore_not_found(result)?;
    }
    ignore_not_found(delete_file(&spv_dir.with_extension("lock"), intent))
}

/// Treat an already-missing file as deleted; map any other failure to [`TaskError::FileSystem`].
fn ignore_not_found(result: std::io::Result<()>) -> Result<(), TaskError> {
    match result {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(TaskError::FileSystem { source: error })
        }
        _ => Ok(()),
    }
}

mod bootstrap;
mod prepare;
pub use prepare::PrepareGateGuard;
mod registration;
mod removal;
mod retention;
pub use retention::BackupPruneReport;
mod spv;
mod unlock;

impl AppContext {
    /// Resolve a loaded HD wallet by its seed hash, cloning the shared handle.
    ///
    /// The single source of truth for the "look up a wallet arc or
    /// [`TaskError::WalletNotFound`]" pattern every backend task needs. The
    /// in-memory wallet map is rebuildable (hydrated from the DB and vault), so
    /// a poisoned lock is recovered rather than surfaced as an error — matching
    /// the poison-recovery discipline used elsewhere for rebuildable state.
    pub(crate) fn wallet_arc(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Arc<RwLock<Wallet>>, TaskError> {
        self.wallet_context().wallet(seed_hash)
    }
}

/// Unlink DET's two retired legacy shielded files from `network`'s spv
/// directory, tolerating a missing file.
///
/// These are the files DET's deleted shielded subsystem owned:
/// `det-shielded.sqlite` (the plaintext note sidecar) and
/// `shielded-commitment-tree.sqlite` (the grovedb commitment tree). The
/// upstream coordinator's store (`det-<network>-shielded.sqlite`, outside this
/// directory) is a DIFFERENT file and is deliberately NOT touched here — it is
/// reset via the coordinator's own `clear_shielded`. Scoped strictly to
/// that directory — enforced by the [`delete_file`] chokepoint's
/// [`DeletionIntent::NetworkClear`] scope — so a clear of one network can never
/// reach another network's files.
fn cleanup_legacy_shielded_files(data_dir: &Path, network: Network) -> Result<(), TaskError> {
    const LEGACY_SHIELDED_FILES: [&str; 2] =
        ["det-shielded.sqlite", "shielded-commitment-tree.sqlite"];
    let intent = DeletionIntent::NetworkClear { data_dir, network };
    let spv_dir = spv_storage_dir(data_dir, network);
    for file in LEGACY_SHIELDED_FILES {
        ignore_not_found(delete_file(&spv_dir.join(file), intent))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
