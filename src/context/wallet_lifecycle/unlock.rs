//! Lock / unlock handling: reacting to a wallet becoming unlocked or locked
//! and snapshotting the set of open wallets.

use super::*;

impl AppContext {
    /// Reconcile a password-protected wallet after an explicit unlock gesture.
    ///
    /// Since the JIT migration this is **not** a seed-distribution point —
    /// signing pulls the seed just-in-time from the encrypted vault through
    /// the [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint.
    /// It promotes the just-verified seed through the secret chokepoint, then
    /// re-drives the JIT bootstrap so the wallet is upstream-registered this
    /// session. Operation-only unlocks forget the temporary cache entry once
    /// that reconciliation finishes.
    ///
    /// `passphrase` is the secret the UI just validated via
    /// [`WalletSeed::open`](crate::model::wallet::WalletSeed::open). Callers
    /// invoke this after every successful password-wallet unlock. `retention`
    /// controls whether the temporary seed remains available afterwards.
    ///
    /// The seed is obtained ONLY by decrypting the stored envelope through the
    /// chokepoint — no parked seed is read, because an open `Wallet` parks none
    /// (R3). Shielded state is not warmed here: it is derived on the first
    /// shielded operation via the chokepoint.
    pub fn handle_wallet_unlocked(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        passphrase: &str,
        retention: WalletUnlockRetention,
    ) {
        let (seed_hash, uses_password) = match wallet.read() {
            Ok(guard) => (guard.seed_hash(), guard.uses_password),
            Err(_) => return,
        };

        // No-password wallets need no promotion — they resolve prompt-free
        // through the chokepoint's unprotected fast-path.
        if !uses_password {
            return;
        }

        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let secret = platform_wallet_storage::secrets::SecretString::new(passphrase);
        if let Err(error) = backend.secret_access().promote_hd_seed_with_passphrase(
            &seed_hash,
            Some(&secret),
            crate::wallet_backend::RememberPolicy::UntilAppClose,
        ) {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                %error,
                "Unlock seed promotion skipped"
            );
            return;
        }
        tracing::trace!(
            wallet = %hex::encode(seed_hash),
            "Verified-open seed promoted to the session cache on unlock"
        );

        // W2 reconciliation on the unlock gesture. A
        // password-protected wallet hydrates `Closed` at cold boot, so
        // `bootstrap_wallet_addresses_jit` skips it (no surprise startup prompt)
        // and it is never upstream-registered until the seed becomes available.
        // The unlock just verified the passphrase and promoted the seed into the
        // session cache above, so re-driving the JIT bootstrap now registers the
        // wallet with the upstream SPV backend without a second prompt — the
        // difference between the wallet being usable this session and a
        // `WalletNotLoaded` until the next launch. Idempotent (an
        // already-registered wallet is a no-op) and resolved prompt-free from the
        // session cache. The in-memory wallet is already flipped `Open` by the
        // unlock callsite before this runs, so the JIT `is_open()` gate passes.
        self.drive_unlock_registration(wallet, retention);
    }

    /// Spawn the unlock-triggered JIT bootstrap/registration for a wallet whose
    /// seed was just promoted to the session cache by [`Self::handle_wallet_unlocked`].
    ///
    /// `handle_wallet_unlocked` is synchronous (called from the UI thread) while
    /// [`Self::bootstrap_wallet_addresses_jit`] is async, so the reconciliation
    /// runs on a tracked subtask — mirroring [`Self::register_wallet_upstream`].
    /// Best-effort: the JIT bootstrap logs and swallows its own failures, and a
    /// missing-backend cold-boot path is covered by `bootstrap_loaded_wallets`.
    fn drive_unlock_registration(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        retention: WalletUnlockRetention,
    ) {
        let ctx = Arc::clone(self);
        let wallet = Arc::clone(wallet);
        self.subtasks
            .spawn_sync("wallet_unlock_registration", async move {
                ctx.bootstrap_wallet_addresses_jit(&wallet).await;
                ctx.discover_unlocked_wallet_identities(&wallet).await;
                if retention == WalletUnlockRetention::OperationOnly
                    && let Ok(backend) = ctx.wallet_backend()
                    && let Ok(seed_hash) = wallet.read().map(|wallet| wallet.seed_hash())
                {
                    backend
                        .secret_access()
                        .forget(&crate::wallet_backend::SecretScope::HdSeed { seed_hash });
                }
            });
    }

    /// Wipe the session-cached seed when a wallet is locked.
    ///
    /// Unlock promotes the seed into the JIT session cache with
    /// [`RememberPolicy::UntilAppClose`](crate::wallet_backend::RememberPolicy),
    /// and signing resolves cache-first — so without this the wallet would keep
    /// signing prompt-free after a "lock", leaving plaintext seed bytes
    /// resident. Forgetting the seed's scope here restores the locked
    /// guarantee: the next signing op re-prompts (or, for a no-password wallet,
    /// resolves through the chokepoint's unprotected fast-path). Mirrors
    /// [`Self::handle_wallet_unlocked`]'s promotion, in reverse.
    pub fn handle_wallet_locked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        let Ok(seed_hash) = wallet.read().map(|guard| guard.seed_hash()) else {
            return;
        };
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        backend
            .secret_access()
            .forget(&crate::wallet_backend::SecretScope::HdSeed { seed_hash });
        tracing::trace!(
            wallet = %hex::encode(seed_hash),
            "Session-cached seed wiped on wallet lock"
        );
    }

    /// Bind Orchard ZIP-32 keys for all currently-open wallets that have not
    /// yet been shielded-bound through the upstream coordinator.  Called when
    /// the network protocol version first crosses the shielded threshold —
    /// at that point open wallets are typically already bootstrapped and
    /// registered, so the regular JIT path in
    /// [`Self::bootstrap_wallet_addresses_jit`] would have been the right
    /// vehicle, but it may not have run yet for wallets opened before the
    /// version was known.
    ///
    /// Reuses `bootstrap_wallet_addresses_jit` (which now unconditionally
    /// calls `ensure_shielded_bound`) so the logic is not duplicated.
    /// The upstream 60 s `ShieldedSyncManager` loop picks up any newly bound
    /// wallets automatically — no manual sync trigger needed.
    /// Snapshot the currently-open wallet arcs, dropping the read lock before
    /// returning. A locked protected wallet hydrates `WalletSeed::Closed`, so
    /// `is_open()` excludes it — the single source of truth for "which wallets a
    /// background pass may touch without a passphrase prompt."
    pub(super) fn open_wallets(self: &Arc<Self>) -> Vec<Arc<RwLock<Wallet>>> {
        self.wallets
            .read()
            .ok()
            .map(|wallets| {
                wallets
                    .values()
                    .filter(|w| w.read().ok().map(|g| g.is_open()).unwrap_or(false))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Snapshot password-protected wallets that are still closed.
    pub(crate) fn locked_wallet_hashes(self: &Arc<Self>) -> Vec<WalletSeedHash> {
        let wallets = match self.wallets.read() {
            Ok(wallets) => wallets,
            Err(poisoned) => poisoned.into_inner(),
        };
        wallets
            .iter()
            .filter_map(|(seed_hash, wallet)| {
                wallet
                    .read()
                    .ok()
                    .filter(|wallet| wallet.uses_password && !wallet.is_open())
                    .map(|_| *seed_hash)
            })
            .collect()
    }

    /// Count wallets that block the cold-start completion sentinel: an OPEN
    /// wallet not yet registered with the upstream wallet backend, OR any
    /// wallet whose lock cannot be read.
    ///
    /// The migration writes its sentinel only when this is zero. Soundness for
    /// the registered set relies on the copy step rejecting exactly what
    /// hydration drops (see
    /// `migration::finish_unwire::hd_seed_row_is_hydratable`), so every wallet
    /// that reached the vault is hydrated and seen here.
    ///
    /// Counted (sentinel withheld):
    /// - a readable, open, not-yet-registered wallet;
    /// - any wallet whose `RwLock` cannot be read — fail-safe, so a poisoned
    ///   lock can never green-light a premature "completed".
    ///
    /// Excluded (handled before this check):
    /// - a readable, `Closed` / locked password-protected wallet — migration's
    ///   awaiting-password state collects and unlocks these first.
    ///
    /// Counts over the raw `self.wallets` map, NOT the [`Self::open_wallets`]
    /// snapshot — that snapshot already drops a poisoned-lock wallet before the
    /// fail-safe could see it. A poisoned OUTER map lock is recovered via
    /// `into_inner` so a prior panic elsewhere cannot zero the count. When the
    /// backend is not yet wired nothing is registered, so every open (or
    /// unreadable) wallet counts.
    pub(crate) fn unregistered_open_wallet_count(self: &Arc<Self>) -> usize {
        let backend = self.wallet_backend().ok();
        let guard = match self.wallets.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .values()
            .filter(|w| match w.read() {
                // Unreadable per-wallet lock: cannot prove it is registered, so
                // fail safe and count it (withholds the sentinel).
                Err(_) => true,
                // Readable Closed / locked-protected: the migration password gate
                // handles it before calling this registration check.
                Ok(g) if !g.is_open() => false,
                // Readable and open: unregistered unless the wired backend knows
                // it. With no backend wired nothing is registered, so it counts.
                Ok(g) => backend
                    .as_ref()
                    .map(|b| b.registered_wallet_id(&g.seed_hash()).is_none())
                    .unwrap_or(true),
            })
            .count()
    }
}
