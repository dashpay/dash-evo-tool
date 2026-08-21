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
    DetPlatformSigner, DetSigner, PlatformPathIndex, WalletBackend, WalletId,
    map_identity_register_error, map_identity_top_up_error, map_platform_address_fund_error,
};

use dash_sdk::dpp::key_wallet::account::Account;
use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey;
use platform_wallet::changeset::{
    AccountRegistrationEntry, PersistenceError, PlatformWalletChangeSet, PlatformWalletPersistence,
};

/// Total attempts for a transient persister failure on the funding-account
/// path. Mirrors upstream's `PERSIST_RETRY_MAX_ATTEMPTS` on the
/// wallet-registration path, which is `pub(super)` there and so unreachable
/// from DET.
const PERSIST_RETRY_MAX_ATTEMPTS: u32 = 4;

/// Backoff before the first retry; doubles up to [`PERSIST_RETRY_MAX_BACKOFF`].
const PERSIST_RETRY_INITIAL_BACKOFF: std::time::Duration = std::time::Duration::from_millis(20);

/// Ceiling for the doubling backoff, so a busy database still surfaces
/// promptly (worst case with the constants above: 20 + 40 + 80 ≈ 140 ms).
const PERSIST_RETRY_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_millis(200);

/// The two identity-funding account flavours DET provisions. Parsing the
/// caller's intent into this once (at [`WalletBackend::ensure_identity_funding_accounts`])
/// removes the repeated `AccountType` matches — and their `unreachable!` arms —
/// inside [`WalletBackend::provision_identity_funding_account`], and makes the
/// unsupported-account-type case unrepresentable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Funding {
    /// The identity-registration funding account.
    Registration,
    /// The per-identity top-up funding account at the given registration index.
    TopUp(u32),
    /// The single top-up funding account bound to no identity index — what a
    /// top-up of an identity this wallet does not own draws its credit output
    /// from, since no index in this wallet's tree describes that identity.
    TopUpNotBound,
}

impl Funding {
    /// The upstream account type this funding flavour derives and registers.
    fn account_type(self) -> dash_sdk::dpp::key_wallet::AccountType {
        use dash_sdk::dpp::key_wallet::AccountType;
        match self {
            Self::Registration => AccountType::IdentityRegistration,
            Self::TopUp(registration_index) => AccountType::IdentityTopUp { registration_index },
            Self::TopUpNotBound => AccountType::IdentityTopUpNotBoundToIdentity,
        }
    }

    /// Whether the managed (info-side) collection holds this funding account.
    fn is_managed(
        self,
        info: &platform_wallet::wallet::platform_wallet::PlatformWalletInfo,
    ) -> bool {
        match self {
            Self::Registration => info.core_wallet.accounts.identity_registration.is_some(),
            Self::TopUp(registration_index) => info
                .core_wallet
                .accounts
                .identity_topup
                .contains_key(&registration_index),
            Self::TopUpNotBound => info.core_wallet.accounts.identity_topup_not_bound.is_some(),
        }
    }

    /// This funding account as the key-wallet holds it, if at all.
    fn account(self, kw: &dash_sdk::dpp::key_wallet::wallet::Wallet) -> Option<&Account> {
        match self {
            Self::Registration => kw.accounts.identity_registration.as_ref(),
            Self::TopUp(registration_index) => kw.accounts.identity_topup.get(&registration_index),
            Self::TopUpNotBound => kw.accounts.identity_topup_not_bound.as_ref(),
        }
    }

    /// Drop this funding account from both in-memory collections, so the next
    /// provisioning attempt re-creates and re-persists it instead of
    /// short-circuiting on the presence guards.
    fn remove(
        self,
        kw: &mut dash_sdk::dpp::key_wallet::wallet::Wallet,
        info: &mut platform_wallet::wallet::platform_wallet::PlatformWalletInfo,
    ) {
        match self {
            Self::Registration => {
                kw.accounts.identity_registration = None;
                info.core_wallet.accounts.identity_registration = None;
            }
            Self::TopUp(registration_index) => {
                kw.accounts.identity_topup.remove(&registration_index);
                info.core_wallet
                    .accounts
                    .identity_topup
                    .remove(&registration_index);
            }
            Self::TopUpNotBound => {
                kw.accounts.identity_topup_not_bound = None;
                info.core_wallet.accounts.identity_topup_not_bound = None;
            }
        }
    }
}

/// One provisioned identity-funding account as the wallet holds it, for
/// asserting that a restart restored the same account rather than a
/// same-shaped placeholder.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FundingAccountView {
    /// Account type on the key-wallet account — carries the registration index
    /// for [`AccountType::IdentityTopUp`](dash_sdk::dpp::key_wallet::AccountType::IdentityTopUp).
    pub account_type: dash_sdk::dpp::key_wallet::AccountType,
    /// The account xpub funding-address derivation reads.
    pub account_xpub: dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey,
    /// Whether the managed (info-side) collection also holds the account.
    pub managed: bool,
}

/// Why an account registration did not reach disk — and therefore what the
/// caller must do with the account it just put in memory.
enum RegistrationPersistFailure {
    /// The retry budget ran out while the failure was still transient. The
    /// changeset may still be buffered and land later, so the account MUST
    /// stay in memory — dropping it while the row can still appear would leave
    /// a manifest the live wallet disagrees with. The registration is recorded
    /// as pending and rewritten by the next attempt.
    Staged(PersistenceError),
    /// A terminal failure discarded the staged changeset. The row will never
    /// land, so the account must leave memory.
    Discarded(PersistenceError),
}

impl RegistrationPersistFailure {
    fn into_task_error(self) -> TaskError {
        let (Self::Staged(source) | Self::Discarded(source)) = self;
        TaskError::IdentityFundingAccountPersistFailed {
            source: Box::new(source),
        }
    }
}

/// Retry `op` while it fails transiently, with bounded exponential backoff,
/// and classify whatever error survives.
///
/// Mirrors upstream's `retry_transient` on the wallet-registration path. The
/// sleep is async so it yields the Tokio worker rather than spinning.
async fn retry_transient<F>(mut op: F) -> Result<(), RegistrationPersistFailure>
where
    F: FnMut() -> Result<(), PersistenceError>,
{
    let mut backoff = PERSIST_RETRY_INITIAL_BACKOFF;
    let mut attempt: u32 = 1;
    loop {
        match op() {
            Ok(()) => return Ok(()),
            Err(e) if !e.is_transient() => return Err(RegistrationPersistFailure::Discarded(e)),
            Err(e) if attempt >= PERSIST_RETRY_MAX_ATTEMPTS => {
                return Err(RegistrationPersistFailure::Staged(e));
            }
            Err(e) => {
                tracing::debug!(
                    attempt,
                    max_attempts = PERSIST_RETRY_MAX_ATTEMPTS,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %e,
                    "identity-funding account registration hit a transient persister failure; backing off before rewriting the registration"
                );
                tokio::time::sleep(backoff).await;
                backoff = backoff.saturating_mul(2).min(PERSIST_RETRY_MAX_BACKOFF);
                attempt += 1;
            }
        }
    }
}

/// Write these account registrations through `persister` and flush them, so a
/// cold boot rebuilds the accounts from the manifest.
///
/// Every attempt — the first and each retry — **resupplies** the entries via
/// `store` rather than issuing a bare `flush`. The persister's buffer is
/// shared per wallet, not owned by this call site: `flush` answers `Ok(())`
/// when it finds nothing staged, and any other writer's terminal flush takes
/// the whole merged buffer (these entries included) without telling us. A bare
/// `flush` therefore cannot distinguish "committed" from "thrown away by
/// somebody else", and inferring durability from it is how a funding account
/// ends up live in memory and absent from the manifest.
///
/// Resupplying is safe precisely here, and the general "re-`store` would
/// double-merge" caution does not bind: `account_registrations` merges by
/// `extend` and applies through `UPSERT_ACCOUNT_SQL`, an
/// `ON CONFLICT(wallet_id, account_type, account_index, …) DO UPDATE` keyed on
/// the account's identity, so writing the same entry twice is idempotent.
async fn persist_account_registrations(
    persister: &dyn PlatformWalletPersistence,
    wallet_id: WalletId,
    entries: &[AccountRegistrationEntry],
) -> Result<(), RegistrationPersistFailure> {
    retry_transient(|| {
        let changeset = PlatformWalletChangeSet {
            account_registrations: entries.to_vec(),
            ..Default::default()
        };
        persister
            .store(wallet_id, changeset)
            .and_then(|()| persister.flush(wallet_id))
    })
    .await
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
    // (no error-string parsing — direct membership checks) — except when a
    // previous transient persist failure left the registration staged, which
    // this call finishes rather than reporting success on an absent row.
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
        use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreKeysAccount;

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();

        // Everything that touches wallet state happens under the manager
        // guard; the persist below deliberately does not. `wallet_manager()`
        // hands out ONE `RwLock<WalletManager>` shared by every wallet in the
        // process, and it is write-preferring, so sleeping a retry backoff
        // under it would stall balance reads, sync callbacks and wallet
        // removal for every other wallet — upstream warns about exactly this.
        let pending = {
            let mut wm = wallet.wallet_manager().write().await;
            let (kw, info) = wm
                .get_wallet_mut_and_info_mut(&wallet_id)
                .ok_or(TaskError::WalletStateInconsistent)?;

            let in_wallet = funding.account(kw).is_some();
            let in_managed = funding.is_managed(info);

            // Registrations a previous attempt could not make durable. They
            // are rewritten below: returning success while one is outstanding
            // would leave its account live in memory and absent from the
            // manifest.
            let mut pending = self.staged_account_registrations(&wallet_id)?;
            if in_wallet && in_managed && pending.is_empty() {
                return Ok(());
            }

            if !in_wallet {
                let account_type = funding.account_type();
                // The live wallet is watch-only: calling `add_account(…, None)` would
                // try to derive a hardened path from an absent private key and fail
                // with "Watch-only wallet has no private key". Derive the xpub from a
                // short-lived seed wallet instead and pass it as `Some(xpub)`.
                let seed_wallet = self.seed_wallet(seed)?;

                let path = account_type
                    .derivation_path(self.inner.network)
                    .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed {
                        source,
                    })?;

                let account_xpub =
                    seed_wallet
                        .derive_extended_public_key(&path)
                        .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed {
                            source,
                        })?;

                kw.add_account(account_type, Some(account_xpub))
                    .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed {
                        source,
                    })?;
            }

            if !(in_wallet && in_managed) {
                let derived = funding
                    .account(kw)
                    .ok_or(TaskError::WalletStateInconsistent)?;
                let account_xpub = derived.account_xpub;
                let managed = ManagedCoreKeysAccount::from_account(derived);
                info.core_wallet
                    .accounts
                    .insert_keys_bearing_account(managed)
                    .map_err(|source| TaskError::IdentityFundingAccountProvisionFailed {
                        source,
                    })?;
                pending.insert(funding, account_xpub);
            }

            pending
        };

        // In test builds the write goes through a fault-injecting decorator
        // that is inert until a test arms it; production talks to the
        // persister directly.
        #[cfg(not(test))]
        let persister: &dyn PlatformWalletPersistence = self.inner.wallet_persister.as_ref();
        #[cfg(test)]
        let injector = super::persist_fault_test_support::PersistFaultInjector::new(
            &self.inner.wallet_persister,
            &self.inner.persist_faults,
        );
        #[cfg(test)]
        let persister: &dyn PlatformWalletPersistence = &injector;

        // Persist the registrations. `load()` rebuilds `Wallet.accounts` from
        // `account_registrations` alone, and the upstream creator that would
        // normally write these rows skips them once both in-memory collections
        // hold the account — which they do by the time it runs, because of the
        // inserts above. Without this the account is memory-only: a restart
        // between an asset-lock broadcast and its consumption cannot re-derive
        // the credit-output path, stranding the lock and its funds.
        let entries: Vec<AccountRegistrationEntry> = pending
            .iter()
            .map(|(funding, account_xpub)| AccountRegistrationEntry {
                account_type: funding.account_type(),
                account_xpub: *account_xpub,
            })
            .collect();

        let failure = match persist_account_registrations(persister, wallet_id, &entries).await {
            Ok(()) => return self.clear_staged_account_registrations(&wallet_id),
            Err(failure) => failure,
        };

        // Keep the accounts only when the write is still worth retrying AND
        // that intent was actually recorded. A marker we could not write is a
        // silent dead end: the account would stay in memory with nothing
        // scheduled to persist it, and the next call would early-return
        // success on a row that is not there.
        let recoverable = matches!(failure, RegistrationPersistFailure::Staged(_))
            && match self.mark_account_registrations_staged(wallet_id, &pending) {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(
                        wallet = %hex::encode(wallet_id),
                        error = ?e,
                        "could not record a pending identity-funding registration; dropping the account so a retry rebuilds it"
                    );
                    false
                }
            };

        if !recoverable {
            // The staged changeset is gone (or unrecoverable), so every
            // account it covered must leave memory — otherwise the presence
            // guards short-circuit persists that never happened.
            self.evict_funding_accounts(&wallet, wallet_id, &pending)
                .await;
            if let Err(e) = self.clear_staged_account_registrations(&wallet_id) {
                tracing::error!(
                    wallet = %hex::encode(wallet_id),
                    error = ?e,
                    "could not clear pending identity-funding registrations after a terminal persist failure"
                );
            }
        }

        Err(failure.into_task_error())
    }

    /// Drop funding accounts whose registrations did not reach disk from both
    /// in-memory collections, so the next attempt re-creates and re-persists
    /// them.
    ///
    /// Re-acquires the manager guard, which was released for the persist, and
    /// only removes an account still carrying the xpub we tried to write — a
    /// concurrent provisioning that re-created it has its own durable row and
    /// must not be evicted on our behalf.
    async fn evict_funding_accounts(
        &self,
        wallet: &Arc<platform_wallet::PlatformWallet>,
        wallet_id: WalletId,
        pending: &std::collections::BTreeMap<Funding, ExtendedPubKey>,
    ) {
        let mut wm = wallet.wallet_manager().write().await;
        let Some((kw, info)) = wm.get_wallet_mut_and_info_mut(&wallet_id) else {
            return;
        };
        for (funding, account_xpub) in pending {
            if funding
                .account(kw)
                .is_some_and(|a| a.account_xpub == *account_xpub)
            {
                funding.remove(kw, info);
            }
        }
    }

    /// The funding accounts whose registration a previous attempt could not
    /// make durable, with the account xpub each one still needs written.
    fn staged_account_registrations(
        &self,
        wallet_id: &WalletId,
    ) -> Result<std::collections::BTreeMap<Funding, ExtendedPubKey>, TaskError> {
        Ok(self
            .inner
            .buffered_account_registrations
            .lock()?
            .get(wallet_id)
            .cloned()
            .unwrap_or_default())
    }

    /// Record registrations for the next provisioning attempt on this wallet
    /// to rewrite, instead of early-returning success on absent rows.
    fn mark_account_registrations_staged(
        &self,
        wallet_id: WalletId,
        pending: &std::collections::BTreeMap<Funding, ExtendedPubKey>,
    ) -> Result<(), TaskError> {
        self.inner
            .buffered_account_registrations
            .lock()?
            .entry(wallet_id)
            .or_default()
            .extend(pending.iter().map(|(f, x)| (*f, *x)));
        Ok(())
    }

    /// Forget this wallet's pending registrations, once they are durable or
    /// definitively lost.
    fn clear_staged_account_registrations(&self, wallet_id: &WalletId) -> Result<(), TaskError> {
        self.inner
            .buffered_account_registrations
            .lock()?
            .remove(wallet_id);
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

    /// The provisioned identity-funding account for `account_type`, as seen by
    /// the two collections that must both hold it.
    ///
    /// `peek_next_funding_address` reads the account xpub from
    /// `Wallet.accounts` and the address pool from the managed collection, so
    /// a restore that repopulates only one side leaves funding broken. Returns
    /// `None` when the key-wallet side has no such account.
    #[cfg(test)]
    pub(crate) async fn identity_funding_account(
        &self,
        seed_hash: &WalletSeedHash,
        account_type: dash_sdk::dpp::key_wallet::AccountType,
    ) -> Result<Option<FundingAccountView>, TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let wm = wallet.wallet_manager().read().await;
        let (kw, info) = wm
            .get_wallet_and_info(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;

        let (derived, managed) = match account_type {
            AccountType::IdentityRegistration => (
                kw.accounts.identity_registration.as_ref(),
                info.core_wallet.accounts.identity_registration.is_some(),
            ),
            AccountType::IdentityTopUp { registration_index } => (
                kw.accounts.identity_topup.get(&registration_index),
                info.core_wallet
                    .accounts
                    .identity_topup
                    .contains_key(&registration_index),
            ),
            AccountType::IdentityTopUpNotBoundToIdentity => (
                kw.accounts.identity_topup_not_bound.as_ref(),
                info.core_wallet.accounts.identity_topup_not_bound.is_some(),
            ),
            _ => return Err(TaskError::WalletStateInconsistent),
        };

        Ok(derived.map(|account| FundingAccountView {
            account_type: account.account_type,
            account_xpub: account.account_xpub,
            managed,
        }))
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
