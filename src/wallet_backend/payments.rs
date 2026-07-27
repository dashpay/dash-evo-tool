//! Payment and asset-lock operations on [`WalletBackend`] — the funds-signing
//! path.
//!
//! Every seed-bearing operation here opens a single just-in-time
//! [`SecretAccess`](super::SecretAccess) session so the HD seed is decrypted
//! once, borrowed by the [`DetSigner`] for signing, and zeroized when the
//! scope ends. `send_payment` builds and broadcasts a BIP-44 payment;
//! `create_asset_lock_proof` builds a non-identity asset lock and returns its
//! one-time credit-output key.

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::dashcore::blockdata::constants::MAX_MONEY;
use dash_sdk::dpp::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
use dash_sdk::dpp::dashcore::{ScriptBuf, TxOut};
use dash_sdk::dpp::key_wallet::account::Account;
use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreFundsAccount;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::{
    SelectionError, SelectionStrategy,
};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeRate;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::{
    BuilderError, TransactionBuilder,
};
use std::sync::Arc;

use super::{DEFAULT_BIP44_ACCOUNT, DetSigner, SecretPlaintext, WalletBackend};

// `AssetLockManager` passes its private `DEFAULT_FEE_PER_KB` explicitly to key-wallet, so DET
// cannot reuse the real path's source.
// TODO(upstream): export that default or expose an asset-lock ceiling quote primitive.
const ASSET_LOCK_FEE_PER_KB: u64 = 1_000;
const MAX_DRAIN_SEARCH_DOUBLINGS: u32 = 40;

enum AssetLockDryRun {
    Builds,
    Rejected { available: u64 },
}

enum AssetLockDrainSeed {
    Ceiling(u64),
    Unavailable,
    TooManyInputs,
}

fn asset_lock_builder_height(
    info: &dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo,
) -> u32 {
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

    info.last_processed_height()
}

fn dry_run_asset_lock_amount(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    amount_duffs: u64,
) -> Result<AssetLockDryRun, BuilderError> {
    let mut dry_run_account = managed_account.clone();
    let result = TransactionBuilder::new()
        .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
        .set_current_height(current_height)
        .set_special_payload(TransactionPayload::AssetLockPayloadType(
            AssetLockPayload::new(vec![TxOut {
                value: amount_duffs,
                script_pubkey: ScriptBuf::new(),
            }]),
        ))
        .set_funding(&mut dry_run_account, account)
        .require_final_inputs()
        .build_unsigned();

    match result {
        Ok((transaction, _)) => {
            dry_run_account.release_reservation(&transaction);
            Ok(AssetLockDryRun::Builds)
        }
        Err(BuilderError::CoinSelection(SelectionError::NoUtxosAvailable)) => {
            Ok(AssetLockDryRun::Rejected { available: 0 })
        }
        Err(BuilderError::CoinSelection(SelectionError::InsufficientFunds {
            available, ..
        }))
        | Err(BuilderError::InsufficientFunds { available, .. }) => {
            Ok(AssetLockDryRun::Rejected { available })
        }
        Err(BuilderError::TooManyInputs { .. }) => Ok(AssetLockDryRun::Rejected { available: 0 }),
        Err(source) => Err(source),
    }
}

fn asset_lock_drain_ceiling(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
) -> Result<AssetLockDrainSeed, BuilderError> {
    let mut dry_run_account = managed_account.clone();
    let result = TransactionBuilder::new()
        .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
        .set_current_height(current_height)
        .set_selection_strategy(SelectionStrategy::All)
        .set_special_payload(TransactionPayload::AssetLockPayloadType(
            AssetLockPayload::new(vec![TxOut {
                value: 1,
                script_pubkey: ScriptBuf::new(),
            }]),
        ))
        .set_funding(&mut dry_run_account, account)
        .require_final_inputs()
        .build_unsigned();

    match result {
        Ok((transaction, _)) => {
            dry_run_account.release_reservation(&transaction);
            transaction
                .output
                .first()
                .map(|output| AssetLockDrainSeed::Ceiling(output.value))
                .ok_or_else(|| {
                    BuilderError::InvalidData("asset-lock drain produced no credit output".into())
                })
        }
        Err(BuilderError::CoinSelection(SelectionError::NoUtxosAvailable))
        | Err(BuilderError::CoinSelection(SelectionError::InsufficientFunds { .. }))
        | Err(BuilderError::InsufficientFunds { .. }) => Ok(AssetLockDrainSeed::Unavailable),
        Err(BuilderError::TooManyInputs { .. }) => Ok(AssetLockDrainSeed::TooManyInputs),
        Err(source) => Err(source),
    }
}

fn full_range_asset_lock_max_amount(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
) -> Result<u64, BuilderError> {
    let mut high = match dry_run_asset_lock_amount(
        managed_account,
        account,
        current_height,
        MAX_MONEY.saturating_add(1),
    )? {
        AssetLockDryRun::Rejected { available } => available,
        AssetLockDryRun::Builds => MAX_MONEY,
    };
    let mut low = 0;

    while low < high {
        let candidate = low + (high - low).div_ceil(2);
        match dry_run_asset_lock_amount(managed_account, account, current_height, candidate)? {
            AssetLockDryRun::Builds => low = candidate,
            AssetLockDryRun::Rejected { .. } => high = candidate - 1,
        }
    }

    Ok(low)
}

/// Return the largest credit-output amount the real asset-lock builder accepts.
///
/// A drain build supplies a tight upper bound, then default-strategy probes find
/// the exact boundary without reproducing the selector's internal fee model.
fn asset_lock_max_amount_from_account(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
) -> Result<u64, BuilderError> {
    let drain_ceiling = match asset_lock_drain_ceiling(managed_account, account, current_height)? {
        AssetLockDrainSeed::Ceiling(ceiling) => ceiling,
        AssetLockDrainSeed::Unavailable => return Ok(0),
        AssetLockDrainSeed::TooManyInputs => {
            return full_range_asset_lock_max_amount(managed_account, account, current_height);
        }
    };
    let mut step = 1_u64;
    let mut high = drain_ceiling;
    for _ in 0..MAX_DRAIN_SEARCH_DOUBLINGS {
        let candidate = drain_ceiling.saturating_sub(step);
        match dry_run_asset_lock_amount(managed_account, account, current_height, candidate)? {
            AssetLockDryRun::Builds => {
                let mut low = candidate;
                while low < high {
                    let midpoint = low + (high - low).div_ceil(2);
                    match dry_run_asset_lock_amount(
                        managed_account,
                        account,
                        current_height,
                        midpoint,
                    )? {
                        AssetLockDryRun::Builds => low = midpoint,
                        AssetLockDryRun::Rejected { .. } => high = midpoint - 1,
                    }
                }
                return Ok(low);
            }
            AssetLockDryRun::Rejected { .. } if candidate == 0 => return Ok(0),
            AssetLockDryRun::Rejected { .. } => {
                high = candidate - 1;
                step = step.saturating_mul(2);
            }
        }
    }

    full_range_asset_lock_max_amount(managed_account, account, current_height)
}

impl WalletBackend {
    /// Query the largest asset-lock credit output accepted by the live wallet.
    ///
    /// This is the read-only counterpart of [`Self::create_asset_lock_proof`].
    /// It runs off the UI thread against the upstream wallet manager and never
    /// exposes or reconstructs UTXO selection in the UI.
    pub async fn asset_lock_max_amount(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<u64, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let mut wallet_manager = wallet.wallet_manager().write().await;
        let (key_wallet, info) = wallet_manager
            .get_wallet_and_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        let account = key_wallet
            .get_bip44_account(DEFAULT_BIP44_ACCOUNT)
            .ok_or(TaskError::WalletStateInconsistent)?
            .clone();
        let current_height = asset_lock_builder_height(&info.core_wallet);
        let managed_account = info
            .core_wallet
            .accounts
            .standard_bip44_accounts
            .get(&DEFAULT_BIP44_ACCOUNT)
            .ok_or(TaskError::WalletStateInconsistent)?;

        asset_lock_max_amount_from_account(managed_account, &account, current_height).map_err(
            |source| TaskError::AssetLockBalanceQueryFailed {
                source: Box::new(source),
            },
        )
    }

    /// Derive the secp256k1 [`PrivateKey`](dash_sdk::dpp::dashcore::PrivateKey) at `path` from a held HD seed.
    /// Used after `create_asset_lock_proof` to obtain the one-time
    /// credit-output key needed to sign DET-retained non-identity state
    /// transitions (Platform-address top-up, shielded deposit). The seed is
    /// the one already held open by the surrounding `with_secret_session`
    /// scope, so this never re-prompts.
    fn derive_private_key_from_held(
        &self,
        plaintext: SecretPlaintext<'_>,
        path: &dash_sdk::dpp::key_wallet::bip32::DerivationPath,
    ) -> Result<dash_sdk::dpp::dashcore::PrivateKey, TaskError> {
        let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
        let xprv = path
            .derive_priv_ecdsa_for_master_seed(seed, self.inner.network)
            .map_err(|source| TaskError::WalletBackend {
                source: Arc::new(platform_wallet::error::PlatformWalletError::KeyDerivation(
                    source.to_string(),
                )),
            })?;
        Ok(xprv.to_priv())
    }

    /// Test-only probe that the chokepoint can decrypt the seed for
    /// `seed_hash` without a prompt (the no-password / unprotected fast-path)
    /// AND that the resulting [`DetSigner`] actually produces a signature.
    /// Mirrors the production signing precondition so a regression on the
    /// no-password cold-boot path — decrypt or sign — is caught. The
    /// unprotected seed resolves with no interaction.
    #[cfg(test)]
    pub(crate) async fn assert_can_sign(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
        use dash_sdk::dpp::key_wallet::signer::Signer;

        let scope = Self::hd_scope(seed_hash);
        let path: DerivationPath = "m/44'/1'/0'/0/0"
            .parse()
            .expect("static derivation path parses");
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
                // Drive a real sign, not just signer construction: a derive or
                // sign regression must fail here.
                signer
                    .sign_ecdsa(&path, [0x11u8; 32])
                    .await
                    .map_err(|_| TaskError::SingleKeyCryptoFailure)?;
                Ok(())
            })
            .await
    }

    /// Build, sign, and broadcast a payment from the wallet's default BIP-44
    /// account to `recipients` (`(address, duffs)`). Returns the txid.
    ///
    /// One [`SecretAccess::with_secret_session`](super::SecretAccess::with_secret_session) scope wraps the whole build:
    /// the seed is decrypted just-in-time (one prompt for a passphrase-
    /// protected wallet, none for a no-password wallet), borrowed by the
    /// [`DetSigner`] for every input sign, and zeroized when the scope ends.
    pub async fn send_payment(
        &self,
        seed_hash: &WalletSeedHash,
        recipients: Vec<(dash_sdk::dpp::dashcore::Address, u64)>,
    ) -> Result<dash_sdk::dpp::dashcore::Txid, TaskError> {
        use dash_sdk::dpp::key_wallet::account::account_type::StandardAccountType;
        use dash_sdk::dpp::key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::BuilderError;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::TransactionBuilder;
        use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                let wallet_id = wallet.wallet_id();

                // Assemble and sign under one uninterrupted hold of the
                // wallet-manager write lock: `set_funding` reads the funding
                // account's free UTXOs and `build_signed` reserves the ones it
                // selects. Holding the lock across both closes the
                // read-then-reserve window a concurrent build could otherwise use
                // to double-select the same UTXO. The guard drops at the end of
                // this block, before the broadcast re-acquires the lock.
                let tx = {
                    let mut wm = wallet.wallet_manager().write().await;
                    let (kw_wallet, info) = wm
                        .get_wallet_and_info_mut(&wallet_id)
                        .ok_or(TaskError::WalletStateInconsistent)?;

                    let account = kw_wallet
                        .get_bip44_account(DEFAULT_BIP44_ACCOUNT)
                        .ok_or(TaskError::WalletStateInconsistent)?;
                    let current_height = info.core_wallet.synced_height();
                    let managed_account = info
                        .core_wallet
                        .accounts
                        .standard_bip44_accounts
                        .get_mut(&DEFAULT_BIP44_ACCOUNT)
                        .ok_or(TaskError::WalletStateInconsistent)?;

                    let mut builder = TransactionBuilder::new()
                        .set_current_height(current_height)
                        .set_selection_strategy(SelectionStrategy::LargestFirst)
                        .set_funding(managed_account, account);
                    for (address, amount) in &recipients {
                        builder = builder.add_output(address, *amount);
                    }

                    let (tx, _fee) = builder
                        .build_signed(&signer, |addr| {
                            managed_account.address_derivation_path(&addr)
                        })
                        .await
                        .map_err(|source| {
                            // Give balance-specific and input-count-specific
                            // advice only for the failures that are actually
                            // about balance or input count — every other
                            // `BuilderError` variant falls back to the
                            // generic `WalletPaymentBuildFailed` message
                            // rather than misdirecting the user to "check
                            // your balance" for e.g. a signing failure.
                            match source {
                                BuilderError::InsufficientFunds {
                                    available,
                                    required,
                                } => TaskError::InsufficientFunds {
                                    available,
                                    required,
                                },
                                BuilderError::TooManyInputs { count, max } => {
                                    TaskError::WalletPaymentTooManyInputs { count, max }
                                }
                                other => TaskError::WalletPaymentBuildFailed {
                                    source: Box::new(other),
                                },
                            }
                        })?;
                    tx
                };

                // Broadcast through the wallet's own `SpvBroadcaster`, releasing
                // the build's UTXO reservation on a definitive pre-send rejection
                // so an immediate retry can reselect those inputs. Preserves the
                // reservation reconciliation the removed `core().send_to_addresses`
                // performed.
                wallet
                    .core()
                    .broadcast_transaction_releasing_reservation(
                        StandardAccountType::BIP44Account,
                        DEFAULT_BIP44_ACCOUNT,
                        &tx,
                    )
                    .await
                    .map_err(|source| TaskError::WalletBackend {
                        source: Arc::new(source),
                    })?;
                Ok(tx.txid())
            })
            .await
    }

    /// Build, track, and broadcast a **non-identity** asset lock via the
    /// upstream `AssetLockManager`. `funding_type` selects the funding
    /// derivation; `identity_index` is the funding-account derivation index
    /// (ignored for non-identity funding types). Returns the finalized
    /// asset-lock proof, its one-time credit-output private key (derived
    /// locally from the wallet seed at the path upstream selected), and the
    /// txid.
    ///
    /// For identity-funded asset locks
    /// (`AssetLockFundingType::IdentityRegistration` /
    /// `AssetLockFundingType::IdentityTopUp`) the upstream
    /// `IdentityWallet::*_with_funding` orchestrators submit the
    /// Platform-side state transition themselves and never expose a
    /// credit-output `PrivateKey` — use [`Self::register_identity`] /
    /// [`Self::top_up_identity`] instead.
    pub(crate) async fn create_asset_lock_proof(
        &self,
        seed_hash: &WalletSeedHash,
        amount_duffs: u64,
        funding_type: platform_wallet::AssetLockFundingType,
        identity_index: u32,
    ) -> Result<
        (
            dash_sdk::dpp::prelude::AssetLockProof,
            dash_sdk::dpp::dashcore::PrivateKey,
            dash_sdk::dpp::dashcore::Txid,
        ),
        TaskError,
    > {
        use platform_wallet::AssetLockFundingType;

        // One held-seed scope covers account provisioning, the funding-input
        // signer, and the credit-output key derivation, so the whole operation
        // prompts at most once and the seed zeroizes when the scope ends.
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                // Identity asset locks fund from the IdentityRegistration /
                // IdentityTopUp HD accounts, which the upstream persister never
                // reconstructs (a5538dc8). Provision them here — the single
                // chokepoint every asset-lock caller funnels through — so no
                // call site can bypass it. Idempotent. Non-identity funding
                // types are no-ops. Exhaustive — a new upstream variant must
                // force a review here instead of silently falling through.
                // Must run inside the session so the seed is available for
                // hardened xpub derivation (the live wallet is watch-only).
                match funding_type {
                    AssetLockFundingType::IdentityRegistration
                    | AssetLockFundingType::IdentityTopUp => {
                        let plaintext = session.plaintext();
                        let seed = plaintext
                            .expose_hd_seed()
                            .ok_or(TaskError::WalletStateInconsistent)?;
                        self.ensure_identity_funding_accounts(seed_hash, seed, identity_index)
                            .await?;
                    }
                    AssetLockFundingType::IdentityTopUpNotBound
                    | AssetLockFundingType::IdentityInvitation
                    | AssetLockFundingType::AssetLockAddressTopUp
                    | AssetLockFundingType::AssetLockShieldedAddressTopUp => {}
                }
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                let (proof, credit_output_path, out_point) = wallet
                    .asset_locks()
                    .create_funded_asset_lock_proof(
                        amount_duffs,
                        DEFAULT_BIP44_ACCOUNT,
                        funding_type,
                        identity_index,
                        &signer,
                    )
                    .await
                    .map_err(|e| TaskError::WalletBackend {
                        source: Arc::new(e),
                    })?;
                let private_key =
                    self.derive_private_key_from_held(session.plaintext(), &credit_output_path)?;
                Ok((proof, private_key, out_point.txid))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ASSET_LOCK_FEE_PER_KB, asset_lock_builder_height, asset_lock_max_amount_from_account,
    };
    use crate::model::fee_estimation::core_max_send_amount_duffs;
    use crate::wallet_backend::snapshot::DetWalletBalance;
    use dash_sdk::dpp::dashcore::ScriptBuf;
    use dash_sdk::dpp::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
    use dash_sdk::dpp::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, PublicKey, TxOut, Txid};
    use dash_sdk::dpp::key_wallet::Utxo;
    use dash_sdk::dpp::key_wallet::wallet::Wallet;
    use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeRate;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::TransactionBuilder;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

    /// Reproduces <https://github.com/dashpay/dash-evo-tool/issues/909>.
    /// The root cause is upstream in `dashpay/rust-dashcore` key-wallet's
    /// `coin_selection.rs`, pinned at revision `be6e776`, tracked at
    /// <https://github.com/dashpay/rust-dashcore/issues/911>.
    ///
    /// Asserts the *correct* behavior (a Max send folds the zero/dust remainder
    /// into its fee), so it stays RED until the upstream fix lands. `#[ignore]`
    /// keeps it out of the CI gate meanwhile; run it manually with:
    ///
    /// ```sh
    /// cargo test --lib -- core_max_send_with_single_utxo_builds_without_change --ignored
    /// ```
    ///
    /// Remove `#[ignore]` once the pinned key-wallet revision contains the fix.
    #[test]
    #[ignore = "RED until upstream key-wallet coin-selection fix lands (rust-dashcore#911); run with --ignored"]
    fn core_max_send_with_single_utxo_builds_without_change() {
        const BALANCE_DUFFS: u64 = 10_000_000;

        let address = Address::p2pkh(
            &PublicKey::from_slice(&[0x02; 33]).expect("valid compressed public key"),
            Network::Testnet,
        );
        let utxo = Utxo::new(
            OutPoint::new(Txid::all_zeros(), 0),
            TxOut {
                value: BALANCE_DUFFS,
                script_pubkey: address.script_pubkey(),
            },
            address.clone(),
            1,
            false,
        );
        let max_amount = core_max_send_amount_duffs(BALANCE_DUFFS, 1, 1)
            .expect("the balance covers the Max-send fee reserve");

        assert_eq!(max_amount, 9_999_780);
        let (transaction, fee) = TransactionBuilder::new()
            .set_current_height(200)
            .set_selection_strategy(SelectionStrategy::LargestFirst)
            .add_inputs([utxo])
            .add_output(&address, max_amount)
            .set_change_address(address)
            .build_unsigned()
            .expect("a Max send must fold the zero/dust remainder into its fee");

        assert_eq!(transaction.output.len(), 1);
        assert_eq!(fee, BALANCE_DUFFS - max_amount);
    }

    #[test]
    fn asset_lock_max_excludes_unconfirmed_funds_counted_by_snapshot() {
        const CONFIRMED_DUFFS: u64 = 1_000_000;
        const UNCONFIRMED_DUFFS: u64 = 5_000_000;
        const CURRENT_HEIGHT: u32 = 200;

        let wallet = Wallet::new_random(Network::Testnet, WalletAccountCreationOptions::Default)
            .expect("test wallet");
        let mut wallet_info =
            ManagedWalletInfo::from_wallet_with_name(&wallet, "Test".to_string(), 0);
        let account = wallet.get_bip44_account(0).expect("BIP44 account");
        let managed_account = wallet_info
            .accounts
            .standard_bip44_accounts
            .get_mut(&0)
            .expect("managed BIP44 account");
        let funding_address = managed_account
            .next_receive_address(Some(&account.account_xpub), true)
            .expect("funding address");

        for (txid_byte, value, is_confirmed) in [
            (0x11, CONFIRMED_DUFFS, true),
            (0x22, UNCONFIRMED_DUFFS, false),
        ] {
            let outpoint = OutPoint::new(Txid::from_byte_array([txid_byte; 32]), 0);
            let mut utxo = Utxo::new(
                outpoint,
                TxOut {
                    value,
                    script_pubkey: funding_address.script_pubkey(),
                },
                funding_address.clone(),
                100,
                false,
            );
            utxo.is_confirmed = is_confirmed;
            managed_account.utxos.insert(outpoint, utxo);
        }

        let snapshot_balance = DetWalletBalance {
            confirmed: CONFIRMED_DUFFS,
            unconfirmed: UNCONFIRMED_DUFFS,
            total: CONFIRMED_DUFFS + UNCONFIRMED_DUFFS,
        };
        let max_amount =
            asset_lock_max_amount_from_account(managed_account, account, CURRENT_HEIGHT)
                .expect("asset-lock maximum");

        assert!(
            max_amount > 0,
            "the confirmed output must still provide a usable asset-lock Max"
        );
        assert!(
            max_amount < snapshot_balance.spendable(),
            "Max must exclude the unconfirmed output that the asset-lock builder rejects"
        );

        let mut dry_run_account = managed_account.clone();
        let (transaction, _) = TransactionBuilder::new()
            .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
            .set_current_height(CURRENT_HEIGHT)
            .set_special_payload(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload::new(vec![TxOut {
                    value: max_amount,
                    script_pubkey: ScriptBuf::new(),
                }]),
            ))
            .set_funding(&mut dry_run_account, account)
            .require_final_inputs()
            .build_unsigned()
            .expect("quoted Max must build through the real asset-lock selector");
        dry_run_account.release_reservation(&transaction);

        let mut one_over_account = managed_account.clone();
        let one_over = TransactionBuilder::new()
            .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
            .set_current_height(CURRENT_HEIGHT)
            .set_special_payload(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload::new(vec![TxOut {
                    value: max_amount + 1,
                    script_pubkey: ScriptBuf::new(),
                }]),
            ))
            .set_funding(&mut one_over_account, account)
            .require_final_inputs()
            .build_unsigned();
        assert!(
            one_over.is_err(),
            "one duff above the quoted Max must fail through the real selector"
        );

        let mut overshoot_account = managed_account.clone();
        let overshoot = TransactionBuilder::new()
            .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
            .set_current_height(CURRENT_HEIGHT)
            .set_special_payload(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload::new(vec![TxOut {
                    value: snapshot_balance.spendable(),
                    script_pubkey: ScriptBuf::new(),
                }]),
            ))
            .set_funding(&mut overshoot_account, account)
            .require_final_inputs()
            .build_unsigned();
        assert!(
            overshoot.is_err(),
            "the display-only snapshot amount must reproduce the builder rejection"
        );
    }

    #[test]
    fn asset_lock_max_uses_last_processed_height_when_sync_watermarks_diverge() {
        let wallet = Wallet::new_random(Network::Testnet, WalletAccountCreationOptions::Default)
            .expect("test wallet");
        let mut wallet_info =
            ManagedWalletInfo::from_wallet_with_name(&wallet, "Test".to_string(), 0);
        wallet_info.update_last_processed_height(200);
        wallet_info.update_synced_height(300);

        assert_eq!(wallet_info.last_processed_height(), 200);
        assert_eq!(wallet_info.synced_height(), 300);
        assert_eq!(
            asset_lock_builder_height(&wallet_info),
            wallet_info.last_processed_height(),
            "the Max probe must use the same block-processed watermark as the real asset-lock builder"
        );
    }
}
