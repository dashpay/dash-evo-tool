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
use std::sync::Arc;

use super::{
    DetPlatformSigner, DetSigner, PlatformPathIndex, WalletBackend, map_identity_register_error,
    map_identity_top_up_error, map_platform_address_fund_error,
};

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
