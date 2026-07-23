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
use std::sync::Arc;

use super::{DEFAULT_BIP44_ACCOUNT, DetSigner, SecretPlaintext, WalletBackend};

impl WalletBackend {
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
    use crate::model::fee_estimation::core_max_send_amount_duffs;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, PublicKey, TxOut, Txid};
    use dash_sdk::dpp::key_wallet::Utxo;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::SelectionStrategy;
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::TransactionBuilder;

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
}
