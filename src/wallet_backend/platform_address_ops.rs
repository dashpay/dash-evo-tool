//! Spends from Platform addresses on [`WalletBackend`]: credit transfers,
//! withdrawals to Core, identity top-ups, and the proof of an address-funded
//! identity registration.
//!
//! Every spend goes through platform-wallet, so platform-wallet's own
//! per-address balances (the ones its shield preflight and input selection
//! read) take the proof-attested post-spend values at once, instead of
//! waiting for the next platform-address sync. Each method returns the
//! entries platform-wallet actually committed, as [`PlatformAddressEntry`]s
//! pinned at the proof's height, for the caller to apply to DET's model and
//! frame-safe total. DET reads the result back from platform-wallet and never
//! writes its own copy from the proof, so the two cannot disagree.
//!
//! Platform-wallet commits nothing, and the returned list is empty, when it
//! already holds an equal or fresher balance for an address, or when its
//! platform-address provider is not initialised yet. DET then keeps its
//! previous figures until the next coordinator push, exactly like
//! platform-wallet itself.

use crate::backend_task::error::TaskError;
use crate::model::wallet::{PlatformAddressEntry, WalletSeedHash};
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    DetPlatformSigner, IdentityOpErrorKind, PlatformPathIndex, WalletBackend,
    identity_op_error_kind,
};

use dash_sdk::dpp::address_funds::{AddressFundsFeeStrategy, PlatformAddress};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::query_types::AddressInfos;
use platform_wallet::PlatformAddressChangeSet;
use platform_wallet::wallet::platform_addresses::InputSelection;

/// DIP-17 platform-payment account the explicit-input spends name. Upstream
/// reads the account only to auto-select inputs; DET always passes explicit
/// inputs, so the value is informational.
const PLATFORM_PAYMENT_ACCOUNT: u32 = 0;

impl WalletBackend {
    /// Make the transaction's DET-owned payment slots reconcilable upstream.
    async fn ensure_platform_address_slots(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        path_index: &PlatformPathIndex,
        addresses: impl IntoIterator<Item = PlatformAddress>,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::KeySource;
        use dash_sdk::dpp::key_wallet::account::account_collection::PlatformPaymentAccountKey;
        use dash_sdk::dpp::key_wallet::bip32::ChildNumber;

        let mut required = Vec::new();
        let mut account_ends = BTreeMap::<u32, u32>::new();
        for address in addresses {
            let Some(path) = path_index.path_for(&address) else {
                continue; // Foreign outputs are not this wallet's slots.
            };
            let children: Vec<_> = path.into_iter().copied().collect();
            let [
                _,
                _,
                _,
                ChildNumber::Hardened { index: account },
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index },
            ] = children.as_slice()
            else {
                return Err(TaskError::WalletStateInconsistent);
            };
            let end = index
                .checked_add(1)
                .ok_or(TaskError::WalletStateInconsistent)?;
            account_ends
                .entry(*account)
                .and_modify(|current| *current = (*current).max(end))
                .or_insert(end);
            required.push((address, *account, *index));
        }
        if required.is_empty() {
            return Ok(());
        }

        // Only the manager lock is held, and it drops before any provider call.
        // Pool extension derives public children; no seed or balance is copied.
        let mut manager = wallet.wallet_manager().write().await;
        let (public_wallet, info) = manager
            .get_wallet_mut_and_info_mut(&wallet.wallet_id())
            .ok_or(TaskError::WalletStateInconsistent)?;
        for (account_index, end) in account_ends {
            let key = PlatformPaymentAccountKey {
                account: account_index,
                key_class: 0,
            };
            let xpub = public_wallet
                .accounts
                .platform_payment_accounts
                .get(&key)
                .ok_or(TaskError::WalletStateInconsistent)?
                .account_xpub;
            let account = info
                .core_wallet
                .platform_payment_managed_account_at_index_mut(account_index)
                .ok_or(TaskError::WalletStateInconsistent)?;
            let generated_end = match account.addresses.highest_generated {
                Some(index) => index
                    .checked_add(1)
                    .ok_or(TaskError::WalletStateInconsistent)?,
                None => 0,
            };
            if end > generated_end {
                account
                    .addresses
                    .generate_addresses(end - generated_end, &KeySource::Public(xpub), true)
                    .map_err(|source| TaskError::PlatformAddressPoolPreparationFailed { source })?;
            }
        }
        for (expected, account_index, index) in required {
            let generated = info
                .core_wallet
                .platform_payment_managed_account_at_index(account_index)
                .and_then(|account| account.addresses.address_at_index(index))
                .ok_or(TaskError::WalletStateInconsistent)?;
            if PlatformAddress::try_from(generated).ok() != Some(expected) {
                return Err(TaskError::WalletStateInconsistent);
            }
        }
        Ok(())
    }

    /// Transfer credits between Platform addresses through platform-wallet's
    /// `PlatformAddressWallet::transfer`.
    ///
    /// `inputs` and `outputs` go to Platform as given, and `fee_strategy` is
    /// the indexed consensus strategy. Upstream reconciles the proof-attested
    /// balances of the wallet's own addresses before returning; the result
    /// carries that committed set (see the module docs for when it is empty).
    pub(crate) async fn transfer_platform_address_credits(
        &self,
        seed_hash: &WalletSeedHash,
        path_index: &PlatformPathIndex,
        inputs: BTreeMap<PlatformAddress, Credits>,
        outputs: BTreeMap<PlatformAddress, Credits>,
        fee_strategy: AddressFundsFeeStrategy,
    ) -> Result<Vec<PlatformAddressEntry>, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        self.ensure_platform_address_slots(
            &wallet,
            path_index,
            inputs.keys().chain(outputs.keys()).copied(),
        )
        .await?;
        let scope = Self::hd_scope(seed_hash);
        let changeset = self
            .inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let signer = DetPlatformSigner::from_held(seed, self.inner.network, path_index);
                wallet
                    .platform()
                    .transfer(
                        PLATFORM_PAYMENT_ACCOUNT,
                        InputSelection::Explicit(inputs),
                        outputs,
                        fee_strategy,
                        None,
                        &signer,
                    )
                    .await
                    .map_err(map_platform_address_spend_error)
            })
            .await?;
        Ok(changeset_platform_entries(&changeset))
    }

    /// Withdraw credits from Platform addresses to a Core script through
    /// platform-wallet's `PlatformAddressWallet::withdraw` (no change output,
    /// no pooling).
    ///
    /// `prepare_path_index` runs inside the secret session with the borrowed
    /// HD seed and returns the signer's address→path index. The caller uses it
    /// to repair missing signing paths first, so a protected wallet asks for
    /// its passphrase once. Returns what platform-wallet committed.
    #[allow(clippy::too_many_arguments)] // one upstream argument each, plus the path-index hook.
    pub(crate) async fn withdraw_platform_address_credits(
        &self,
        seed_hash: &WalletSeedHash,
        prepare_path_index: impl FnOnce(&[u8; 64]) -> Result<PlatformPathIndex, TaskError>,
        inputs: BTreeMap<PlatformAddress, Credits>,
        output_script: CoreScript,
        core_fee_per_byte: u32,
        fee_strategy: AddressFundsFeeStrategy,
    ) -> Result<Vec<PlatformAddressEntry>, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let scope = Self::hd_scope(seed_hash);
        let changeset = self
            .inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let path_index = prepare_path_index(seed)?;
                self.ensure_platform_address_slots(&wallet, &path_index, inputs.keys().copied())
                    .await?;
                let signer = DetPlatformSigner::from_held(seed, self.inner.network, &path_index);
                wallet
                    .platform()
                    .withdraw(
                        PLATFORM_PAYMENT_ACCOUNT,
                        InputSelection::Explicit(inputs),
                        output_script,
                        core_fee_per_byte,
                        fee_strategy,
                        None,
                        &signer,
                    )
                    .await
                    .map_err(map_platform_address_spend_error)
            })
            .await?;
        Ok(changeset_platform_entries(&changeset))
    }

    /// Top up `identity` from this wallet's Platform addresses. Returns the
    /// identity's new balance, the height of the proof carrying it and what
    /// platform-wallet committed for the paying addresses.
    ///
    /// The SDK submits the top-up; platform-wallet's public
    /// `reconcile_address_infos` then records the spent balances, pinned at
    /// the proof's height. This is the seam upstream's composite
    /// `PlatformWallet::top_up_from_addresses` uses internally. The composite
    /// itself is not called because it only accepts identities managed by
    /// the paying wallet, and it flattens SDK failures into a string.
    pub(crate) async fn top_up_identity_from_platform_addresses(
        &self,
        seed_hash: &WalletSeedHash,
        path_index: &PlatformPathIndex,
        identity: &dash_sdk::platform::Identity,
        inputs: BTreeMap<PlatformAddress, Credits>,
    ) -> Result<(Credits, u64, Vec<PlatformAddressEntry>), TaskError> {
        use dash_sdk::platform::transition::top_up_identity_from_addresses::TopUpIdentityFromAddresses;

        // Resolve before the secret session so an unregistered wallet fails
        // fast without a passphrase prompt. The same handle reconciles after
        // the broadcast, so nothing after a successful top-up can fail.
        let wallet = self.resolve_wallet(seed_hash).await?;
        self.ensure_platform_address_slots(&wallet, path_index, inputs.keys().copied())
            .await?;
        let sdk = self.sdk().clone();
        let scope = Self::hd_scope(seed_hash);
        // TODO(upstream): switch to
        // `PlatformWallet::top_up_from_addresses` once it accepts identities
        // the paying wallet does not manage and keeps the typed SDK error.
        let (address_infos, new_balance, proof_height) = self
            .inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let signer = DetPlatformSigner::from_held(seed, self.inner.network, path_index);
                identity
                    .top_up_from_addresses(&sdk, inputs, &signer, None)
                    .await
                    .map_err(map_top_up_sdk_error)
            })
            .await?;
        let entries = reconcile_on(&wallet, &address_infos, proof_height, "identity top-up").await;
        Ok((new_balance, proof_height, entries))
    }

    /// Make the DET-owned inputs of a spend submitted outside platform-wallet
    /// (address-funded identity registration) reconcilable upstream before
    /// it is broadcast, as the spends above do. The outcome decides where the
    /// spend's proof is recorded: a wallet platform-wallet does not hold has
    /// nothing to prepare, and its proof updates DET's view alone. Any other
    /// lookup failure refuses the spend before it is broadcast.
    pub(crate) async fn prepare_platform_address_slots(
        &self,
        seed_hash: &WalletSeedHash,
        path_index: &PlatformPathIndex,
        addresses: impl IntoIterator<Item = PlatformAddress>,
    ) -> Result<PreparedPlatformSpend, TaskError> {
        match self.resolve_wallet(seed_hash).await {
            Ok(wallet) => {
                self.ensure_platform_address_slots(&wallet, path_index, addresses)
                    .await?;
                Ok(PreparedPlatformSpend(Some(wallet)))
            }
            Err(TaskError::WalletNotLoaded { .. } | TaskError::WalletLockedNotLoaded { .. }) => {
                tracing::debug!(
                    "platform-wallet does not hold the paying wallet; no slots to prepare"
                );
                Ok(PreparedPlatformSpend(None))
            }
            Err(error) => Err(error),
        }
    }
}

/// Where the proof of a spend submitted outside platform-wallet is recorded,
/// fixed when its inputs are prepared: the prepared platform-wallet, or
/// DET's view alone when platform-wallet did not hold the wallet then.
#[derive(Debug, Clone)]
pub(crate) struct PreparedPlatformSpend(Option<Arc<platform_wallet::PlatformWallet>>);

impl PreparedPlatformSpend {
    /// Record the spend's proof-attested `address_infos` in the prepared
    /// platform-wallet, pinned at `proof_height`, and return what it
    /// committed; `None` when the proof belongs to DET's view alone.
    pub(crate) async fn reconcile(
        &self,
        address_infos: &AddressInfos,
        proof_height: u64,
        context: &'static str,
    ) -> Option<Vec<PlatformAddressEntry>> {
        let wallet = self.0.as_deref()?;
        Some(reconcile_on(wallet, address_infos, proof_height, context).await)
    }
}

/// Record proof-attested address balances in `wallet`'s platform-address
/// state, pinned at `proof_height`, and return the entries it committed.
/// Addresses the wallet does not own are dropped by upstream. Infallible:
/// upstream logs and skips what it cannot apply.
async fn reconcile_on(
    wallet: &platform_wallet::PlatformWallet,
    address_infos: &AddressInfos,
    proof_height: u64,
    context: &'static str,
) -> Vec<PlatformAddressEntry> {
    let changeset = wallet
        .platform()
        .reconcile_address_infos(address_infos, proof_height, context)
        .await;
    changeset_platform_entries(&changeset)
}

/// Convert the changeset platform-wallet committed into DET's push entries.
/// Every committed entry knows its exact DIP-17 slot, so `index` is always set.
fn changeset_platform_entries(changeset: &PlatformAddressChangeSet) -> Vec<PlatformAddressEntry> {
    changeset
        .addresses
        .iter()
        .map(|entry| PlatformAddressEntry {
            hash: entry.address.to_bytes(),
            balance: entry.funds.balance,
            nonce: entry.funds.nonce,
            account: entry.account_index,
            index: Some(entry.address_index),
            as_of_height: entry.funds.as_of_height,
        })
        .collect()
}

/// Classify the SDK error of a direct identity top-up from addresses.
///
/// Promotes a nonce rejection the way upstream does for transfers and
/// withdrawals, so all three spends share [`map_platform_address_spend_error`].
fn map_top_up_sdk_error(e: dash_sdk::Error) -> TaskError {
    map_platform_address_spend_error(platform_wallet::error::promote_address_nonce_error_or_sdk(
        e,
    ))
}

/// Classify a `PlatformWalletError` from a Platform-address spend.
///
/// `Sdk` errors get the regular [`TaskError::from`] classification, the same
/// one these flows used when they called the SDK directly. Upstream promotes
/// a nonce rejection out of the SDK error, so it gets its own variant. The
/// rest go through the shared asset-lock refusal mapping, then the exhaustive
/// [`identity_op_error_kind`] buckets, so an unconfirmed broadcast never lands
/// in an envelope that advises a retry.
fn map_platform_address_spend_error(e: platform_wallet::error::PlatformWalletError) -> TaskError {
    use platform_wallet::error::PlatformWalletError as P;
    match e {
        P::Sdk(sdk_error) => TaskError::from(sdk_error),
        nonce @ P::AddressNonceMismatch { .. } => TaskError::PlatformAddressNonceMismatch {
            source: Box::new(nonce),
        },
        other => {
            super::payments::map_asset_lock_refusal_or(other, |other| match identity_op_error_kind(
                &other,
            ) {
                IdentityOpErrorKind::ConfirmationUnknown => {
                    TaskError::TransactionConfirmationUnknown {
                        txid: None,
                        source: Box::new(other),
                    }
                }
                IdentityOpErrorKind::Rejected
                | IdentityOpErrorKind::FinalityTimeout
                | IdentityOpErrorKind::NotManaged
                | IdentityOpErrorKind::Other => TaskError::WalletBackend {
                    source: Arc::new(other),
                },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::context::test_support::wired_test_app_context;
    use crate::model::wallet::Wallet;
    use crate::model::wallet::birth_height::WalletOrigin;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;
    use dash_sdk::platform::address_sync::AddressFunds;
    use dash_sdk::query_types::AddressInfo;
    use platform_wallet::PlatformAddressBalanceEntry;
    use platform_wallet::error::PlatformWalletError;
    use std::sync::RwLock;

    fn broadcast_error(consensus: dash_sdk::dpp::consensus::ConsensusError) -> PlatformWalletError {
        PlatformWalletError::Sdk(dash_sdk::Error::StateTransitionBroadcastError(
            dash_sdk::error::StateTransitionBroadcastError {
                code: 40000,
                message: "rejected".to_string(),
                cause: Some(consensus),
            },
        ))
    }

    /// An SDK error keeps the classification `TaskError::from` gives it, as
    /// when these flows called the SDK directly.
    #[test]
    fn spend_error_keeps_sdk_classification() {
        use dash_sdk::dpp::consensus::basic::identity::IdentityAssetLockTransactionOutPointAlreadyConsumedError;
        let consensus = dash_sdk::dpp::consensus::ConsensusError::from(
            IdentityAssetLockTransactionOutPointAlreadyConsumedError::new(
                dash_sdk::dpp::dashcore::Txid::from_byte_array([0u8; 32]),
                0,
            ),
        );
        let mapped = map_platform_address_spend_error(broadcast_error(consensus));
        assert!(
            matches!(mapped, TaskError::AssetLockOutPointAlreadyConsumed { .. }),
            "got {mapped:?}"
        );
    }

    /// An unclassified rejection gets the generic `PlatformRejected`, as before.
    #[test]
    fn spend_error_keeps_generic_rejection() {
        use dash_sdk::dpp::consensus::basic::UnsupportedVersionError;
        let consensus =
            dash_sdk::dpp::consensus::ConsensusError::from(UnsupportedVersionError::new(1, 2, 3));
        let mapped = map_platform_address_spend_error(broadcast_error(consensus));
        assert!(
            matches!(mapped, TaskError::PlatformRejected { .. }),
            "got {mapped:?}"
        );
    }

    #[test]
    fn spend_error_maps_nonce_mismatch() {
        let mapped = map_platform_address_spend_error(PlatformWalletError::AddressNonceMismatch {
            address: PlatformAddress::P2pkh([1u8; 20]),
            provided_nonce: 3,
            expected_nonce: 4,
        });
        assert!(
            matches!(mapped, TaskError::PlatformAddressNonceMismatch { .. }),
            "got {mapped:?}"
        );
    }

    /// A stale-nonce rejection of an identity top-up gets the same
    /// refresh-and-retry variant as transfers and withdrawals.
    #[test]
    fn top_up_error_promotes_nonce_mismatch() {
        use dash_sdk::dpp::consensus::state::address_funds::AddressInvalidNonceError;
        use dash_sdk::dpp::consensus::state::state_error::StateError;
        let consensus = dash_sdk::dpp::consensus::ConsensusError::StateError(
            StateError::AddressInvalidNonceError(AddressInvalidNonceError::new(
                PlatformAddress::P2pkh([1u8; 20]),
                3,
                4,
            )),
        );
        let PlatformWalletError::Sdk(sdk_error) = broadcast_error(consensus) else {
            unreachable!("broadcast_error builds an SDK error");
        };
        let mapped = map_top_up_sdk_error(sdk_error);
        assert!(
            matches!(mapped, TaskError::PlatformAddressNonceMismatch { .. }),
            "got {mapped:?}"
        );
    }

    /// Any other top-up failure keeps the regular SDK classification.
    #[test]
    fn top_up_error_keeps_sdk_classification() {
        use dash_sdk::dpp::consensus::basic::UnsupportedVersionError;
        let consensus =
            dash_sdk::dpp::consensus::ConsensusError::from(UnsupportedVersionError::new(1, 2, 3));
        let PlatformWalletError::Sdk(sdk_error) = broadcast_error(consensus) else {
            unreachable!("broadcast_error builds an SDK error");
        };
        let mapped = map_top_up_sdk_error(sdk_error);
        assert!(
            matches!(mapped, TaskError::PlatformRejected { .. }),
            "got {mapped:?}"
        );
    }

    /// An unconfirmed broadcast may already be on chain, so it must not get
    /// an envelope that advises sending again.
    #[test]
    fn spend_error_maps_unconfirmed_broadcast() {
        let mapped = map_platform_address_spend_error(
            PlatformWalletError::TransactionBroadcastUnconfirmed("timeout".to_string()),
        );
        assert!(
            matches!(
                mapped,
                TaskError::TransactionConfirmationUnknown { txid: None, .. }
            ),
            "got {mapped:?}"
        );
    }

    #[test]
    fn spend_error_maps_preconditions_to_wallet_backend() {
        let mapped = map_platform_address_spend_error(PlatformWalletError::AddressOperation(
            "Transfer requires at least one output address".to_string(),
        ));
        assert!(
            matches!(mapped, TaskError::WalletBackend { .. }),
            "got {mapped:?}"
        );
    }

    #[test]
    fn nonce_mismatch_message_is_plain_and_actionable() {
        let msg = TaskError::PlatformAddressNonceMismatch {
            source: Box::new(PlatformWalletError::AddressNonceMismatch {
                address: PlatformAddress::P2pkh([1u8; 20]),
                provided_nonce: 3,
                expected_nonce: 4,
            }),
        }
        .to_string();
        for jargon in ["nonce", "SDK", "state transition", "consensus"] {
            assert!(!msg.contains(jargon), "message contains {jargon}: {msg}");
        }
        assert!(msg.contains("Refresh"), "message lacks an action: {msg}");
    }

    #[test]
    fn changeset_entries_carry_slot_balance_and_nonce() {
        let changeset = PlatformAddressChangeSet {
            addresses: vec![
                PlatformAddressBalanceEntry {
                    wallet_id: [9u8; 32],
                    account_index: 0,
                    address_index: 4,
                    address: PlatformP2PKHAddress::new([0xAA; 20]),
                    funds: AddressFunds {
                        nonce: 7,
                        balance: 0,
                        as_of_height: 120,
                    },
                },
                PlatformAddressBalanceEntry {
                    wallet_id: [9u8; 32],
                    account_index: 1,
                    address_index: 2,
                    address: PlatformP2PKHAddress::new([0xBB; 20]),
                    funds: AddressFunds {
                        nonce: 1,
                        balance: 55_000,
                        as_of_height: 120,
                    },
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            changeset_platform_entries(&changeset),
            vec![
                PlatformAddressEntry {
                    hash: [0xAA; 20],
                    balance: 0,
                    nonce: 7,
                    account: 0,
                    index: Some(4),
                    as_of_height: 120,
                },
                PlatformAddressEntry {
                    hash: [0xBB; 20],
                    balance: 55_000,
                    nonce: 1,
                    account: 1,
                    index: Some(2),
                    as_of_height: 120,
                },
            ]
        );
    }

    /// Register a wallet with the context and the upstream manager, and
    /// reveal its first Platform address.
    async fn wallet_with_platform_address(
        ctx: &Arc<AppContext>,
        seed: [u8; 64],
    ) -> (WalletSeedHash, Arc<RwLock<Wallet>>, PlatformAddress) {
        let mut wallet =
            Wallet::new_from_seed(seed, Network::Testnet, Some("payer".to_string()), None)
                .expect("build wallet");
        let address = wallet
            .generate_platform_receive_address_with_seed(&seed, Network::Testnet)
            .expect("platform address");
        let platform_address = PlatformAddress::try_from(address).expect("p2pkh address");
        let (seed_hash, wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet with the context");
        ctx.wallet_backend()
            .expect("wired backend")
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("register wallet with the upstream manager");
        (seed_hash, wallet_arc, platform_address)
    }

    #[tokio::test]
    async fn issued_address_beyond_the_gap_pool_is_reconciled_after_a_spend() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("backend");
        let seed = crate::test_support::keys::seed();
        let (seed_hash, wallet_arc, _) = wallet_with_platform_address(&ctx, seed).await;
        let (owned, path_index) = {
            let mut wallet = wallet_arc.write().expect("wallet");
            let mut last = None;
            for _ in 1..=20 {
                last = Some(
                    wallet
                        .generate_platform_receive_address_with_seed(&seed, Network::Testnet)
                        .expect("issued address"),
                );
            }
            let owned =
                PlatformAddress::try_from(last.expect("slot 20")).expect("platform address");
            wallet.apply_pinned_platform_address_info(
                owned.to_address_with_network(Network::Testnet),
                1_000_000,
                1,
                500,
            );
            (
                owned,
                PlatformPathIndex::from_wallet(&wallet, Network::Testnet),
            )
        };
        let upstream = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("upstream wallet");
        let before = reconcile_on(
            &upstream,
            &infos(&[(owned, 1_000_000, 1)]),
            500,
            "unregistered slot",
        )
        .await;
        assert!(
            before.is_empty(),
            "slot 20 starts outside the managed gap pool"
        );
        backend
            .ensure_platform_address_slots(&upstream, &path_index, [owned])
            .await
            .expect("register issued slots");
        let entries =
            reconcile_on(&upstream, &infos(&[(owned, 750_000, 2)]), 600, "test spend").await;
        assert_eq!(entries.len(), 1, "the post-spend debit must be committed");
        assert_eq!(
            (entries[0].balance, entries[0].nonce, entries[0].index),
            (750_000, 2, Some(20))
        );
        assert_eq!(upstream.platform().total_credits().await, 750_000);
        ctx.apply_platform_address_spend(seed_hash, entries);
        assert_eq!(
            wallet_arc
                .read()
                .expect("wallet")
                .get_platform_address_info(&owned.to_address_with_network(Network::Testnet))
                .expect("debit")
                .balance,
            750_000
        );
    }

    /// A wallet whose platform-wallet mapping is published only after its
    /// slots were prepared keeps the proof of a spend: preparing found nothing
    /// to extend, so the proof goes to DET's view instead of a second lookup
    /// that would reconcile against a pool missing the spent slot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spend_proof_follows_the_prepared_outcome_when_the_mapping_appears_late() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("wired backend");
        let seed = crate::test_support::keys::seed();
        let mut wallet =
            Wallet::new_from_seed(seed, Network::Testnet, Some("payer".to_string()), None)
                .expect("build wallet");
        let mut issued = None;
        for _ in 0..=20 {
            issued = Some(
                wallet
                    .generate_platform_receive_address_with_seed(&seed, Network::Testnet)
                    .expect("issued address"),
            );
        }
        let spent = issued.expect("slot 20");
        let funding = PlatformAddress::try_from(spent.clone()).expect("p2pkh");
        let path_index = PlatformPathIndex::from_wallet(&wallet, Network::Testnet);
        let (seed_hash, wallet_arc) = ctx
            .register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet with the context");

        let prepared = backend
            .prepare_platform_address_slots(&seed_hash, &path_index, [funding])
            .await
            .expect("an unmapped wallet has nothing to prepare");
        backend
            .register_wallet_from_seed(&seed_hash, &seed, Some(0))
            .await
            .expect("the mapping appears before the proof is handled");
        ctx.record_platform_address_spend_proof(
            seed_hash,
            &prepared,
            &infos(&[(funding, 250_000, 2)]),
            600,
            "test registration",
        )
        .await;

        let wallet = wallet_arc.read().expect("wallet lock");
        let det = wallet
            .get_platform_address_info(&spent)
            .expect("DET records the spend");
        assert_eq!(
            (det.balance, det.nonce, det.as_of_height),
            (250_000, 2, 600)
        );
    }

    /// A poisoned wallet map is not an unloaded wallet: preparing refuses
    /// before anything is broadcast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn preparing_slots_refuses_a_poisoned_wallet_map() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("wired backend");
        let (seed_hash, wallet_arc, owned) =
            wallet_with_platform_address(&ctx, rand::random::<[u8; 64]>()).await;
        let path_index =
            PlatformPathIndex::from_wallet(&wallet_arc.read().expect("wallet"), Network::Testnet);
        let poisoner = Arc::clone(&backend);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.inner.id_map.write().expect("id map");
            panic!("poison the wallet map");
        })
        .join();

        let prepared = backend
            .prepare_platform_address_slots(&seed_hash, &path_index, [owned])
            .await;
        assert!(prepared.is_err(), "{prepared:?}");
    }

    fn infos(entries: &[(PlatformAddress, Credits, u32)]) -> AddressInfos {
        let mut infos = AddressInfos::new();
        for &(address, balance, nonce) in entries {
            infos.insert(
                address,
                Some(AddressInfo {
                    address,
                    balance,
                    nonce,
                }),
            );
        }
        infos
    }

    /// After a spend, platform-wallet holds the proof's balance for the
    /// wallet's own address and DET's model, fed from what platform-wallet
    /// committed, shows the same figure. A third-party recipient in the same
    /// proof is dropped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconciled_spend_updates_platform_wallet_and_det_model_alike() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("wired backend");
        let (seed_hash, wallet_arc, owned) =
            wallet_with_platform_address(&ctx, rand::random::<[u8; 64]>()).await;
        let foreign = PlatformAddress::P2pkh([0x99; 20]);
        let pw = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("resolve wallet");

        let entries = reconcile_on(
            &pw,
            &infos(&[(owned, 41_000, 3), (foreign, 900, 1)]),
            500,
            "test spend",
        )
        .await;
        assert_eq!(entries.len(), 1, "only the owned address: {entries:?}");
        let PlatformAddress::P2pkh(owned_hash) = owned else {
            panic!("p2pkh address expected");
        };
        assert_eq!(entries[0].hash, owned_hash);
        assert_eq!(
            (
                entries[0].balance,
                entries[0].nonce,
                entries[0].as_of_height
            ),
            (41_000, 3, 500)
        );

        ctx.apply_platform_address_spend(seed_hash, entries);
        assert_eq!(ctx.platform_balance_duffs(&seed_hash), 41);

        let upstream: BTreeMap<_, _> = pw
            .platform()
            .addresses_with_balances()
            .await
            .into_iter()
            .collect();
        assert_eq!(upstream.get(&owned), Some(&41_000));
        assert!(!upstream.contains_key(&foreign));

        let core_owned = owned.to_address_with_network(Network::Testnet);
        let wallet = wallet_arc.read().expect("wallet lock");
        let det = wallet
            .platform_address_info
            .get(&core_owned)
            .expect("DET has the owned address");
        assert_eq!((det.balance, det.nonce), (41_000, 3));
    }

    /// When platform-wallet already holds the same or a fresher figure, it
    /// commits nothing and DET's model is left alone until the next push.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reconcile_commits_nothing_when_platform_wallet_is_not_behind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("wired backend");
        let (seed_hash, _wallet_arc, owned) =
            wallet_with_platform_address(&ctx, rand::random::<[u8; 64]>()).await;
        let pw = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("resolve wallet");

        let first = reconcile_on(&pw, &infos(&[(owned, 41_000, 3)]), 500, "test spend").await;
        assert_eq!(first.len(), 1, "first proof is committed: {first:?}");

        let older = reconcile_on(&pw, &infos(&[(owned, 80_000, 2)]), 400, "older proof").await;
        assert!(older.is_empty(), "older proof is skipped: {older:?}");
    }

    /// A spend submitted outside platform-wallet (address-funded identity
    /// registration) reaches platform-wallet, DET's model and the wallet
    /// total from its proof alone. An older proof or pass applied later
    /// changes none of them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spend_proof_reaches_platform_wallet_det_model_and_total() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = wired_test_app_context(dir.path()).await;
        let backend = ctx.wallet_backend().expect("wired backend");
        let (seed_hash, wallet_arc, owned) =
            wallet_with_platform_address(&ctx, rand::random::<[u8; 64]>()).await;
        let foreign = PlatformAddress::P2pkh([0x99; 20]);
        let PlatformAddress::P2pkh(owned_hash) = owned else {
            panic!("p2pkh address expected");
        };
        let prepared = PreparedPlatformSpend(Some(
            backend
                .resolve_wallet(&seed_hash)
                .await
                .expect("held wallet"),
        ));

        ctx.record_platform_address_spend_proof(
            seed_hash,
            &prepared,
            &infos(&[(owned, 41_000, 3), (foreign, 900, 1)]),
            500,
            "test registration",
        )
        .await;

        // Older results arriving afterwards: a proof, then a coordinator pass.
        ctx.record_platform_address_spend_proof(
            seed_hash,
            &prepared,
            &infos(&[(owned, 80_000, 2)]),
            400,
            "older proof",
        )
        .await;
        // The event bridge writes a coordinator pass into the wallet this way.
        wallet_arc
            .write()
            .expect("wallet lock")
            .apply_platform_address_entries(
                &[PlatformAddressEntry {
                    hash: owned_hash,
                    balance: 80_000,
                    nonce: 2,
                    account: 0,
                    index: Some(0),
                    as_of_height: 400,
                }],
                Network::Testnet,
            );

        assert_eq!(ctx.platform_balance_duffs(&seed_hash), 41);

        let pw = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("resolve wallet");
        let upstream: BTreeMap<_, _> = pw
            .platform()
            .addresses_with_balances()
            .await
            .into_iter()
            .collect();
        assert_eq!(upstream.get(&owned), Some(&41_000));
        assert!(!upstream.contains_key(&foreign));

        let core_owned = owned.to_address_with_network(Network::Testnet);
        let wallet = wallet_arc.read().expect("wallet lock");
        let det = wallet
            .platform_address_info
            .get(&core_owned)
            .expect("DET has the owned address");
        assert_eq!(
            (det.balance, det.nonce, det.as_of_height),
            (41_000, 3, 500),
            "the coordinator pass merges into this pin"
        );
        assert!(
            !wallet
                .platform_address_info
                .contains_key(&foreign.to_address_with_network(Network::Testnet)),
            "a foreign recipient stays out of DET's model"
        );
    }

    /// Without platform-wallet holding the wallet, the proof still updates
    /// DET's model, pinned at its height, as it did before platform-wallet
    /// reconciled these spends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spend_proof_without_platform_wallet_still_updates_det_model() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let seed = rand::random::<[u8; 64]>();
        let mut wallet =
            Wallet::new_from_seed(seed, Network::Testnet, None, None).expect("build wallet");
        let address = wallet
            .generate_platform_receive_address_with_seed(&seed, Network::Testnet)
            .expect("platform address");
        let owned = PlatformAddress::try_from(address.clone()).expect("p2pkh address");
        let seed_hash = wallet.seed_hash();
        let wallet_arc = Arc::new(RwLock::new(wallet));
        ctx.wallet_context()
            .insert_test_wallet(seed_hash, Arc::clone(&wallet_arc));
        let prepared = PreparedPlatformSpend(None);

        ctx.record_platform_address_spend_proof(
            seed_hash,
            &prepared,
            &infos(&[(owned, 41_000, 3)]),
            500,
            "test registration",
        )
        .await;
        ctx.record_platform_address_spend_proof(
            seed_hash,
            &prepared,
            &infos(&[(owned, 80_000, 2)]),
            400,
            "older proof",
        )
        .await;

        let wallet = wallet_arc.read().expect("wallet lock");
        let det = wallet
            .get_platform_address_info(&address)
            .expect("DET has the owned address");
        assert_eq!((det.balance, det.nonce), (41_000, 3));
    }
}
