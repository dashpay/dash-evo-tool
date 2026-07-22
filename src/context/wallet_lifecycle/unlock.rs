//! Lock / unlock handling: reacting to a wallet becoming unlocked or locked
//! and snapshotting the set of open wallets.

use super::*;

use crate::wallet_backend::SecretLease;

impl AppContext {
    /// Verify and open a password-protected wallet through the secret chokepoint.
    ///
    /// Since the JIT migration this is **not** a seed-distribution point —
    /// signing pulls the seed just-in-time from the encrypted vault through
    /// the [`SecretAccess`](crate::wallet_backend::SecretAccess) chokepoint.
    /// It verifies the supplied password against whichever at-rest scheme the
    /// vault reports, marks the secret-free wallet model open only after that
    /// succeeds, then re-drives the JIT bootstrap so the wallet is
    /// upstream-registered this session. Unlocks with a retention shorter than
    /// the session hold the cache entry under a ref-counted
    /// [`SecretLease`](crate::wallet_backend::SecretLease) and forget it once
    /// every consumer of that unlock is done with the seed.
    ///
    /// `passphrase` is verified here, not in the model's legacy-envelope-only
    /// reader. `retention` controls whether the temporary seed remains
    /// available afterwards.
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
    ) -> Result<(), TaskError> {
        let (seed_hash, uses_password) = {
            let guard = wallet.read_recover();
            (guard.seed_hash(), guard.uses_password)
        };

        // No-password wallets need no promotion — they resolve prompt-free
        // through the chokepoint's unprotected fast-path.
        if !uses_password {
            return Ok(());
        }

        let backend = self.wallet_backend()?;
        let secret = platform_wallet_storage::secrets::SecretString::new(passphrase);
        if let Err(error) = backend.secret_access().promote_hd_seed_with_passphrase(
            &seed_hash,
            Some(&secret),
            crate::wallet_backend::RememberPolicy::UntilAppClose,
        ) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?error,
                "Unlocked wallet seed could not be saved in the current vault"
            );
            wallet.write_recover().wallet_seed.close();
            return Err(error);
        }
        wallet
            .write_recover()
            .wallet_seed
            .mark_open_after_verification();
        tracing::trace!(
            wallet = %hex::encode(seed_hash),
            "Verified-open seed promoted to the session cache on unlock"
        );
        // A retention shorter than the session is enforced by a ref-counted
        // lease, not by a single owner: the seed is forgotten once every
        // consumer has dropped its clone. The storage update is a second,
        // unsynchronised consumer of the very seed its own prompt unlocked
        // (`register_migrated_wallets` re-enters the scope through
        // `bootstrap_loaded_wallets`), so it takes a clone of the same lease and
        // releases it when the update finishes.
        let lease = match retention {
            WalletUnlockRetention::UntilAppClose => None,
            WalletUnlockRetention::OperationOnly
            | WalletUnlockRetention::UntilStorageUpdateComplete => Some(
                backend
                    .secret_access()
                    .lease(crate::wallet_backend::SecretScope::HdSeed { seed_hash }),
            ),
        };
        if let Some(lease) = &lease
            && retention == WalletUnlockRetention::UntilStorageUpdateComplete
        {
            self.migration_status().hold_seed_lease(lease.clone());
        }

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
        // session cache. The in-memory wallet is flipped `Open` only after the
        // chokepoint succeeds above, so the JIT `is_open()` gate passes.
        self.drive_unlock_registration(wallet, lease);
        Ok(())
    }

    /// Spawn the unlock-triggered JIT bootstrap/registration for a wallet whose
    /// seed was just promoted to the session cache by [`Self::handle_wallet_unlocked`].
    ///
    /// `handle_wallet_unlocked` is synchronous (called from the UI thread) while
    /// [`Self::bootstrap_wallet_addresses_jit`] is async, so the reconciliation
    /// runs on a tracked subtask — mirroring [`Self::register_wallet_upstream`].
    /// Best-effort: the JIT bootstrap logs and swallows its own failures, and a
    /// missing-backend cold-boot path is covered by `bootstrap_loaded_wallets`.
    ///
    /// `lease` keeps the promoted seed resolvable for this subtask's own work.
    /// It is only *a* holder of that lease, never the sole one: dropping it here
    /// forgets the seed only if no other consumer still holds a clone.
    fn drive_unlock_registration(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        lease: Option<SecretLease>,
    ) {
        let ctx = Arc::clone(self);
        let wallet = Arc::clone(wallet);
        let _ = self
            .subtasks
            .spawn_sync("wallet_unlock_registration", async move {
                let lease = lease;
                ctx.bootstrap_wallet_addresses_jit(&wallet).await;
                ctx.discover_unlocked_wallet_identities(&wallet).await;
                drop(lease);
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
            .read_recover()
            .values()
            .filter(|wallet| wallet.read_recover().is_open())
            .cloned()
            .collect()
    }

    /// Snapshot password-protected wallets that are still closed.
    pub(crate) fn locked_wallet_hashes(self: &Arc<Self>) -> Vec<WalletSeedHash> {
        let wallets = self.wallets.read_recover();
        wallets
            .iter()
            .filter_map(|(seed_hash, wallet)| {
                wallet
                    .read_recover()
                    .requires_password_unlock()
                    .then_some(*seed_hash)
            })
            .collect()
    }

    /// Count open wallets that block the cold-start completion sentinel because
    /// they are not yet registered with the upstream wallet backend.
    ///
    /// The migration writes its sentinel only when this is zero. Soundness for
    /// the registered set relies on the copy step rejecting exactly what
    /// hydration drops (see
    /// `migration::finish_unwire::hd_seed_row_is_hydratable`), so every wallet
    /// that reached the vault is hydrated and seen here.
    ///
    /// Poisoned outer and per-wallet locks are recovered consistently with
    /// [`Self::open_wallets`] and [`Self::locked_wallet_hashes`], so a prior
    /// panic never makes a wallet disappear from this decision.
    pub(crate) fn unregistered_open_wallet_count(self: &Arc<Self>) -> usize {
        let backend = self.wallet_backend().ok();
        let guard = self.wallets.read_recover();
        guard
            .values()
            .filter(|wallet| {
                let wallet = wallet.read_recover();
                if !wallet.is_open() {
                    false
                } else {
                    backend
                        .as_ref()
                        .map(|backend| backend.registered_wallet_id(&wallet.seed_hash()).is_none())
                        .unwrap_or(true)
                }
            })
            .count()
    }
}
