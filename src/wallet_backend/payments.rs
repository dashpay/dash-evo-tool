//! Payment and asset-lock operations on [`WalletBackend`] — the funds-signing
//! path.
//!
//! Every seed-bearing operation here opens a single just-in-time
//! [`SecretAccess`](super::SecretAccess) session so the HD seed is decrypted
//! once, borrowed by the [`DetSigner`] for signing, and zeroized when the
//! scope ends. `send_payment` builds and broadcasts a BIP-44 payment;
//! `create_asset_lock_proof` builds an asset lock whose credit output DET
//! spends itself and returns its one-time key, and
//! `resume_unbound_topup_asset_lock` does the same for one already broadcast.

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
use std::time::{Duration, Instant};

use super::snapshot::asset_lock_final_input_state;
use super::{
    AssetLockInputState, DEFAULT_BIP44_ACCOUNT, DetSigner, SecretPlaintext, WalletBackend,
};

// `AssetLockManager` passes its private `DEFAULT_FEE_PER_KB` explicitly to key-wallet, so DET
// cannot reuse the real path's source.
// TODO(upstream): export that default or expose an asset-lock ceiling quote primitive.
const ASSET_LOCK_FEE_PER_KB: u64 = 1_000;
/// Budget for the probe's builder work — composition observation plus quote
/// search — measured from when the blocking task starts running, so total
/// exclusive wallet access is bounded by this plus setup overhead.
const ASSET_LOCK_PROBE_LOCK_HOLD_DEADLINE: Duration = Duration::from_secs(5);
const MAX_DRAIN_SEARCH_DOUBLINGS: u32 = 40;
const P2PKH_CREDIT_OUTPUT_SCRIPT_LEN: usize = 25;
const P2PKH_INPUT_SIZE: usize = 148;

enum AssetLockDryRun {
    Builds,
    Rejected { available: u64 },
    TooManyInputs { max: usize },
}

enum AssetLockDrainSeed {
    Ceiling(u64),
    Unavailable,
    TooManyInputs { max: usize },
}

/// Conservative builder quote and the exact input composition it observed.
#[derive(Debug, Clone)]
pub struct AssetLockMaxAmountQuote {
    /// Largest amount proven buildable before the search deadline.
    pub amount_duffs: u64,
    /// Final, unreserved inputs visible to the builder under the wallet lock.
    pub observed_inputs: AssetLockInputState,
    /// Whether the deadline stopped the search before the exact maximum was found.
    pub is_partial: bool,
}

pub(super) struct ProbeDeadline {
    expires_at: Option<Instant>,
    timed_out: std::cell::Cell<bool>,
}

impl ProbeDeadline {
    pub(super) fn after(duration: Duration) -> Self {
        Self {
            expires_at: Instant::now().checked_add(duration),
            timed_out: std::cell::Cell::new(false),
        }
    }

    pub(super) fn unbounded() -> Self {
        Self {
            expires_at: None,
            timed_out: std::cell::Cell::new(false),
        }
    }

    pub(super) fn expired(&self) -> bool {
        let expired = self
            .expires_at
            .is_some_and(|expires_at| Instant::now() >= expires_at);
        if expired {
            self.timed_out.set(true);
        }
        expired
    }

    fn timed_out(&self) -> bool {
        self.timed_out.get()
    }
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
    dry_run_asset_lock_amount_with_strategy(
        managed_account,
        account,
        current_height,
        amount_duffs,
        None,
    )
}

// INTENTIONAL(upstream-bnb-dos): default `BranchAndBound` is algorithmically
// bounded by rust-dashcore#919's suffix-sum feasibility prune and node budget,
// which closes rust-dashcore#918. The aggregate quote still has a deadline
// because it performs many individually bounded selections under a write lock.
// Confirmed present: this crate's `Cargo.toml` pins `platform` to rev
// 993584a6b53831026f2f5fe30f21a8f4d14a06e8, whose `key-wallet` dep pins
// rust-dashcore rev 173ffac0, and that rev's `coin_selection.rs` is
// byte-identical to the one carrying #919. Re-verify whenever either pin
// moves — the key-wallet reference is a rev, so it cannot drift on its own.
fn dry_run_asset_lock_amount_with_strategy(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    amount_duffs: u64,
    selection_strategy: Option<SelectionStrategy>,
) -> Result<AssetLockDryRun, BuilderError> {
    let mut dry_run_account = managed_account.clone();
    let mut builder = TransactionBuilder::new()
        .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
        .set_current_height(current_height)
        .set_special_payload(TransactionPayload::AssetLockPayloadType(
            AssetLockPayload::new(vec![TxOut {
                value: amount_duffs,
                script_pubkey: ScriptBuf::from_bytes(vec![0; P2PKH_CREDIT_OUTPUT_SCRIPT_LEN]),
            }]),
        ))
        .add_funding(&mut dry_run_account, account);
    if let Some(strategy) = selection_strategy {
        builder = builder.set_selection_strategy(strategy);
    }
    let result = builder.require_final_inputs().build_unsigned_reserved();

    match result {
        Ok((transaction, _, reservation)) => {
            // `ManagedCoreFundsAccount::clone` shares the live `ReservationSet`
            // (`Arc<Mutex<_>>`; source of truth: key-wallet's
            // `managed_account/reservation.rs` doc comment). Every successful probe
            // reserves real wallet outpoints, so this call MUST run on every success
            // path; deleting it silently strands real UTXOs for the 24-block TTL.
            // The owner guard frees only what this build reserved, never a
            // concurrent build's re-reservation of the same outpoint.
            if let Some(token) = reservation {
                dry_run_account.release_reservation_if_owner(&transaction, token);
            }
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
        Err(BuilderError::TooManyInputs { max, .. }) => Ok(AssetLockDryRun::TooManyInputs { max }),
        Err(source) => Err(source),
    }
}

fn asset_lock_drain_ceiling(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    deadline: &ProbeDeadline,
) -> Result<AssetLockDrainSeed, BuilderError> {
    if deadline.expired() {
        return Ok(AssetLockDrainSeed::Unavailable);
    }
    let mut dry_run_account = managed_account.clone();
    let result = TransactionBuilder::new()
        .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
        .set_current_height(current_height)
        .set_selection_strategy(SelectionStrategy::All)
        .set_special_payload(TransactionPayload::AssetLockPayloadType(
            AssetLockPayload::new(vec![TxOut {
                value: 1,
                // Credit-output script length affects the serialized payload fee.
                script_pubkey: ScriptBuf::from_bytes(vec![0; P2PKH_CREDIT_OUTPUT_SCRIPT_LEN]),
            }]),
        ))
        .add_funding(&mut dry_run_account, account)
        .require_final_inputs()
        .build_unsigned_reserved();

    match result {
        Ok((transaction, _, reservation)) => {
            // `ManagedCoreFundsAccount::clone` shares the live `ReservationSet`
            // (`Arc<Mutex<_>>`; source of truth: key-wallet's
            // `managed_account/reservation.rs` doc comment). Every successful probe
            // reserves real wallet outpoints, so this call MUST run on every success
            // path; deleting it silently strands real UTXOs for the 24-block TTL.
            // The owner guard frees only what this build reserved, never a
            // concurrent build's re-reservation of the same outpoint.
            if let Some(token) = reservation {
                dry_run_account.release_reservation_if_owner(&transaction, token);
            }
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
        Err(BuilderError::TooManyInputs { max, .. }) => {
            Ok(AssetLockDrainSeed::TooManyInputs { max })
        }
        Err(source) => Err(source),
    }
}

fn asset_lock_input_cap_upper_bound(
    managed_account: &ManagedCoreFundsAccount,
    current_height: u32,
    max_inputs: usize,
) -> u64 {
    let mut values: Vec<u64> = managed_account
        .utxos
        .values()
        .filter(|utxo| {
            (utxo.is_confirmed || utxo.is_instantlocked) && utxo.is_spendable(current_height)
        })
        .map(|utxo| utxo.value())
        .collect();
    values.sort_unstable_by(|left, right| right.cmp(left));
    values
        .into_iter()
        .take(max_inputs)
        .fold(0_u64, u64::saturating_add)
}

fn largest_first_asset_lock_max_below(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    upper_bound: u64,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    let mut low = 0;
    let mut high = upper_bound;
    while low < high {
        if deadline.expired() {
            return Ok(low);
        }
        let candidate = low + (high - low).div_ceil(2);
        match dry_run_asset_lock_amount_with_strategy(
            managed_account,
            account,
            current_height,
            candidate,
            Some(SelectionStrategy::LargestFirst),
        )? {
            AssetLockDryRun::Builds => low = candidate,
            AssetLockDryRun::Rejected { .. } | AssetLockDryRun::TooManyInputs { .. } => {
                high = candidate - 1
            }
        }
    }
    Ok(low)
}

fn default_strategy_max_from_seed(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    seed: u64,
    upper_bound: u64,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    if deadline.expired() {
        return Ok(0);
    }
    if !matches!(
        dry_run_asset_lock_amount(managed_account, account, current_height, seed)?,
        AssetLockDryRun::Builds
    ) {
        return asset_lock_max_below_upper_bound(
            managed_account,
            account,
            current_height,
            seed,
            deadline,
        );
    }

    let mut low = seed;
    let mut step = 1_u64;
    while low < upper_bound {
        if deadline.expired() {
            return Ok(low);
        }
        let candidate = seed.saturating_add(step).min(upper_bound);
        match dry_run_asset_lock_amount(managed_account, account, current_height, candidate)? {
            AssetLockDryRun::Builds if candidate == upper_bound => return Ok(candidate),
            AssetLockDryRun::Builds => {
                low = candidate;
                step = step.saturating_mul(2);
            }
            AssetLockDryRun::Rejected { .. } | AssetLockDryRun::TooManyInputs { .. } => {
                let mut high = candidate - 1;
                while low < high {
                    if deadline.expired() {
                        return Ok(low);
                    }
                    let midpoint = low + (high - low).div_ceil(2);
                    match dry_run_asset_lock_amount(
                        managed_account,
                        account,
                        current_height,
                        midpoint,
                    )? {
                        AssetLockDryRun::Builds => low = midpoint,
                        AssetLockDryRun::Rejected { .. }
                        | AssetLockDryRun::TooManyInputs { .. } => high = midpoint - 1,
                    }
                }
                return Ok(low);
            }
        }
    }
    Ok(low)
}

fn asset_lock_max_with_input_cap(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    max_inputs: usize,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    if deadline.expired() {
        return Ok(0);
    }
    let upper_bound = asset_lock_input_cap_upper_bound(managed_account, current_height, max_inputs);
    let largest_first_seed = largest_first_asset_lock_max_below(
        managed_account,
        account,
        current_height,
        upper_bound,
        deadline,
    )?;
    // Branch-and-bound's exact-match sizing looks one P2PKH input ahead; this
    // is only a starting point, and default-strategy probes remain authoritative.
    let seed = largest_first_seed
        .saturating_sub(FeeRate::new(ASSET_LOCK_FEE_PER_KB).calculate_fee(P2PKH_INPUT_SIZE));
    default_strategy_max_from_seed(
        managed_account,
        account,
        current_height,
        seed,
        upper_bound,
        deadline,
    )
}

fn asset_lock_max_below_upper_bound(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    upper_bound: u64,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    let mut step = 1_u64;
    let mut high = upper_bound;
    for _ in 0..MAX_DRAIN_SEARCH_DOUBLINGS {
        if deadline.expired() {
            return Ok(0);
        }
        let candidate = upper_bound.saturating_sub(step);
        match dry_run_asset_lock_amount(managed_account, account, current_height, candidate)? {
            AssetLockDryRun::Builds => {
                let mut low = candidate;
                while low < high {
                    if deadline.expired() {
                        return Ok(low);
                    }
                    let midpoint = low + (high - low).div_ceil(2);
                    match dry_run_asset_lock_amount(
                        managed_account,
                        account,
                        current_height,
                        midpoint,
                    )? {
                        AssetLockDryRun::Builds => low = midpoint,
                        AssetLockDryRun::Rejected { .. }
                        | AssetLockDryRun::TooManyInputs { .. } => high = midpoint - 1,
                    }
                }
                return Ok(low);
            }
            AssetLockDryRun::Rejected { .. } | AssetLockDryRun::TooManyInputs { .. }
                if candidate == 0 =>
            {
                return Ok(0);
            }
            AssetLockDryRun::Rejected { .. } | AssetLockDryRun::TooManyInputs { .. } => {
                high = candidate - 1;
                step = step.saturating_mul(2);
            }
        }
    }

    full_range_asset_lock_max_amount(managed_account, account, current_height, None, deadline)
}

fn full_range_asset_lock_max_amount(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    input_cap: Option<usize>,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    if let Some(max_inputs) = input_cap {
        return asset_lock_max_with_input_cap(
            managed_account,
            account,
            current_height,
            max_inputs,
            deadline,
        );
    }

    if deadline.expired() {
        return Ok(0);
    }

    let mut high = match dry_run_asset_lock_amount(
        managed_account,
        account,
        current_height,
        MAX_MONEY.saturating_add(1),
    )? {
        AssetLockDryRun::Rejected { available } => available,
        AssetLockDryRun::Builds => MAX_MONEY,
        AssetLockDryRun::TooManyInputs { max } => {
            return asset_lock_max_with_input_cap(
                managed_account,
                account,
                current_height,
                max,
                deadline,
            );
        }
    };
    let mut low = 0;

    while low < high {
        if deadline.expired() {
            return Ok(low);
        }
        let candidate = low + (high - low).div_ceil(2);
        match dry_run_asset_lock_amount(managed_account, account, current_height, candidate)? {
            AssetLockDryRun::Builds => low = candidate,
            AssetLockDryRun::Rejected { .. } | AssetLockDryRun::TooManyInputs { .. } => {
                high = candidate - 1
            }
        }
    }

    Ok(low)
}

/// Return the largest credit-output amount the real asset-lock builder accepts.
///
/// A drain build supplies a tight upper bound, then default-strategy probes find
/// the exact boundary without reproducing the selector's internal fee model.
fn asset_lock_max_amount_from_account_until(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    let drain_ceiling =
        match asset_lock_drain_ceiling(managed_account, account, current_height, deadline)? {
            AssetLockDrainSeed::Ceiling(ceiling) => ceiling,
            AssetLockDrainSeed::Unavailable => return Ok(0),
            AssetLockDrainSeed::TooManyInputs { max } => {
                return full_range_asset_lock_max_amount(
                    managed_account,
                    account,
                    current_height,
                    Some(max),
                    deadline,
                );
            }
        };
    asset_lock_max_below_upper_bound(
        managed_account,
        account,
        current_height,
        drain_ceiling,
        deadline,
    )
}

#[cfg(test)]
fn asset_lock_max_amount_from_account(
    managed_account: &ManagedCoreFundsAccount,
    account: &Account,
    current_height: u32,
) -> Result<u64, BuilderError> {
    asset_lock_max_amount_from_account_until(
        managed_account,
        account,
        current_height,
        &ProbeDeadline::unbounded(),
    )
}

impl WalletBackend {
    /// Query the largest asset-lock credit output accepted by the live wallet.
    ///
    /// This is the non-broadcasting counterpart of [`Self::create_asset_lock_proof`].
    /// It runs off the UI thread against the upstream wallet manager and never
    /// exposes or reconstructs UTXO selection in the UI.
    pub async fn asset_lock_max_amount(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<AssetLockMaxAmountQuote, TaskError> {
        self.asset_lock_max_amount_with_timeout(seed_hash, ASSET_LOCK_PROBE_LOCK_HOLD_DEADLINE)
            .await
    }

    async fn asset_lock_max_amount_with_timeout(
        &self,
        seed_hash: &WalletSeedHash,
        timeout: Duration,
    ) -> Result<AssetLockMaxAmountQuote, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        // Every dry run performs one reservation read→reserve→release cycle.
        // Keep them all under the same exclusive boundary as real builds.
        let wallet_manager = Arc::clone(wallet.wallet_manager()).write_owned().await;
        let (managed_account, account, current_height) = {
            let (key_wallet, info) = wallet_manager
                .get_wallet_and_info(&wallet_id)
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
                .ok_or(TaskError::WalletStateInconsistent)?
                .clone();
            (managed_account, account, current_height)
        };

        tokio::task::spawn_blocking(move || {
            // Started here — not on the async side — so blocking-pool queueing
            // delay is not charged against the builder-work budget.
            let deadline = ProbeDeadline::after(timeout);
            let Some(observed_inputs) = asset_lock_final_input_state(
                &managed_account,
                &account,
                current_height,
                &deadline,
            )?
            else {
                // The budget ran out before the composition was fully
                // observed: return the provably-safe empty quote instead of
                // continuing to hold the wallet-manager lock. The default
                // (empty) key only matches a genuinely-empty composition, so
                // validation stays fail-closed.
                drop(wallet_manager);
                return Ok(AssetLockMaxAmountQuote {
                    amount_duffs: 0,
                    observed_inputs: AssetLockInputState::default(),
                    is_partial: true,
                });
            };
            let amount_duffs = asset_lock_max_amount_from_account_until(
                &managed_account,
                &account,
                current_height,
                &deadline,
            )?;
            let is_partial = deadline.timed_out();
            drop(wallet_manager);
            Ok(AssetLockMaxAmountQuote {
                amount_duffs,
                observed_inputs,
                is_partial,
            })
        })
        .await?
        .map_err(|source| TaskError::AssetLockBalanceQueryFailed {
            source: Box::new(source),
        })
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
                // wallet-manager write lock: `add_funding` reads the funding
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
                        .add_funding(managed_account, account);
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

    /// Build, track, and broadcast an asset lock whose credit output DET itself
    /// spends, via the upstream `AssetLockManager`. `funding_type` selects the
    /// funding derivation; `identity_index` is the funding-account derivation
    /// index (ignored by funding types that have no per-identity account).
    /// Returns the finalized asset-lock proof, its one-time credit-output
    /// private key (derived locally from the wallet seed at the path upstream
    /// selected), and the txid.
    ///
    /// For the two index-bound identity funding types
    /// (`AssetLockFundingType::IdentityRegistration` /
    /// `AssetLockFundingType::IdentityTopUp`) the upstream
    /// `IdentityWallet::*_with_funding` orchestrators submit the
    /// Platform-side state transition themselves and never expose a
    /// credit-output `PrivateKey` — use [`Self::register_identity`] /
    /// [`Self::top_up_identity`] instead. `IdentityTopUpNotBound` has no such
    /// orchestrator: it funds a top-up of an identity outside this wallet,
    /// which the caller submits through the SDK itself.
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
                    AssetLockFundingType::IdentityTopUpNotBound => {
                        let plaintext = session.plaintext();
                        let seed = plaintext
                            .expose_hd_seed()
                            .ok_or(TaskError::WalletStateInconsistent)?;
                        self.ensure_unbound_topup_funding_account(seed_hash, seed)
                            .await?;
                    }
                    AssetLockFundingType::IdentityInvitation
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

    /// Resume a tracked **index-less** top-up asset lock: its finalized proof
    /// plus the one-time credit-output key that signs the transition consuming
    /// it. The counterpart of [`Self::create_asset_lock_proof`] for a lock this
    /// wallet already broadcast.
    ///
    /// Only an index-less lock is eligible — see
    /// [`unbound_topup_lock_eligible`] for why the other kinds are refused.
    ///
    /// # Errors
    /// [`TaskError::AssetLockAlreadyUsed`] when the lock was already spent,
    /// [`TaskError::AssetLockNotEligibleForTopUp`] when it belongs to another
    /// role or this wallet does not track it, and the generic
    /// [`TaskError::WalletBackend`] envelope when upstream cannot resume it.
    pub(crate) async fn resume_unbound_topup_asset_lock(
        &self,
        seed_hash: &WalletSeedHash,
        out_point: dash_sdk::dpp::dashcore::OutPoint,
    ) -> Result<
        (
            dash_sdk::dpp::prelude::AssetLockProof,
            dash_sdk::dpp::dashcore::PrivateKey,
        ),
        TaskError,
    > {
        let tracked = self
            .list_tracked_asset_locks(seed_hash)
            .await?
            .into_iter()
            .find(|lock| lock.out_point == out_point);
        unbound_topup_lock_eligible(
            tracked
                .as_ref()
                .map(|lock| (lock.funding_type, &lock.status)),
        )?;

        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let wallet = self.resolve_wallet(seed_hash).await?;
                let (proof, credit_output_path) = wallet
                    .asset_locks()
                    .resume_asset_lock(&out_point, None)
                    .await
                    .map_err(|e| TaskError::WalletBackend {
                        source: Arc::new(e),
                    })?;
                let private_key =
                    self.derive_private_key_from_held(session.plaintext(), &credit_output_path)?;
                Ok((proof, private_key))
            })
            .await
    }
}

/// Whether a tracked lock may fund a top-up of an identity this wallet does not
/// own, given its `(funding_type, status)` — or `None` when this wallet tracks
/// no such lock.
///
/// Only the index-less kind qualifies. A lock built for a registration index is
/// reserved for this wallet's own identity at that index, and an invitation
/// voucher's credit key was handed to the invitee, so spending either here
/// would take funds earmarked elsewhere. Mirrors the role check the upstream
/// orchestrator applies to its own resume path. Pure — no I/O — so it is
/// unit-testable.
fn unbound_topup_lock_eligible(
    lock: Option<(
        platform_wallet::AssetLockFundingType,
        &platform_wallet::wallet::asset_lock::tracked::AssetLockStatus,
    )>,
) -> Result<(), TaskError> {
    use platform_wallet::AssetLockFundingType;
    use platform_wallet::wallet::asset_lock::tracked::AssetLockStatus;

    match lock {
        Some((_, AssetLockStatus::Consumed)) => Err(TaskError::AssetLockAlreadyUsed),
        Some((AssetLockFundingType::IdentityTopUpNotBound, _)) => Ok(()),
        Some(_) | None => Err(TaskError::AssetLockNotEligibleForTopUp),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ASSET_LOCK_FEE_PER_KB, MAX_MONEY, asset_lock_builder_height,
        asset_lock_max_amount_from_account, unbound_topup_lock_eligible,
    };
    use crate::backend_task::error::TaskError;
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
        let (transaction, fee, _no_reservation) = TransactionBuilder::new()
            .set_current_height(200)
            .set_selection_strategy(SelectionStrategy::LargestFirst)
            .add_inputs([utxo])
            .add_output(&address, max_amount)
            .set_change_address(address)
            .build_unsigned_reserved()
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
        let (transaction, _, reservation) = TransactionBuilder::new()
            .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
            .set_current_height(CURRENT_HEIGHT)
            .set_special_payload(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload::new(vec![TxOut {
                    value: max_amount,
                    script_pubkey: ScriptBuf::new(),
                }]),
            ))
            .add_funding(&mut dry_run_account, account)
            .require_final_inputs()
            .build_unsigned_reserved()
            .expect("quoted Max must build through the real asset-lock selector");
        if let Some(token) = reservation {
            dry_run_account.release_reservation_if_owner(&transaction, token);
        }

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
            .add_funding(&mut one_over_account, account)
            .require_final_inputs()
            .build_unsigned_reserved();
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
            .add_funding(&mut overshoot_account, account)
            .require_final_inputs()
            .build_unsigned_reserved();
        assert!(
            overshoot.is_err(),
            "the display-only snapshot amount must reproduce the builder rejection"
        );
    }

    #[test]
    fn asset_lock_max_uses_an_in_cap_subset_when_the_wallet_has_too_many_utxos() {
        const CURRENT_HEIGHT: u32 = 200;
        const INPUT_CAP: usize = 500;
        const LARGE_UTXO_COUNT: usize = INPUT_CAP - 1;
        const SMALL_UTXO_COUNT: usize = 17;
        const UTXO_COUNT: usize = LARGE_UTXO_COUNT + SMALL_UTXO_COUNT;
        const ASSET_LOCK_BASE_SIZE: usize = 115;

        // Match upstream exact-match sizing so the MAX+1 seed deterministically
        // selects every UTXO and returns `TooManyInputs`.
        let seed_probe_fee = FeeRate::new(ASSET_LOCK_FEE_PER_KB)
            .calculate_fee(ASSET_LOCK_BASE_SIZE + 148 * (UTXO_COUNT + 1));
        let total_value = MAX_MONEY + 1 + seed_probe_fee;
        let small_values: Vec<u64> = (0..SMALL_UTXO_COUNT)
            .rev()
            .map(|power| 1_u64 << power)
            .collect();
        let large_total = total_value - small_values.iter().sum::<u64>();
        let base_utxo_duffs = large_total / LARGE_UTXO_COUNT as u64;
        let larger_utxos = large_total % LARGE_UTXO_COUNT as u64;

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

        let values = (0..LARGE_UTXO_COUNT)
            .map(|index| base_utxo_duffs + u64::from((index as u64) < larger_utxos))
            .chain(small_values);
        for (index, value) in values.enumerate() {
            let txid = Txid::from_byte_array([index as u8; 32]);
            let outpoint = OutPoint::new(txid, index as u32);
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
            utxo.is_confirmed = true;
            managed_account.utxos.insert(outpoint, utxo);
        }

        let max_amount =
            asset_lock_max_amount_from_account(managed_account, account, CURRENT_HEIGHT)
                .expect("asset-lock maximum");
        assert!(
            max_amount > base_utxo_duffs,
            "Max must use a real in-cap subset instead of collapsing to zero"
        );

        let mut dry_run_account = managed_account.clone();
        let (transaction, _, reservation) = TransactionBuilder::new()
            .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
            .set_current_height(CURRENT_HEIGHT)
            .set_special_payload(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload::new(vec![TxOut {
                    value: max_amount,
                    script_pubkey: ScriptBuf::new(),
                }]),
            ))
            .add_funding(&mut dry_run_account, account)
            .require_final_inputs()
            .build_unsigned_reserved()
            .expect("quoted Max must build from an in-cap subset");
        assert!(
            transaction.input.len() <= INPUT_CAP,
            "the achievable quote must respect the builder's input cap"
        );
        if let Some(token) = reservation {
            dry_run_account.release_reservation_if_owner(&transaction, token);
        }

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
            .add_funding(&mut one_over_account, account)
            .require_final_inputs()
            .build_unsigned_reserved();
        assert!(
            one_over.is_err(),
            "one duff above the quote must exceed the achievable in-cap subset"
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asset_lock_max_probe_waits_for_wallet_manager_write_lock() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;
        use std::sync::Arc;
        use std::time::Duration;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        app_context
            .ensure_wallet_backend(SenderAsync::new(sender, app_context.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = app_context.wallet_backend().expect("wired backend");

        let seed = [0x41; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("Probe lock test".to_string()),
            None,
        )
        .expect("DET wallet");
        let seed_hash = wallet.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register wallet");

        let platform_wallet = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("platform wallet");
        let read_guard = platform_wallet.wallet_manager().read().await;
        let probe_backend = Arc::clone(&backend);
        let mut probe =
            tokio::spawn(async move { probe_backend.asset_lock_max_amount(&seed_hash).await });

        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut probe)
                .await
                .is_err(),
            "the probe must wait for exclusive wallet-manager access"
        );

        drop(read_guard);
        let amount = tokio::time::timeout(Duration::from_secs(5), probe)
            .await
            .expect("probe completes after read guard drops")
            .expect("probe task")
            .expect("asset-lock maximum");
        assert_eq!(
            amount.amount_duffs, 0,
            "an empty wallet has no asset-lock maximum"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asset_lock_max_probe_deadline_bounds_the_wallet_manager_write_lock() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;
        use std::time::Duration;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        app_context
            .ensure_wallet_backend(SenderAsync::new(sender, app_context.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = app_context.wallet_backend().expect("wired backend");

        let seed = [0x42; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("Probe timeout test".to_string()),
            None,
        )
        .expect("DET wallet");
        let seed_hash = wallet.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register wallet");

        let quote = backend
            .asset_lock_max_amount_with_timeout(&seed_hash, Duration::ZERO)
            .await
            .expect("deadline returns a conservative quote");
        assert!(
            quote.is_partial,
            "an expired search must be tagged as partial"
        );

        let platform_wallet = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("platform wallet");
        let _write_guard = tokio::time::timeout(
            Duration::from_secs(1),
            platform_wallet.wallet_manager().write(),
        )
        .await
        .expect("the expired probe must release its exclusive lock");
    }

    /// Companion to the search-phase deadline test above: the UTXO-composition
    /// observation that runs before the search must obey the same budget, so a
    /// wallet with many eligible UTXOs cannot hold the process-global
    /// wallet-manager write lock through unbounded builder batches.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asset_lock_probe_deadline_bounds_the_observation_phase() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::wallet_backend::AssetLockInputState;
        use std::time::Duration;

        // Two observation batches' worth of eligible UTXOs.
        const UTXO_COUNT: u32 = 501;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        app_context
            .ensure_wallet_backend(SenderAsync::new(sender, app_context.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = app_context.wallet_backend().expect("wired backend");

        let seed = [0x44; 64];
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("Observation deadline test".to_string()),
            None,
        )
        .expect("DET wallet");
        let seed_hash = wallet.seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register wallet");
        let platform_wallet = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("platform wallet");
        let wallet_id = platform_wallet.wallet_id();

        {
            let mut manager = platform_wallet.wallet_manager().write().await;
            let account = manager
                .get_wallet_and_info(&wallet_id)
                .and_then(|(key_wallet, _)| key_wallet.get_bip44_account(0))
                .expect("BIP44 account")
                .clone();
            let info = manager
                .get_wallet_info_mut(&wallet_id)
                .expect("wallet info");
            info.core_wallet.update_last_processed_height(200);
            let managed_account = info
                .core_wallet
                .accounts
                .standard_bip44_accounts
                .get_mut(&0)
                .expect("managed account");
            let funding_address = managed_account
                .next_receive_address(Some(&account.account_xpub), true)
                .expect("funding address");
            for index in 0..UTXO_COUNT {
                let outpoint = OutPoint::new(Txid::from_byte_array([index as u8; 32]), index);
                let mut utxo = Utxo::new(
                    outpoint,
                    TxOut {
                        value: 100_000,
                        script_pubkey: funding_address.script_pubkey(),
                    },
                    funding_address.clone(),
                    100,
                    false,
                );
                utxo.is_confirmed = true;
                managed_account.utxos.insert(outpoint, utxo);
            }
        }

        let quote = backend
            .asset_lock_max_amount_with_timeout(&seed_hash, Duration::ZERO)
            .await
            .expect("an exhausted budget still returns a conservative quote");
        assert!(
            quote.is_partial,
            "an observation stopped by the deadline must be tagged as partial"
        );
        assert_eq!(
            quote.amount_duffs, 0,
            "no amount was proven buildable within the budget"
        );
        assert_eq!(
            quote.observed_inputs,
            AssetLockInputState::default(),
            "an expired budget must abort the observation instead of running builder batches"
        );

        let _write_guard = tokio::time::timeout(
            Duration::from_secs(1),
            platform_wallet.wallet_manager().write(),
        )
        .await
        .expect("the expired observation must release its exclusive lock");
    }

    /// Only an index-less lock may fund an identity outside this wallet: an
    /// index-bound lock is reserved for this wallet's own identity at that
    /// index, an invitation voucher's key is already in the invitee's hands,
    /// and an untracked outpoint is not this wallet's to spend.
    #[test]
    fn only_an_index_less_lock_can_fund_a_foreign_identity_top_up() {
        use platform_wallet::AssetLockFundingType as F;
        use platform_wallet::wallet::asset_lock::tracked::AssetLockStatus;

        unbound_topup_lock_eligible(Some((
            F::IdentityTopUpNotBound,
            &AssetLockStatus::InstantSendLocked,
        )))
        .expect("an index-less lock funds a foreign top-up");

        for funding_type in [
            F::IdentityTopUp,
            F::IdentityRegistration,
            F::IdentityInvitation,
            F::AssetLockAddressTopUp,
        ] {
            let err = unbound_topup_lock_eligible(Some((
                funding_type,
                &AssetLockStatus::InstantSendLocked,
            )))
            .expect_err("a lock bound to another role must be refused");
            assert!(
                matches!(err, TaskError::AssetLockNotEligibleForTopUp),
                "expected AssetLockNotEligibleForTopUp for {funding_type:?}, got: {err:?}"
            );
        }

        let err = unbound_topup_lock_eligible(None)
            .expect_err("an outpoint this wallet does not track must be refused");
        assert!(
            matches!(err, TaskError::AssetLockNotEligibleForTopUp),
            "expected AssetLockNotEligibleForTopUp, got: {err:?}"
        );
    }

    /// A spent lock is refused with its own message: retrying it would only
    /// earn Platform's "already consumed" rejection.
    #[test]
    fn an_already_spent_lock_is_refused_before_submission() {
        use platform_wallet::AssetLockFundingType as F;
        use platform_wallet::wallet::asset_lock::tracked::AssetLockStatus;

        let err = unbound_topup_lock_eligible(Some((
            F::IdentityTopUpNotBound,
            &AssetLockStatus::Consumed,
        )))
        .expect_err("a consumed lock must be refused");
        assert!(
            matches!(err, TaskError::AssetLockAlreadyUsed),
            "expected AssetLockAlreadyUsed, got: {err:?}"
        );
    }
}
