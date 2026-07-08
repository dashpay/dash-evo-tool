//! Shielded-pool (Orchard) operations on [`WalletBackend`].
//!
//! `platform-wallet` is always built with the `shielded` Cargo feature in
//! DET (added in Phase A).  No DET-level opt-out exists, so these methods
//! are unconditionally available.  The fund-moving ops and reads consumed by
//! `run_shielded_task` / `ensure_shielded_bound` are wired (Phase D); the
//! activity/notes list reads remain `#[allow(dead_code)]` until the Phase-F
//! upstream-coordinator read path lands.

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;

use super::{
    DetPlatformSigner, DetSigner, PlatformPathIndex, WalletBackend, map_shielded_op_error,
};

impl WalletBackend {
    /// Resolve the network-scoped shielded coordinator.
    ///
    /// Returns `ShieldedNotConfigured` when `configure_shielded` was not called
    /// during backend construction (should never happen in practice — it is
    /// called unconditionally in `WalletBackend::new`).
    async fn shielded_coordinator_arc(
        &self,
    ) -> Result<
        std::sync::Arc<platform_wallet::wallet::shielded::NetworkShieldedCoordinator>,
        TaskError,
    > {
        self.inner
            .pwm
            .shielded_coordinator()
            .await
            .ok_or(TaskError::ShieldedNotConfigured)
    }

    /// Idempotently bind Orchard ZIP-32 keys for `seed_hash` to the shielded
    /// coordinator.  A no-op when the wallet is already bound; one call needed
    /// per wallet per process lifetime.
    ///
    /// Called from `bootstrap_wallet_addresses_jit` (Phase C-bind) inside the
    /// existing `with_secret_session` scope; also callable directly for the
    /// MCP headless path.
    pub(crate) async fn ensure_shielded_bound(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8],
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        if wallet.is_shielded_bound().await {
            return Ok(());
        }
        let coordinator = self.shielded_coordinator_arc().await?;
        wallet
            .bind_shielded(seed, &[0], &coordinator)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
    }

    /// Bind Orchard keys for `seed_hash` by resolving its HD seed just-in-time
    /// through the [`SecretAccess`](super::SecretAccess) chokepoint, then delegating to
    /// [`Self::ensure_shielded_bound`].
    ///
    /// The headless sibling of the Phase-C-bind call in
    /// `bootstrap_wallet_addresses_jit`: an unprotected wallet resolves
    /// prompt-free via the no-passphrase fast-path; a protected one needs its
    /// seed already promoted to the session cache. Idempotent — exits
    /// immediately when the wallet is already bound. The Phase-G `shielded_init`
    /// MCP tool uses this so an agent can bind a wallet without a GUI prompt.
    #[cfg(any(feature = "mcp", feature = "cli"))]
    pub(crate) async fn ensure_shielded_bound_jit(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                self.ensure_shielded_bound(seed_hash, seed).await
            })
            .await
    }

    /// Warm the Orchard proving key so the first shielded spend does not block.
    ///
    /// The Halo2 proving key takes ~30 s to build on first use; this primes the
    /// process-global cache ahead of time. Runs on a blocking thread so the
    /// async runtime keeps serving other work during the build, and is
    /// idempotent — a second call returns immediately. Returns whether the key
    /// is ready afterwards (a `spawn_blocking` panic leaves it unbuilt → `false`).
    /// The Phase-G `shielded_init` MCP tool calls this during headless setup.
    #[cfg(any(feature = "mcp", feature = "cli"))]
    pub(crate) async fn warm_shielded_prover(&self) -> bool {
        tokio::task::spawn_blocking(|| {
            let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
            prover.warm_up();
            prover.is_ready()
        })
        .await
        .unwrap_or(false)
    }

    /// Fund the shielded pool from a Core asset lock through the upstream
    /// orchestrator pipeline.
    ///
    /// Mirrors `fund_platform_address` exactly: the `asset_lock_signer` is a
    /// `DetSigner` borrowed from the held JIT session, and the upstream method
    /// owns the full IS→CL fallback + `consume_asset_lock` path.  A single
    /// `(recipient, None)` entry passes the whole lock value (minus the flat
    /// shielded fee) to `recipient`.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn shield_from_asset_lock(
        &self,
        seed_hash: &WalletSeedHash,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        recipient: dash_sdk::dpp::address_funds::OrchardAddress,
        dummy_outputs: usize,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
                wallet
                    .shielded_fund_from_asset_lock(
                        &coordinator,
                        funding,
                        vec![(recipient, None)],
                        &asset_lock_signer,
                        &prover,
                        None,
                        dummy_outputs,
                        settings,
                        // User-facing funding: wait indefinitely for the
                        // ChainLock (the broadcast lock is pending, never failed).
                        None,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Shield platform-address credits (Type 15) into the Orchard pool.
    ///
    /// The `signer` authorises the per-address `AddressWitness`; it is a
    /// `DetPlatformSigner` built from the held JIT seed and `path_index`.
    /// Build `path_index` via `PlatformPathIndex::from_wallet` before calling —
    /// the same pattern as `fund_platform_address`.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shield_from_balance(
        &self,
        seed_hash: &WalletSeedHash,
        path_index: &PlatformPathIndex,
        shielded_account: u32,
        payment_account: u32,
        amount: u64,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let signer = DetPlatformSigner::from_held(seed, self.inner.network, path_index);
                let wallet = self.resolve_wallet(seed_hash).await?;
                let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
                wallet
                    .shielded_shield_from_account(
                        &coordinator,
                        shielded_account,
                        payment_account,
                        amount,
                        &signer,
                        &prover,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Shielded → shielded transfer from `account`'s notes to `recipient`.
    ///
    /// No seed scope needed — the Orchard ASK is already resident in the
    /// wallet's bound `shielded_keys` slot from `ensure_shielded_bound`.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shielded_transfer(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        recipient_raw_43: &[u8; 43],
        amount: u64,
        memo: [u8; 36],
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        wallet
            .shielded_transfer_to(
                &coordinator,
                account,
                recipient_raw_43,
                amount,
                memo,
                &prover,
            )
            .await
            .map_err(map_shielded_op_error)
    }

    /// Unshield from `account`'s notes to a transparent platform address
    /// (bech32m `"dash1…"` / `"tdash1…"` string).
    ///
    /// No seed scope needed — keys are already bound.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shielded_unshield(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        to_platform_addr_bech32m: &str,
        amount: u64,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        wallet
            .shielded_unshield_to(
                &coordinator,
                account,
                to_platform_addr_bech32m,
                amount,
                &prover,
            )
            .await
            .map_err(map_shielded_op_error)
    }

    /// Withdraw from `account`'s notes to a Core L1 address (Base58Check).
    ///
    /// No seed scope needed — keys are already bound.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shielded_withdraw(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        to_core_address: &str,
        amount: u64,
        core_fee_per_byte: u32,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        wallet
            .shielded_withdraw_to(
                &coordinator,
                account,
                to_core_address,
                amount,
                core_fee_per_byte,
                &prover,
            )
            .await
            .map_err(map_shielded_op_error)
    }

    /// Per-account unspent shielded balance for `seed_hash`'s wallet.
    ///
    /// Returns an empty map when the wallet is not bound or has no shielded
    /// balance.  This is the push-snapshot producer (Phase E): the result is
    /// written into `AppContext::shielded_balances` by `on_shielded_sync_completed`.
    pub(crate) async fn shielded_balances(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<std::collections::BTreeMap<u32, u64>, TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        wallet
            .shielded_balances(&coordinator)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
    }

    /// The default Orchard payment address for `account` on `seed_hash`'s wallet
    /// (raw 43-byte representation).  Returns `None` if the wallet is not bound
    /// or `account` is not registered.
    pub async fn shielded_default_address(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
    ) -> Result<Option<[u8; 43]>, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        Ok(wallet.shielded_default_address(account).await)
    }

    /// A page of shielded activity for `account` on `seed_hash`'s wallet,
    /// sorted for display (pending first, then descending block height).
    ///
    /// `offset` / `limit` mirror the coordinator store's pagination contract.
    #[allow(dead_code)]
    pub(crate) async fn shielded_activity(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<platform_wallet::wallet::shielded::ShieldedActivityEntry>, TaskError> {
        use platform_wallet::wallet::shielded::{ShieldedStore, SubwalletId};

        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let subwallet = SubwalletId::new(wallet_id, account);

        coordinator
            .store()
            .read()
            .await
            .get_activity(subwallet, offset, limit)
            .map_err(|source| TaskError::ShieldedStoreReadFailed { source })
    }

    /// Unspent shielded notes for `account` on `seed_hash`'s wallet.
    ///
    /// Note: for spendability checks, prefer `shielded_balances`; this method
    /// exposes the raw note list for diagnostic and display purposes.
    #[allow(dead_code)]
    pub(crate) async fn shielded_notes(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
    ) -> Result<Vec<platform_wallet::wallet::shielded::ShieldedNote>, TaskError> {
        use platform_wallet::wallet::shielded::{ShieldedStore, SubwalletId};

        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let subwallet = SubwalletId::new(wallet_id, account);

        coordinator
            .store()
            .read()
            .await
            .get_unspent_notes(subwallet)
            .map_err(|source| TaskError::ShieldedStoreReadFailed { source })
    }

    /// Force an immediate shielded sync pass (network-wide across every bound
    /// wallet on the coordinator).
    ///
    /// `sync_now` fires `on_shielded_sync_completed` synchronously before it
    /// returns, so the [`EventBridge`](super::EventBridge) has already written the post-sync
    /// per-wallet balances into `AppContext::shielded_balances` (Phase E) by the
    /// time this resolves — a subsequent `shielded_balance_credits` read sees
    /// the fresh figure. The 60-second background loop is the normal driver;
    /// this is the explicit-refresh primitive for the backend-e2e lifecycle test
    /// and the Phase-G `shielded_sync` MCP tool. A no-op when shielded support
    /// was never configured (empty coordinator → empty pass).
    pub async fn sync_shielded_now(&self, force: bool) {
        self.inner.pwm.shielded_sync_arc().sync_now(force).await;
    }
}
