//! Wallet lifecycle orchestration: the thin [`AppContext`] delegation layer.
//!
//! Each method here coordinates DET-side state (`wallets`, databases, subtasks,
//! connection status) around the wallet seam. Pure upstream-crate orchestration
//! lives in [`wallet_backend`](crate::wallet_backend) — the size here is
//! coordination surface.
//!
//! The `impl AppContext` methods are grouped by responsibility across
//! submodules, mirroring the multi-impl-of-one-struct layout `wallet_backend`
//! uses: [`spv`] (backend wiring / chain-storage), [`registration`],
//! [`removal`], [`bootstrap`] (address derivation + post-unlock warmup), and
//! [`unlock`] (lock/unlock handling). Shared imports, constants, the free
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
use crate::wallet_backend::poison::RwLockRecover;
use crate::wallet_backend::{
    ClearAllOutcome, DetScope, WalletBackend, WalletMetaView, WalletSeedView, network_prefix,
};
use dash_sdk::dpp::dashcore::Network;
use std::path::{Path, PathBuf};
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

/// Per-network SPV storage directory: `<data_dir>/spv/<network>/`. Mirrors
/// `WalletBackend::resolve_spv_storage_dir` so the path resolves identically
/// whether or not the wallet backend is wired yet.
fn spv_storage_dir(data_dir: &Path, network: Network) -> PathBuf {
    data_dir.join("spv").join(network_prefix(network))
}

/// Remove the upstream chain-sync cache files under `spv_dir`, leaving the
/// legacy shielded sidecars in that directory untouched (the durable wallet
/// databases are not in it at all — see [`SPV_CHAIN_STORAGE_ENTRIES`]). The
/// `DiskStorageManager` lock lives at `<spv_dir>.lock` (a sibling of the
/// directory); it is removed too so a stale lock cannot block the next sync.
/// A missing entry is the expected fresh/never-synced state and is tolerated.
fn clear_spv_chain_storage(spv_dir: &Path) -> Result<(), TaskError> {
    for entry in SPV_CHAIN_STORAGE_ENTRIES {
        let path = spv_dir.join(entry);
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = result
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(TaskError::FileSystem { source: e });
        }
    }

    let lock_path = spv_dir.with_extension("lock");
    if let Err(e) = std::fs::remove_file(&lock_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(TaskError::FileSystem { source: e });
    }

    Ok(())
}

mod bootstrap;
mod registration;
mod removal;
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
        crate::wallet_backend::poison::read_recover(&self.wallets)
            .get(seed_hash)
            .cloned()
            .ok_or(TaskError::WalletNotFound)
    }
}

/// Unlink DET's two retired legacy shielded files from `spv_dir` (the active
/// network's spv directory), tolerating a missing file.
///
/// These are the files DET's deleted shielded subsystem owned:
/// `det-shielded.sqlite` (the plaintext note sidecar) and
/// `shielded-commitment-tree.sqlite` (the grovedb commitment tree). The
/// upstream coordinator's store (`det-<network>-shielded.sqlite`, outside this
/// directory) is a DIFFERENT file and is deliberately NOT touched here — it is
/// reset via the coordinator's own `clear_shielded`. Scoped strictly to
/// `spv_dir` so a clear of one network can never reach another network's files.
fn cleanup_legacy_shielded_files(spv_dir: &Path) -> Result<(), TaskError> {
    const LEGACY_SHIELDED_FILES: [&str; 2] =
        ["det-shielded.sqlite", "shielded-commitment-tree.sqlite"];
    for file in LEGACY_SHIELDED_FILES {
        let path = spv_dir.join(file);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(TaskError::FileSystem { source: e });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
