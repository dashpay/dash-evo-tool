//! Persisted-wallet load seam (G2) — DET-opaque outcome types.
//!
//! [`WalletBackend::load_from_persistor_seedless`](super::WalletBackend::load_from_persistor_seedless)
//! drives the upstream **seedless / watch-only** rehydration API
//! (`PlatformWalletManager::load_from_persistor`, PR #3692): for every
//! wallet **already present in the upstream persistor**, balances, UTXOs,
//! identities, and contacts come back at launch with no seed in memory.
//! Signing keys enter memory later, on demand, when a signing operation
//! pulls the seed just-in-time through the
//! [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint.
//!
//! That load is **read-only**: it does NOT register or re-register
//! wallets. It can only return what the persistor already holds. The
//! persistor is populated at the two seed-bearing moments —
//! `WalletBackend::register_wallet_from_seed` (W1, create/import) and
//! `WalletBackend::ensure_upstream_registered` (W2, cold-boot
//! reconciliation). If those have never run for a wallet (fresh install,
//! post-reset, migrated/sidecar-only), the persistor is empty for it and
//! the load brings back nothing — exactly the funds-invisible state the
//! W1/W2 writers exist to prevent.
//!
//! The outcome types here are DET-opaque: [`LoadedWallets`] carries only
//! DET's [`WalletSeedHash`] keys and a DET-side [`PersistedLoadSkip`]
//! reason — no `platform_wallet` / `key_wallet` type crosses the seam.

use crate::model::wallet::WalletSeedHash;

/// Outcome of a persisted-wallet load pass, mapped to DET-opaque types.
///
/// Carries no upstream `platform-wallet` type. A non-empty `skipped`
/// list is still a **success**: a single corrupt persisted row is
/// reported and the remaining wallets load. A whole-load failure is a
/// `TaskError` from [`WalletBackend::load_from_persistor_seedless`], not
/// a populated `skipped`.
///
/// [`WalletBackend::load_from_persistor_seedless`]: super::WalletBackend::load_from_persistor_seedless
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadedWallets {
    /// Wallets now registered with the backend, keyed by DET's seed hash.
    pub loaded: Vec<WalletSeedHash>,
    /// Wallets present on disk but skipped because their persisted row
    /// was unusable. The [`WalletSeedHash`] is present only when the
    /// skipped wallet could be matched to a DET sidecar entry; an
    /// unmatched skip carries `None`.
    pub skipped: Vec<(Option<WalletSeedHash>, PersistedLoadSkip)>,
}

/// DET-opaque reason a persisted wallet row was skipped during load.
///
/// Mirrors the upstream corrupt-row families without leaking the
/// upstream `CorruptKind` type or any row-derived bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistedLoadSkip {
    /// The wallet row has no usable account manifest to rebuild from.
    MissingManifest,
    /// The persisted account xpub failed to parse.
    MalformedXpub,
    /// Any other structural decode/projection failure on the row.
    DecodeError,
}
