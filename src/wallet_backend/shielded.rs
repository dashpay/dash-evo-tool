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
use crate::model::wallet::shielded_max_transfer::ShieldedMaxTransferState;
use platform_wallet::wallet::shielded::WalletDrainPreview;
use std::sync::Arc;

use super::{
    DetPlatformSigner, DetSigner, PlatformPathIndex, SecretPlaintext, WalletBackend,
    map_shielded_op_error,
};

/// Borrow the 64-byte HD seed from a resolved secret session for a shielded
/// spend. Upstream re-derives the Orchard spend keyset from this seed on every
/// `shielded_*_to` call.
///
/// A resolved secret that is not an HD seed indicates inconsistent wallet
/// structure and maps to [`TaskError::WalletStateInconsistent`]. This is
/// distinct from the protected-wallet path: `with_secret_session` prompts for
/// that wallet, and a dismissed or headless prompt surfaces as
/// [`TaskError::SecretPromptCancelled`] / [`TaskError::SecretPromptUnavailable`].
fn hd_seed_for_shielded_spend<'a>(
    plaintext: &'a SecretPlaintext<'_>,
) -> Result<&'a [u8; 64], TaskError> {
    plaintext
        .expose_hd_seed()
        .ok_or(TaskError::WalletStateInconsistent)
}

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
                source: Arc::new(e),
            })
    }

    #[cfg(test)]
    pub(crate) async fn bind_shielded_from_persisted_for_test(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<bool, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let coordinator = self.shielded_coordinator_arc().await?;
        wallet
            .bind_shielded_from_persisted(&[0], &coordinator)
            .await
            .map_err(|error| TaskError::WalletBackend {
                source: Arc::new(error),
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
        if !matches!(
            funding,
            platform_wallet::wallet::asset_lock::AssetLockFunding::FromExistingAssetLock { .. }
        ) {
            self.validate_core_spend_history(seed_hash).await?;
        }
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
    /// The HD seed is resolved just-in-time through the
    /// [`SecretAccess`](super::SecretAccess) chokepoint: upstream re-derives the
    /// Orchard spend keyset (ASK included) from it for this call only and drops
    /// it on return, so the spend authority is never left resident. An
    /// unprotected wallet resolves prompt-free; a protected one is prompted for
    /// its passphrase unless already cached, and a dismissed or headless prompt
    /// surfaces as [`TaskError::SecretPromptCancelled`] /
    /// [`TaskError::SecretPromptUnavailable`]. A non-HD secret at this HD scope
    /// is a structural invariant failure and returns
    /// [`TaskError::WalletStateInconsistent`] — see
    /// `hd_seed_for_shielded_spend`.
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
        // coordinator / wallet / prover need no seed, so resolve them before the
        // secret session to trim its front edge and fail fast (ShieldedNotConfigured
        // / WalletNotLoaded) without prompting for a passphrase.
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = hd_seed_for_shielded_spend(&plaintext)?;
                wallet
                    .shielded_transfer_to(
                        &coordinator,
                        seed,
                        account,
                        recipient_raw_43,
                        amount,
                        memo,
                        &prover,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Unshield from `account`'s notes to a transparent platform address
    /// (bech32m `"dash1…"` / `"tdash1…"` string).
    ///
    /// The HD seed is resolved just-in-time through the
    /// [`SecretAccess`](super::SecretAccess) chokepoint (see
    /// [`shielded_transfer`](Self::shielded_transfer) for the spend-authority
    /// lifetime and the prompt / failure-mode behaviour).
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
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = hd_seed_for_shielded_spend(&plaintext)?;
                wallet
                    .shielded_unshield_to(
                        &coordinator,
                        seed,
                        account,
                        to_platform_addr_bech32m,
                        amount,
                        &prover,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Withdraw from `account`'s notes to a Core L1 address (Base58Check).
    ///
    /// The HD seed is resolved just-in-time through the
    /// [`SecretAccess`](super::SecretAccess) chokepoint (see
    /// [`shielded_transfer`](Self::shielded_transfer) for the spend-authority
    /// lifetime and the prompt / failure-mode behaviour).
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
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = hd_seed_for_shielded_spend(&plaintext)?;
                wallet
                    .shielded_withdraw_to(
                        &coordinator,
                        seed,
                        account,
                        to_core_address,
                        amount,
                        core_fee_per_byte,
                        &prover,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Read-only quote for moving the whole Core balance to the shielded
    /// balance. Never reserves, signs or broadcasts anything.
    pub(crate) async fn shielded_max_transfer_preview(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<WalletDrainPreview, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        wallet
            .shielded_wallet_drain_preview(MAX_TRANSFER_CORE_ACCOUNT, 0)
            .await
            .map_err(map_shielded_op_error)
    }

    /// Where the Max transfer `operation_id` stands. Reads the durable and
    /// live wallet state only; never touches the network.
    pub(crate) async fn shielded_max_transfer_state(
        &self,
        seed_hash: &WalletSeedHash,
        operation_id: [u8; 16],
    ) -> Result<ShieldedMaxTransferState, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let snapshot = wallet
            .shielded_wallet_drain_operation_status(operation_id)
            .await
            .map_err(map_shielded_op_error)?;
        Ok(max_transfer_state(snapshot.state))
    }

    /// Make the Max transfer `operation_id` visible to status readers as a
    /// running drain, until the returned handle is dropped.
    ///
    /// Taken before the transfer's id is stored and held until its outcome is
    /// read: in between, a status read sees the drain as pending rather than
    /// "not sent", and so never releases the id of a drain about to run.
    pub(crate) async fn register_max_transfer(
        &self,
        seed_hash: &WalletSeedHash,
        operation_id: [u8; 16],
    ) -> Result<MaxTransferOperation, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let operation = Arc::new(
            platform_wallet::wallet::asset_lock::AssetLockOperationContext::new(operation_id),
        );
        let registration = wallet.register_wallet_drain_operation(Arc::clone(&operation));
        Ok(MaxTransferOperation {
            wallet,
            operation,
            registration,
        })
    }

    /// Move the whole Core balance, minus fees, to this wallet's own shielded
    /// address as the registered upstream wallet drain `transfer`.
    ///
    /// Running it again with the same id finishes that transfer: upstream
    /// resumes the drain's own funding lock and never builds a second one.
    /// Upstream also owns the lock floor: a drain too small to pay the pool
    /// fee is abandoned before anything is broadcast.
    ///
    /// The outcome comes with the state the drain was left in. A state of
    /// `None` means it is unknown: the drain either never reached upstream or
    /// returned before building a lock, and only a later status read can tell.
    pub(crate) async fn shield_max_from_core(
        &self,
        seed_hash: &WalletSeedHash,
        transfer: MaxTransferOperation,
    ) -> MaxTransferAttempt {
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        let MaxTransferOperation {
            wallet,
            operation,
            registration,
        } = transfer;
        let prepared = async {
            let coordinator = self.shielded_coordinator_arc().await?;
            let raw = wallet
                .shielded_default_address(0)
                .await
                .ok_or(TaskError::ShieldedNotBound)?;
            let recipient = dash_sdk::dpp::address_funds::OrchardAddress::from_raw_bytes(&raw)
                .map_err(|_| TaskError::ShieldedInvalidRecipientAddress)?;
            Ok::<_, TaskError>((coordinator, recipient))
        }
        .await;
        let result = match prepared {
            Err(error) => Err(error),
            Ok((coordinator, recipient)) => {
                let scope = Self::hd_scope(seed_hash);
                self.inner
                    .secret_access
                    .with_secret_session(&scope, async |session| {
                        let asset_lock_signer =
                            DetSigner::from_held(session.plaintext(), self.inner.network);
                        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
                        wallet
                            .shielded_fund_from_asset_lock(
                                &coordinator,
                                AssetLockFunding::DrainWalletBalance {
                                    account_index: MAX_TRANSFER_CORE_ACCOUNT,
                                    // Upstream sets the floor from the pool fee.
                                    minimum_lock_duffs: None,
                                    operation: Arc::clone(&operation),
                                },
                                vec![(recipient, None)],
                                &asset_lock_signer,
                                &prover,
                                None,
                                0,
                                None,
                                // User-facing funding: wait indefinitely for the
                                // ChainLock (the broadcast lock is pending, never failed).
                                None,
                            )
                            .await
                            .map_err(map_shielded_op_error)
                    })
                    .await
            }
        };
        // Dropped before the outcome is read, so a pre-build return no longer
        // masks the durable answer of a follow-up status read.
        drop(registration);

        let snapshot = operation.snapshot();
        let state =
            drain_attempt_state(result.is_ok(), snapshot.state, snapshot.out_point.is_some());
        MaxTransferAttempt { state, result }
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
                source: Arc::new(e),
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

/// The Core account a Max transfer is built for. Upstream pools it with the
/// BIP32 account and every DashPay contact-receiving account
/// (`ASSET_LOCK_FUNDING_SOURCES`); CoinJoin funds stay out.
const MAX_TRANSFER_CORE_ACCOUNT: u32 = 0;

/// A Max transfer's drain, registered with its wallet by
/// [`WalletBackend::register_max_transfer`]. Dropping it ends the
/// registration.
pub(crate) struct MaxTransferOperation {
    wallet: Arc<platform_wallet::PlatformWallet>,
    operation: Arc<platform_wallet::wallet::asset_lock::AssetLockOperationContext>,
    registration: platform_wallet::wallet::asset_lock::AssetLockOperationRegistration,
}

/// The state a drain run left behind. Pending is the operation's initial
/// state, so Pending without a lock says nothing about whether one was built:
/// that is `None`, for a status read to settle.
fn drain_attempt_state(
    succeeded: bool,
    state: platform_wallet::wallet::asset_lock::AssetLockOperationState,
    has_lock: bool,
) -> Option<ShieldedMaxTransferState> {
    use platform_wallet::wallet::asset_lock::AssetLockOperationState as Upstream;
    match (succeeded, state, has_lock) {
        (true, _, _) => Some(ShieldedMaxTransferState::Finished),
        (false, Upstream::Pending, false) => None,
        (false, state, _) => Some(max_transfer_state(state)),
    }
}

/// What [`WalletBackend::shield_max_from_core`] left behind.
#[derive(Debug)]
pub(crate) struct MaxTransferAttempt {
    /// The transfer's state, or `None` when only a status read can tell.
    pub(crate) state: Option<ShieldedMaxTransferState>,
    pub(crate) result: Result<(), TaskError>,
}

/// DET's view of an upstream drain operation state.
fn max_transfer_state(
    state: platform_wallet::wallet::asset_lock::AssetLockOperationState,
) -> ShieldedMaxTransferState {
    use platform_wallet::wallet::asset_lock::AssetLockOperationState as Upstream;
    match state {
        Upstream::NotSent => ShieldedMaxTransferState::NotSent,
        Upstream::Pending => ShieldedMaxTransferState::Pending,
        Upstream::Completed => ShieldedMaxTransferState::Finished,
        Upstream::Conflicted => ShieldedMaxTransferState::Conflicted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    #[test]
    fn a_drain_run_reports_what_it_knows_and_defers_the_rest() {
        use platform_wallet::wallet::asset_lock::AssetLockOperationState as Upstream;
        for state in [
            Upstream::NotSent,
            Upstream::Pending,
            Upstream::Completed,
            Upstream::Conflicted,
        ] {
            assert_eq!(
                drain_attempt_state(true, state, false),
                Some(ShieldedMaxTransferState::Finished)
            );
        }
        // A failed run that never built a lock: only a status read can tell.
        assert_eq!(drain_attempt_state(false, Upstream::Pending, false), None);
        assert_eq!(
            drain_attempt_state(false, Upstream::Pending, true),
            Some(ShieldedMaxTransferState::Pending)
        );
        assert_eq!(
            drain_attempt_state(false, Upstream::NotSent, false),
            Some(ShieldedMaxTransferState::NotSent)
        );
        assert_eq!(
            drain_attempt_state(false, Upstream::Conflicted, true),
            Some(ShieldedMaxTransferState::Conflicted)
        );
    }

    #[test]
    fn max_transfer_state_maps_every_upstream_state() {
        use platform_wallet::wallet::asset_lock::AssetLockOperationState as Upstream;
        assert_eq!(
            max_transfer_state(Upstream::NotSent),
            ShieldedMaxTransferState::NotSent
        );
        assert_eq!(
            max_transfer_state(Upstream::Pending),
            ShieldedMaxTransferState::Pending
        );
        assert_eq!(
            max_transfer_state(Upstream::Completed),
            ShieldedMaxTransferState::Finished
        );
        assert_eq!(
            max_transfer_state(Upstream::Conflicted),
            ShieldedMaxTransferState::Conflicted
        );
    }

    /// An HD-seed secret resolves to its 64 raw bytes — the common shielded-spend
    /// path.
    #[test]
    fn hd_seed_for_shielded_spend_accepts_hd_seed() {
        let seed = Zeroizing::new([0x11u8; 64]);
        let plaintext = SecretPlaintext::HdSeed(&seed);
        let got = hd_seed_for_shielded_spend(&plaintext).expect("an HD seed must resolve");
        assert_eq!(got, &[0x11u8; 64]);
    }

    /// A non-HD-seed secret at an HD scope is a structural wallet inconsistency,
    /// distinct from the protected-wallet prompt path.
    #[test]
    fn hd_seed_for_shielded_spend_rejects_non_hd_secret_as_wallet_state_inconsistent() {
        let key = Zeroizing::new([0x22u8; 32]);
        let plaintext = SecretPlaintext::SingleKey(&key);
        let err = hd_seed_for_shielded_spend(&plaintext)
            .expect_err("a non-HD-seed secret must not yield a shielded spend key");
        assert!(
            matches!(err, TaskError::WalletStateInconsistent),
            "got {err:?}"
        );
    }
}
