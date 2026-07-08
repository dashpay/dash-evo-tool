//! Persisted-wallet load seam (G2).
//!
//! `PersistedWalletLoader` decouples the *strategy* for bringing
//! persisted wallets back at startup from `WalletBackend`. The shipping
//! impl is [`UpstreamFromPersisted`], which drives the upstream
//! **seedless / watch-only** rehydration API
//! (`PlatformWalletManager::load_from_persistor`, PR #3692): for every
//! wallet **already present in the upstream persistor**, balances, UTXOs,
//! identities, and contacts come back at launch with no seed in memory.
//! Signing keys enter memory later, on demand, when a signing operation
//! pulls the seed just-in-time through the
//! [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint.
//!
//! This loader is **read-only**: it does NOT register or re-register
//! wallets. It can only return what the persistor already holds. The
//! persistor is populated at the two seed-bearing moments —
//! `WalletBackend::register_wallet_from_seed` (W1, create/import) and
//! `WalletBackend::ensure_upstream_registered` (W2, cold-boot
//! reconciliation). If those have never run for a wallet (fresh install,
//! post-reset, migrated/sidecar-only), the persistor is empty for it and
//! this loader brings back nothing — exactly the funds-invisible
//! state the W1/W2 writers exist to prevent.
//!
//! The trait stays object-safe (`Arc<dyn PersistedWalletLoader>`) and
//! its outputs are DET-opaque: [`LoadedWallets`] carries only DET's
//! [`WalletSeedHash`] keys and a DET-side [`PersistedLoadSkip`] reason —
//! no `platform_wallet` / `key_wallet` type crosses the seam.

use std::sync::Arc;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;

use super::WalletBackend;

/// Outcome of a persisted-wallet load pass, mapped to DET-opaque types.
///
/// Carries no upstream `platform-wallet` type. A non-empty `skipped`
/// list is still a **success**: a single corrupt persisted row is
/// reported and the remaining wallets load. A whole-load failure is a
/// `TaskError` from [`PersistedWalletLoader::load`], not a populated
/// `skipped`.
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

/// Brings persisted wallets back into the running backend at startup.
///
/// Object-safe so `WalletBackend` can hold `Arc<dyn PersistedWalletLoader>`
/// and swap impls behind a one-line construction change (mirrors the
/// single-key `SingleKeyView` seam).
#[async_trait::async_trait]
pub trait PersistedWalletLoader: Send + Sync {
    /// Perform the load pass and report which wallets came back.
    ///
    /// `backend` is the orchestration layer the impl drives; `ctx`
    /// supplies the active network and DET sidecars. The impl must map
    /// every upstream identifier to a DET [`WalletSeedHash`] before
    /// returning — no upstream type may appear in [`LoadedWallets`].
    async fn load(
        &self,
        backend: &WalletBackend,
        ctx: &Arc<AppContext>,
    ) -> Result<LoadedWallets, TaskError>;
}

/// Seedless watch-only loader over the upstream PR #3692 rehydration API.
///
/// Delegates to [`WalletBackend::load_from_persistor_seedless`]: one
/// `load_from_persistor()` call, then each returned upstream wallet is
/// resolved to its DET [`WalletSeedHash`] by matching the loaded
/// watch-only wallet's BIP44 account xpub against the `xpub_encoded` DET
/// persisted in its sidecar. A loaded wallet that matches no sidecar
/// entry is rejected (fund-routing gate, never displayed) — see
/// [`WalletBackend::load_from_persistor_seedless`] for the gate.
pub struct UpstreamFromPersisted;

impl UpstreamFromPersisted {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpstreamFromPersisted {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PersistedWalletLoader for UpstreamFromPersisted {
    async fn load(
        &self,
        backend: &WalletBackend,
        ctx: &Arc<AppContext>,
    ) -> Result<LoadedWallets, TaskError> {
        backend.load_from_persistor_seedless(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the swap boundary: an alternate loader impl drops into
    /// `Arc<dyn PersistedWalletLoader>` with no other change. The
    /// behavioral assertions (real load, the xpub bridge, the
    /// account-xpub-match gate) live alongside the backend impl and the
    /// backend-e2e cold-boot lane, which can stand up a real
    /// `WalletBackend`.
    #[test]
    fn swap_boundary_compiles_with_alternate_impl() {
        struct StubLoader;
        #[async_trait::async_trait]
        impl PersistedWalletLoader for StubLoader {
            async fn load(
                &self,
                _backend: &WalletBackend,
                _ctx: &Arc<AppContext>,
            ) -> Result<LoadedWallets, TaskError> {
                Ok(LoadedWallets::default())
            }
        }
        let _stub: Arc<dyn PersistedWalletLoader> = Arc::new(StubLoader);
        let _shipping: Arc<dyn PersistedWalletLoader> = Arc::new(UpstreamFromPersisted::new());
    }

    /// `LoadedWallets::default` is the empty, no-wallet shape — the
    /// outcome of a cold boot with nothing persisted.
    #[test]
    fn loaded_wallets_default_is_empty() {
        let lw = LoadedWallets::default();
        assert!(lw.loaded.is_empty());
        assert!(lw.skipped.is_empty());
    }
}
