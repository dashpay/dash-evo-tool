//! Identity registration / top-up and platform-address funding on
//! [`WalletBackend`].
//!
//! These methods own the seed-bearing identity lifecycle: they open a
//! just-in-time [`SecretAccess`](super::SecretAccess) session, provision the
//! identity-funding accounts the watch-only live wallet lacks, and drive the
//! upstream orchestrated register / top-up / asset-lock funding pipelines.
//! Upstream errors are classified into typed `TaskError` variants by the
//! `map_identity_*` / `map_platform_address_fund_error` helpers.

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet::wallet::platform_wallet::WalletId;
use std::sync::Arc;

use super::{
    DetPlatformSigner, DetSigner, PlatformPathIndex, WalletBackend, map_identity_register_error,
    map_identity_top_up_error, map_platform_address_fund_error,
};

/// The all-zero `WalletId` upstream storage reserves as the spelling of
/// "owned by no wallet": every scope-aware reader and writer maps it to a
/// NULL `wallet_id`. Never a real wallet id.
const UNOWNED_WALLET_SCOPE: WalletId = [0u8; 32];

/// The two identity-funding account flavours DET provisions. Parsing the
/// caller's intent into this once (at [`WalletBackend::ensure_identity_funding_accounts`])
/// removes the repeated `AccountType` matches — and their `unreachable!` arms —
/// inside [`WalletBackend::provision_identity_funding_account`], and makes the
/// unsupported-account-type case unrepresentable.
#[derive(Clone, Copy)]
enum Funding {
    /// The identity-registration funding account.
    Registration,
    /// The per-identity top-up funding account at the given registration index.
    TopUp(u32),
    /// The single top-up funding account bound to no identity index — what a
    /// top-up of an identity this wallet does not own draws its credit output
    /// from, since no index in this wallet's tree describes that identity.
    TopUpNotBound,
}

impl WalletBackend {
    /// Register a new identity on Platform funded by an asset lock built and
    /// tracked-to-finality by the upstream `AssetLockManager`. Returns the
    /// persisted [`Identity`](dash_sdk::platform::Identity).
    ///
    /// Wraps upstream `IdentityWallet::register_identity_with_funding` —
    /// upstream handles asset-lock build/broadcast, IS→CL fallback with the
    /// CL-height-too-low retry, the actual `PutIdentity` submission, and the
    /// asset-lock cleanup. The DET retry loop around `UnknownVersionError`
    /// and the manual IS-proof-invalid fallback are no longer needed at the
    /// caller — upstream owns both paths.
    ///
    /// `funding` is the upstream funding selector — `FromWalletBalance`
    /// builds a fresh asset lock, `FromExistingAssetLock` resumes from a
    /// tracked outpoint (the wallet-backend tracker is the single source of
    /// asset-lock state).
    pub async fn register_identity(
        &self,
        seed_hash: &WalletSeedHash,
        identity_index: u32,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        keys_map: std::collections::BTreeMap<u32, dash_sdk::dpp::identity::IdentityPublicKey>,
        identity_signer: &crate::model::qualified_identity::QualifiedIdentity,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<dash_sdk::platform::Identity, TaskError> {
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                // Re-provisioning idempotent. Run inside the session so the
                // seed is available for hardened xpub derivation (the live
                // wallet is watch-only).
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::WalletStateInconsistent)?;
                self.ensure_identity_funding_accounts(seed_hash, seed, identity_index)
                    .await?;
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                wallet
                    .identity()
                    .register_identity_with_funding(
                        funding,
                        identity_index,
                        keys_map,
                        identity_signer,
                        &asset_lock_signer,
                        settings,
                    )
                    .await
                    .map_err(map_identity_register_error)
            })
            .await
    }

    /// Ensure `identity` (wallet-owned at HD `identity_index`) is registered in
    /// the upstream `IdentityManager` for `seed_hash`, so identity ops that look
    /// the identity up there (currently: top-up) can find it.
    ///
    /// **Precondition: `seed_hash` owns `identity`** — its DET wallet link names
    /// this wallet. Registering another wallet's identity here files that
    /// identity's keys under this wallet, which its next load cannot resolve,
    /// and displaces whatever this wallet already holds at `identity_index`.
    ///
    /// Idempotent: a no-op once the identity is managed, and a concurrent
    /// `IdentityAlreadyExists` is treated as success. Touches only public-key
    /// data — never the seed — so it is safe to call while the wallet is LOCKED.
    ///
    /// Returns `true` when this call newly registered the identity, `false` when
    /// it was already managed — so the reconcile driver logs only real changes.
    ///
    /// # Errors
    /// [`TaskError::WalletNotLoaded`] if the wallet is not yet upstream
    /// registered; [`TaskError::WalletStateInconsistent`] if the resolved wallet
    /// has no manager entry; [`TaskError::WalletBackend`] on an upstream add
    /// failure other than the swallowed `IdentityAlreadyExists`.
    pub(crate) async fn ensure_identity_managed(
        &self,
        seed_hash: &WalletSeedHash,
        identity: &dash_sdk::platform::Identity,
        identity_index: u32,
    ) -> Result<bool, TaskError> {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let id = identity.id();
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();

        // Read-lock fast path: once the identity is managed (steady state, and
        // after the first reconcile persists it), skip the write lock entirely.
        {
            let wm = wallet.wallet_manager().read().await;
            if wm
                .get_wallet_info(&wallet_id)
                .is_some_and(|info| info.identity_manager.identity(&id).is_some())
            {
                return Ok(false);
            }
        }

        let persister = wallet.persister().clone();
        let mut wm = wallet.wallet_manager().write().await;
        let info = wm
            .get_wallet_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        // Re-check under the write lock — another task may have raced us in.
        if info.identity_manager.identity(&id).is_some() {
            return Ok(false);
        }
        match info.identity_manager.add_identity(
            identity.clone(),
            identity_index,
            wallet_id,
            &persister,
        ) {
            Ok(()) => Ok(true),
            Err(platform_wallet::error::PlatformWalletError::IdentityAlreadyExists(_)) => Ok(false),
            Err(e) => Err(TaskError::WalletBackend {
                source: Arc::new(e),
            }),
        }
    }

    /// The ids of every identity in the upstream store that belongs to no
    /// wallet — DET's wallet-less identities. Masternode/evonode nodes are
    /// the expected case, but any identity
    /// [`AppContext::insert_local_qualified_identity`](crate::context::AppContext::insert_local_qualified_identity)
    /// stores without a wallet association lands here too.
    ///
    /// `AppContext::reconcile_unowned_identities` diffs this set against its
    /// own cheap sidecar id scan on both sides — registering what's missing
    /// here and withdrawing what's missing there — so this is also the read
    /// half of that reconcile's two-way contract, not only a test accessor.
    ///
    /// Ids are all the caller gets, but not all this costs: upstream exposes
    /// no id-only door (`SqlitePersister::load_unowned_identities` is the only
    /// public read of the unowned scope), so every row is decoded into a full
    /// `ManagedIdentity` — keys and contacts merged in — and then dropped.
    /// Acceptable because the unowned scope holds one row per wallet-less
    /// identity, i.e. this device's masternode/evonode nodes.
    pub(crate) fn unowned_identity_ids(
        &self,
    ) -> Result<std::collections::BTreeSet<dash_sdk::platform::Identifier>, TaskError> {
        Ok(self.load_unowned_identities()?.into_keys().collect())
    }

    /// Register `identity` in the upstream store as **unowned** — belonging to
    /// no wallet — so a wallet-less identity DET knows about (masternode and
    /// evonode nodes are the expected case, though any wallet-less identity
    /// takes this path) is visible to upstream instead of living only in
    /// DET's k/v sidecar. Idempotent: a no-op once the id is present in the
    /// unowned scope, whether this call or an earlier boot's reconcile put it
    /// there.
    ///
    /// The wallet scope is deliberately *no wallet*: the row is written through
    /// a persister scoped to the all-zero sentinel, which stores a NULL
    /// `identities.wallet_id`. That is what keeps the node's DET record safe —
    /// a real wallet's scope would put the row under that wallet's `ON DELETE
    /// CASCADE`, and the storage layer's `cascade_meta_on_identity_delete`
    /// trigger would then delete the node's `meta_identity` rows, i.e. DET's own
    /// `det:identity:v1` record, when that unrelated wallet is removed.
    ///
    /// **What the mirrored row does *not* carry: `identity`'s public keys.**
    /// Upstream's out-of-wallet write path persists only an identity/balance/
    /// revision changeset — it never emits a keys changeset — so a
    /// `ManagedIdentity` read back for one of these ids has a permanently
    /// *empty* key map and a `status` frozen at `Unknown`, regardless of what
    /// DET's own sidecar record holds. Tracked upstream:
    /// <https://github.com/dashpay/platform/issues/4443>. The row is also
    /// write-once: a no-op on an already-registered id means its
    /// balance/revision stay frozen at whatever they were on first
    /// registration — there is no update path today.
    ///
    /// Currently latent, not a live bug: every production reader —
    /// `AppContext::reconcile_unowned_identities` via [`Self::unowned_identity_ids`],
    /// and this method's own `bool` return — only ever consults an id, never a
    /// mirrored row's keys, status, or balance. It becomes a live bug the day
    /// a caller trusts the mirror for anything beyond presence.
    ///
    /// Returns `true` when this call newly registered the identity, `false`
    /// when it was already in the unowned scope — matching
    /// [`Self::ensure_identity_managed`], so a reconcile driver can log only
    /// real changes instead of every attempt. A `true` is a *verified* one:
    /// the row is read back before it is claimed (see Errors).
    ///
    /// # Errors
    /// [`TaskError::UnownedIdentityMirrorMissing`] when the row is absent from
    /// the unowned scope right after being written there. The readback is the
    /// only evidence available: the upsert's
    /// `WHERE identities.wallet_id IS NULL OR identities.wallet_id IS excluded.wallet_id`
    /// skips a row already filed under a wallet and reports no error for the
    /// skip, upstream offers no re-scoping write, and `add_out_of_wallet_identity`
    /// swallows a persist failure into `tracing::error!` before returning
    /// `Ok(())` (`manager/lifecycle.rs` at pin `4784de03`). A refused mirror and
    /// a failed write therefore arrive here identically — the error deliberately
    /// claims neither cause. Paid on a registration only: the already-registered
    /// fast path above returns before it.
    ///
    /// [`TaskError::WalletStorage`] if reading the unowned set fails;
    /// [`TaskError::WalletBackend`] on an upstream add failure — currently
    /// unreachable in practice, since upstream's `add_out_of_wallet_identity`
    /// returns `Ok` unconditionally, but that is an implementation detail of
    /// the pinned rev, not a documented contract.
    pub(crate) fn ensure_identity_unowned(
        &self,
        identity: &dash_sdk::platform::Identity,
    ) -> Result<bool, TaskError> {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let mut manager = self.unowned_identity_manager()?;
        if manager.identity(&identity.id()).is_some() {
            return Ok(false);
        }
        #[cfg(test)]
        let write_swallowed = self
            .inner
            .swallow_next_unowned_write
            .swap(false, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let write_swallowed = false;
        if !write_swallowed {
            manager
                .add_out_of_wallet_identity(identity.clone(), &self.unowned_scope_persister())
                .map_err(|e| TaskError::WalletBackend {
                    source: Arc::new(e),
                })?;
        }
        if !self.load_unowned_identities()?.contains_key(&identity.id()) {
            return Err(TaskError::UnownedIdentityMirrorMissing {
                identity_id: identity.id(),
            });
        }
        Ok(true)
    }

    /// Withdraw an identity from the upstream store's unowned scope, mirroring
    /// its removal from DET. A no-op for an identity that is not registered
    /// there — including every wallet-owned one.
    ///
    /// Upstream records this as a tombstone rather than a row delete, so it
    /// cannot reach the identity's `meta_identity` rows.
    ///
    /// Retried like registration is: the caller
    /// ([`AppContext::delete_local_qualified_identity`](crate::context::AppContext::delete_local_qualified_identity))
    /// only logs a failure, but the boot reconcile
    /// (`AppContext::reconcile_unowned_identities`) works both directions and
    /// re-issues this call for every registration whose sidecar record is
    /// gone. A tombstone lost to a crash or storage error between the sidecar
    /// delete and this call therefore costs one boot, not the record.
    ///
    /// An `Ok` is a *verified* one: the row is read back as gone before the
    /// removal is claimed (see Errors).
    ///
    /// # Errors
    /// [`TaskError::UnownedIdentityMirrorRemains`] when the row is still in
    /// the unowned scope right after being withdrawn from it. Upstream's
    /// `remove_identity` persists the tombstone through `persist_removal`,
    /// which swallows a persister failure into `tracing::error!` and returns
    /// `()`, so the removal reports success either way (`manager/lifecycle.rs`
    /// at pin `4784de03`) — the readback is the only evidence available. Paid
    /// on a real withdrawal only: the not-registered fast path above returns
    /// before it.
    ///
    /// [`TaskError::WalletStorage`] if reading the unowned set fails;
    /// [`TaskError::WalletBackend`] on an upstream removal failure.
    pub(crate) fn remove_unowned_identity(
        &self,
        identity_id: &dash_sdk::platform::Identifier,
    ) -> Result<(), TaskError> {
        let mut manager = self.unowned_identity_manager()?;
        if manager.identity(identity_id).is_none() {
            return Ok(());
        }
        #[cfg(test)]
        let removal_swallowed = self
            .inner
            .swallow_next_unowned_removal
            .swap(false, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let removal_swallowed = false;
        if !removal_swallowed {
            manager
                .remove_identity(identity_id, &self.unowned_scope_persister())
                .map(|_| ())
                .map_err(|e| TaskError::WalletBackend {
                    source: Arc::new(e),
                })?;
        }
        if self.load_unowned_identities()?.contains_key(identity_id) {
            return Err(TaskError::UnownedIdentityMirrorRemains {
                identity_id: *identity_id,
            });
        }
        Ok(())
    }

    fn load_unowned_identities(
        &self,
    ) -> Result<
        std::collections::BTreeMap<
            dash_sdk::platform::Identifier,
            platform_wallet::ManagedIdentity,
        >,
        TaskError,
    > {
        #[cfg(test)]
        if self
            .inner
            .unowned_read_test_failure
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(TaskError::WalletStorageNotReady);
        }

        self.inner
            .wallet_persister
            .load_unowned_identities()
            .map_err(|source| TaskError::WalletStorage { source })
    }

    /// The unowned identities as a manager, so a mutation keeps the bucket and
    /// its private location index consistent. Deliberately its own manager and
    /// never a wallet's: an unowned identity inside a wallet's manager is
    /// claimed by that wallet on its next flush, via the `identities` upsert's
    /// orphan-promotion COALESCE.
    fn unowned_identity_manager(&self) -> Result<platform_wallet::IdentityManager, TaskError> {
        Ok(platform_wallet::changeset::IdentityManagerStartState {
            out_of_wallet_identities: self.load_unowned_identities()?,
            wallet_identities: Default::default(),
        }
        .into())
    }

    fn unowned_scope_persister(&self) -> platform_wallet::WalletPersister {
        platform_wallet::WalletPersister::new(
            UNOWNED_WALLET_SCOPE,
            Arc::clone(&self.inner.wallet_persister) as Arc<dyn PlatformWalletPersistence>,
        )
    }

    /// Test-only: the identity id this wallet's upstream manager resolves for
    /// `identity_id`. Upstream files managed identities by `(wallet, index)` and
    /// looks them up through a side index, so a foreign identity written into an
    /// occupied slot answers this query with the intruder's id — which is how a
    /// displacement becomes observable at all.
    #[cfg(test)]
    pub(crate) async fn resolved_managed_identity_id(
        &self,
        seed_hash: &WalletSeedHash,
        identity_id: &dash_sdk::platform::Identifier,
    ) -> Option<dash_sdk::platform::Identifier> {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let wallet = self.resolve_wallet(seed_hash).await.ok()?;
        let wallet_id = wallet.wallet_id();
        let manager = wallet.wallet_manager().read().await;
        manager
            .get_wallet_info(&wallet_id)?
            .identity_manager
            .identity(identity_id)
            .map(|managed| managed.identity.id())
    }

    /// Top up **this wallet's own** identity's credit balance from its UTXOs.
    /// Returns the post-top-up identity balance (credits).
    ///
    /// Only for an identity whose DET wallet link names `seed_hash`: the
    /// upstream orchestrator resolves the identity through this wallet's
    /// manager, so this method registers it there first (see
    /// [`Self::ensure_identity_managed`] for what that costs a foreign
    /// identity). An identity owned elsewhere is funded through the
    /// index-less asset-lock path instead.
    ///
    /// Wraps upstream `IdentityWallet::top_up_identity_with_funding` —
    /// upstream handles asset-lock build/broadcast, IS→CL fallback, the
    /// `TopUpIdentity` submission, and the asset-lock cleanup. The
    /// caller-side IS-proof-invalid fallback and `UnknownVersionError`
    /// retry are no longer needed.
    ///
    /// `funding` is the upstream funding selector — `FromWalletBalance`
    /// builds a fresh asset lock, `FromExistingAssetLock` resumes from a
    /// tracked outpoint.
    pub async fn top_up_identity(
        &self,
        seed_hash: &WalletSeedHash,
        identity: &dash_sdk::platform::Identity,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        identity_index: u32,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<u64, TaskError> {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let identity_id = identity.id();
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                // Run inside the session so the seed is available for
                // hardened xpub derivation (the live wallet is watch-only).
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::WalletStateInconsistent)?;
                self.ensure_identity_funding_accounts(seed_hash, seed, identity_index)
                    .await?;
                // Op-seam guard: register the identity in the upstream manager
                // so the top-up lookup finds it, covering the unlock →
                // immediate-top-up race the reconcile subtask can lose.
                // Seed-free and idempotent.
                self.ensure_identity_managed(seed_hash, identity, identity_index)
                    .await?;
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                wallet
                    .identity()
                    .top_up_identity_with_funding(
                        &identity_id,
                        funding,
                        &asset_lock_signer,
                        settings,
                    )
                    .await
                    .map_err(|e| map_identity_top_up_error(identity_id, e))
            })
            .await
    }

    /// Whether `address` is already revealed in this wallet's upstream
    /// platform-payment pool (account 0). This is the exact membership query the
    /// orchestrated `fund_from_asset_lock` pre-flight runs, so a `true` here
    /// means the orchestrator will accept the recipient. The pool holds only the
    /// wallet's own revealed platform addresses, so a `true` also implies
    /// ownership. No reveal side effect: an owned-but-unrevealed address reads
    /// `false`, and the caller falls back to the manual path.
    pub(crate) async fn platform_address_in_pool(
        &self,
        seed_hash: &WalletSeedHash,
        address: &dash_sdk::dpp::address_funds::PlatformAddress,
    ) -> Result<bool, TaskError> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;

        let PlatformAddress::P2pkh(hash) = address else {
            return Ok(false);
        };
        let p2pkh = PlatformP2PKHAddress::new(*hash);

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let manager = wallet.wallet_manager().read().await;
        Ok(manager
            .get_wallet_info(&wallet_id)
            .and_then(|info| {
                info.core_wallet
                    .platform_payment_managed_account_at_index(0)
            })
            .is_some_and(|account| account.contains_platform_address(&p2pkh)))
    }

    /// Fund wallet-owned platform addresses from a Core asset lock through the
    /// upstream orchestration pipeline.
    ///
    /// Wraps `PlatformAddressWallet::fund_from_asset_lock` — upstream owns the
    /// full recovery pipeline: asset-lock build/broadcast (for
    /// `FromWalletBalance`), `submit_with_cl_height_retry`, the InstantSend →
    /// ChainLock fallback, and `consume_asset_lock` on acceptance (so the lock
    /// is never left reusable on an ambiguous failure).
    ///
    /// Callers must pass only destination addresses already revealed in this
    /// wallet's upstream platform-payment pool (gate with
    /// [`Self::platform_address_in_pool`]) — the orchestrator's pre-flight
    /// rejects any other recipient. The `address_signer` authorises per-output
    /// witnesses; the `asset_lock_signer` signs the outer state transition
    /// against the lock's credit-output key. Neither signer copies the seed —
    /// both borrow it from the held session.
    ///
    /// `platform_account_index` selects the platform-payment account (DET uses
    /// 0). The `addresses` map must contain exactly one `None`-amount entry (the
    /// remainder recipient); the lock is consumed in full.
    // Mirrors the upstream `fund_from_asset_lock` surface; each argument is a
    // distinct, required input to that orchestrator.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fund_platform_address(
        &self,
        seed_hash: &WalletSeedHash,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        platform_account_index: u32,
        addresses: std::collections::BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            Option<dash_sdk::dpp::balances::credits::Credits>,
        >,
        fee_strategy: dash_sdk::dpp::address_funds::AddressFundsFeeStrategy,
        path_index: &PlatformPathIndex,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<(), TaskError> {
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let address_signer =
                    DetPlatformSigner::from_held(seed, self.inner.network, path_index);
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                wallet
                    .platform()
                    .fund_from_asset_lock(
                        funding,
                        platform_account_index,
                        addresses,
                        fee_strategy,
                        &address_signer,
                        &asset_lock_signer,
                        settings,
                    )
                    .await
                    .map(|_changeset| ())
                    .map_err(map_platform_address_fund_error)
            })
            .await
    }

    // UPSTREAM GAP: rs-platform-wallet has no identity-funding-account
    // registrar (sibling to register_contact_account). Contained exception —
    // key_wallet plumbing lives ONLY here, never leaks past WalletBackend.
    // Do not replicate; do not collapse the dual-insert; do not use the
    // funds-bearing API. Tracked: upstream-contribution TODO 9cdcfb25.
    //
    // `peek_next_funding_address` reads BOTH `wallet.accounts.*` (xpub
    // source) AND `wallet_info.accounts.*` (mutable managed account), so the
    // account must exist in both collections. `load()` rebuilds
    // `IdentityRegistration` from the manifest; per-index
    // `IdentityTopUp{registration_index}` enters the manifest only after a
    // register/top-up, so a reloaded already-registered identity needs this
    // re-provision. Idempotent: probes both collections and no-ops if present
    // (no error-string parsing — direct membership checks).
    //
    // `seed` must be the wallet's HD seed so the hardened account xpub can be
    // derived — the live wallet is watch-only and cannot derive hardened paths
    // itself. Mirrors the pattern in `register_contact_receiving_accounts`.
    async fn provision_identity_funding_account(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        funding: Funding,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;
        use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreKeysAccount;

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let mut wm = wallet.wallet_manager().write().await;
        let (kw, info) = wm
            .get_wallet_mut_and_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;

        let (in_wallet, in_managed) = match funding {
            Funding::Registration => (
                kw.accounts.identity_registration.is_some(),
                info.core_wallet.accounts.identity_registration.is_some(),
            ),
            Funding::TopUp(registration_index) => (
                kw.accounts.identity_topup.contains_key(&registration_index),
                info.core_wallet
                    .accounts
                    .identity_topup
                    .contains_key(&registration_index),
            ),
            Funding::TopUpNotBound => (
                kw.accounts.identity_topup_not_bound.is_some(),
                info.core_wallet.accounts.identity_topup_not_bound.is_some(),
            ),
        };
        if in_wallet && in_managed {
            return Ok(());
        }

        if !in_wallet {
            let account_type = match funding {
                Funding::Registration => AccountType::IdentityRegistration,
                Funding::TopUp(registration_index) => {
                    AccountType::IdentityTopUp { registration_index }
                }
                Funding::TopUpNotBound => AccountType::IdentityTopUpNotBoundToIdentity,
            };
            // The live wallet is watch-only: calling `add_account(…, None)` would
            // try to derive a hardened path from an absent private key and fail
            // with "Watch-only wallet has no private key". Derive the xpub from a
            // short-lived seed wallet instead and pass it as `Some(xpub)`.
            let seed_wallet = self.seed_wallet(seed)?;

            let path = account_type
                .derivation_path(self.inner.network)
                .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed { source })?;

            let account_xpub = seed_wallet
                .derive_extended_public_key(&path)
                .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed { source })?;

            kw.add_account(account_type, Some(account_xpub))
                .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed { source })?;
        }

        let derived = match funding {
            Funding::Registration => kw.accounts.identity_registration.as_ref(),
            Funding::TopUp(registration_index) => {
                kw.accounts.identity_topup.get(&registration_index)
            }
            Funding::TopUpNotBound => kw.accounts.identity_topup_not_bound.as_ref(),
        }
        .ok_or(TaskError::WalletStateInconsistent)?;

        let managed = ManagedCoreKeysAccount::from_account(derived);
        info.core_wallet
            .accounts
            .insert_keys_bearing_account(managed)
            .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed { source })?;
        Ok(())
    }

    /// Provision the identity-registration funding account and the per-
    /// identity top-up funding account for the given wallet identity index.
    /// Idempotent; safe to call before every asset-lock and on every reload.
    ///
    /// `seed` must be held for the duration of this call (obtained from
    /// `SecretPlaintext::expose_hd_seed` inside a `with_secret_session` scope).
    pub async fn ensure_identity_funding_accounts(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        registration_index: u32,
    ) -> Result<(), TaskError> {
        self.provision_identity_funding_account(seed_hash, seed, Funding::Registration)
            .await?;
        self.provision_identity_funding_account(seed_hash, seed, Funding::TopUp(registration_index))
            .await
    }

    /// Provision the index-less top-up funding account — the credit-output
    /// source for topping up an identity this wallet does not own. Idempotent;
    /// `seed` must be held for the duration of the call.
    pub(crate) async fn ensure_unbound_topup_funding_account(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
    ) -> Result<(), TaskError> {
        self.provision_identity_funding_account(seed_hash, seed, Funding::TopUpNotBound)
            .await
    }
}
