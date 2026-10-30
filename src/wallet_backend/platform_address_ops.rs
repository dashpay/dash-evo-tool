//! Spends from Platform addresses on [`WalletBackend`]: credit transfers,
//! withdrawals to Core, and identity top-ups.
//!
//! Every spend goes through platform-wallet, so platform-wallet's own
//! per-address balances (the ones its shield preflight and input selection
//! read) take the proof-attested post-spend values at once, instead of
//! waiting for the next platform-address sync. Each method returns the
//! entries platform-wallet actually committed, as [`PlatformAddressEntry`]s,
//! for the caller to apply to DET's model. DET reads the result back from
//! platform-wallet and never writes its own copy from the proof, so the two
//! cannot disagree.
//!
//! Platform-wallet commits nothing, and the returned list is empty, when it
//! already holds an equal or fresher balance for an address, or when its
//! platform-address provider is not initialised yet. DET then keeps its
//! previous figure until the next coordinator push, exactly like
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
    /// Transfer credits between Platform addresses through platform-wallet's
    /// `PlatformAddressWallet::transfer`.
    ///
    /// `inputs` and `outputs` go to Platform as given, and `fee_strategy` is
    /// the indexed consensus strategy. Upstream reconciles the proof-attested
    /// balances of the wallet's own addresses before returning; the result is
    /// that committed set (see the module docs for when it is empty).
    pub(crate) async fn transfer_platform_address_credits(
        &self,
        seed_hash: &WalletSeedHash,
        path_index: &PlatformPathIndex,
        inputs: BTreeMap<PlatformAddress, Credits>,
        outputs: BTreeMap<PlatformAddress, Credits>,
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
    /// its passphrase once. Returns the entries platform-wallet committed.
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
    /// identity's new balance, the height of the proof carrying it and the
    /// address entries platform-wallet committed.
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
                    .map_err(TaskError::from)
            })
            .await?;
        let entries = reconcile_on(&wallet, &address_infos, proof_height, "identity top-up").await;
        Ok((new_balance, proof_height, entries))
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
        })
        .collect()
}

/// Classify a `PlatformWalletError` from a Platform-address spend.
///
/// `Sdk` errors get the regular [`TaskError::from`] classification, the same
/// one these flows used when they called the SDK directly. Upstream promotes
/// a nonce rejection out of the SDK error, so it gets its own variant. The
/// rest go through the exhaustive [`identity_op_error_kind`] buckets, so an
/// unconfirmed broadcast never lands in an envelope that advises a retry.
fn map_platform_address_spend_error(e: platform_wallet::error::PlatformWalletError) -> TaskError {
    use platform_wallet::error::PlatformWalletError as P;
    match e {
        P::Sdk(sdk_error) => TaskError::from(sdk_error),
        nonce @ P::AddressNonceMismatch { .. } => TaskError::PlatformAddressNonceMismatch {
            source: Box::new(nonce),
        },
        other => match identity_op_error_kind(&other) {
            IdentityOpErrorKind::ConfirmationUnknown => TaskError::TransactionConfirmationUnknown {
                txid: None,
                source: Box::new(other),
            },
            IdentityOpErrorKind::Rejected
            | IdentityOpErrorKind::FinalityTimeout
            | IdentityOpErrorKind::NotManaged
            | IdentityOpErrorKind::Other => TaskError::WalletBackend {
                source: Arc::new(other),
            },
        },
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
        use dash_sdk::dpp::dashcore::hashes::Hash;
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
                },
                PlatformAddressEntry {
                    hash: [0xBB; 20],
                    balance: 55_000,
                    nonce: 1,
                    account: 1,
                    index: Some(2),
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
        let (seed_hash, wallet_arc, owned) = wallet_with_platform_address(&ctx, [0x5Au8; 64]).await;
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
        assert_eq!((entries[0].balance, entries[0].nonce), (41_000, 3));

        ctx.apply_platform_address_push(vec![(seed_hash, entries)]);

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
            wallet_with_platform_address(&ctx, [0x5Bu8; 64]).await;
        let pw = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("resolve wallet");

        let first = reconcile_on(&pw, &infos(&[(owned, 41_000, 3)]), 500, "test spend").await;
        assert_eq!(first.len(), 1, "first proof is committed: {first:?}");

        let older = reconcile_on(&pw, &infos(&[(owned, 80_000, 2)]), 400, "older proof").await;
        assert!(older.is_empty(), "older proof is skipped: {older:?}");
    }
}
