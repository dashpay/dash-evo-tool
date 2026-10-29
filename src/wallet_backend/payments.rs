//! Payment and asset-lock operations on [`WalletBackend`] — the funds-signing
//! path.
//!
//! Every seed-bearing operation here opens a single just-in-time
//! [`SecretAccess`](super::SecretAccess) session so the HD seed is decrypted
//! once, borrowed by the [`DetSigner`] for signing, and zeroized when the
//! scope ends. `send_payment` builds and broadcasts a payment from the pooled
//! accounts; `create_asset_lock_proof` builds an asset lock whose credit
//! output DET spends itself and returns its one-time key, and
//! `resume_unbound_topup_asset_lock` does the same for one already broadcast.

use crate::backend_task::error::TaskError;
use crate::model::spend_limit::{CoreDestinationScript, CoreSendShortfall, SpendLimitKind};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::blockdata::constants::MAX_MONEY;
use dash_sdk::dpp::dashcore::blockdata::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::blockdata::transaction::special_transaction::asset_lock::AssetLockPayload;
use dash_sdk::dpp::dashcore::{ScriptBuf, TxOut, Txid};
use dash_sdk::dpp::key_wallet::Utxo;
use dash_sdk::dpp::key_wallet::account::{Account, AccountType};
use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreFundsAccount;
use dash_sdk::dpp::key_wallet::managed_account::managed_account_collection::ManagedAccountCollection;
use dash_sdk::dpp::key_wallet::wallet::Wallet;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::coin_selection::{
    SelectionError, SelectionStrategy,
};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeRate;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_builder::{
    BuilderError, TransactionBuilder,
};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::transaction_building::AccountTypePreference;
use dash_sdk::dpp::version::PlatformVersion;
use std::collections::HashSet;
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
/// How long the Core-send Max search may hold the wallet-manager write lock
/// when reserved inputs make upstream's quote unbuildable.
const CORE_SEND_MAX_PROBE_LOCK_HOLD_DEADLINE: Duration = Duration::from_secs(5);
const MAX_DRAIN_SEARCH_DOUBLINGS: u32 = 40;
const P2PKH_CREDIT_OUTPUT_SCRIPT_LEN: usize = 25;
const P2PKH_INPUT_SIZE: usize = 148;

enum AssetLockDryRun {
    Builds,
    Rejected { available: u64, required: u64 },
    TooManyInputs { count: usize, max: usize },
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

fn asset_lock_builder_height(info: &ManagedWalletInfo) -> u32 {
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

    info.last_processed_height()
}

/// The most inputs one asset lock may carry under `platform_version`.
///
/// Dash Core does not relay a larger asset lock, and Platform validates the
/// same bound. It is tighter than the builder's standard-transaction limit.
pub(super) fn asset_lock_input_cap(platform_version: &PlatformVersion) -> usize {
    usize::from(
        platform_version
            .dpp
            .state_transitions
            .identities
            .asset_locks
            .max_asset_lock_transaction_inputs,
    )
}

/// `account_types` without repeats, each kept at its first position.
fn first_occurrences(account_types: impl IntoIterator<Item = AccountType>) -> Vec<AccountType> {
    let mut seen = HashSet::new();
    account_types
        .into_iter()
        .filter(|account_type| seen.insert(*account_type))
        .collect()
}

/// The funding accounts `sources` name at `account_index`, in funding order.
/// An account present on only one side is skipped, as a pooled upstream build
/// skips it.
fn pooled_funding_account_types(
    wallet: &Wallet,
    accounts: &ManagedAccountCollection,
    sources: &[AccountTypePreference],
    account_index: u32,
) -> Vec<AccountType> {
    first_occurrences(sources.iter().flat_map(|preference| -> Vec<AccountType> {
        match preference {
            AccountTypePreference::AllDashpayReceivingFunds => accounts
                .dashpay_receival_accounts
                .keys()
                .map(|key| AccountType::DashpayReceivingFunds {
                    index: key.index,
                    user_identity_id: key.user_identity_id,
                    friend_identity_id: key.friend_identity_id,
                })
                .collect(),
            single => single.account_type(account_index).into_iter().collect(),
        }
    }))
    .into_iter()
    .filter(|account_type| {
        wallet.accounts.account_of_type(*account_type).is_some()
            && accounts.funds_account(account_type).is_some()
    })
    .collect()
}

/// [`pooled_funding_account_types`], each account cloned with its keys.
fn pooled_funding_accounts(
    wallet: &Wallet,
    accounts: &ManagedAccountCollection,
    sources: &[AccountTypePreference],
    account_index: u32,
) -> Vec<(ManagedCoreFundsAccount, Account)> {
    pooled_funding_account_types(wallet, accounts, sources, account_index)
        .into_iter()
        .filter_map(|account_type| {
            let account = wallet.accounts.account_of_type(account_type)?;
            let managed = accounts.funds_account(&account_type)?;
            Some((managed.clone(), account.clone()))
        })
        .collect()
}

/// Every coin held by the accounts a pooled Core send funds from
/// ([`platform_wallet::SEND_FUNDING_SOURCES`] at the default account), read in
/// place. Coins of other accounts, CoinJoin among them, never fund a send.
pub(super) fn pooled_send_utxos<'a>(
    wallet: &Wallet,
    info: &'a ManagedWalletInfo,
) -> impl Iterator<Item = &'a Utxo> {
    pooled_funding_account_types(
        wallet,
        &info.accounts,
        &platform_wallet::SEND_FUNDING_SOURCES,
        DEFAULT_BIP44_ACCOUNT,
    )
    .into_iter()
    .filter_map(|account_type| info.accounts.funds_account(&account_type))
    .flat_map(|managed| managed.utxos.values())
}

/// How many coins a pooled Core send of the whole balance spends, counted as
/// platform-wallet's `pooled_max_sendable` counts them: spendable at the
/// wallet's height, worth more than their own input's fee, and no more than
/// one standard transaction carries.
pub(super) fn pooled_send_input_count(wallet: &Wallet, info: &ManagedWalletInfo) -> usize {
    use crate::model::fee_estimation::{CORE_STANDARD_TX_MAX_INPUTS, core_send_input_fee_duffs};
    use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;
    let height = info.last_processed_height();
    let input_fee = core_send_input_fee_duffs();
    pooled_send_utxos(wallet, info)
        .filter(|utxo| utxo.is_spendable(height) && utxo.value() > input_fee)
        .count()
        .min(CORE_STANDARD_TX_MAX_INPUTS)
}

/// What a non-broadcasting asset-lock dry run prices against: the funding
/// accounts the real build pools, the height it builds at, and the protocol's
/// asset-lock input cap.
///
/// The pool mirrors [`platform_wallet::ASSET_LOCK_FUNDING_SOURCES`] at the
/// default account index — the set every exact-amount asset lock DET builds
/// draws from. The cap applies on top of the builder's own: the upstream
/// exact-amount builder accepts up to the standard-transaction limit, so an
/// amount only a larger lock could fund is treated as unbuildable here.
// TODO(dashpay/platform#5513): the exact-amount asset-lock builder should
// refuse more inputs than `max_asset_lock_transaction_inputs` itself, as its
// drain does (DET follow-up draft: asset-lock-exact-input-cap).
pub(super) struct AssetLockFundingPool {
    accounts: Vec<(ManagedCoreFundsAccount, Account)>,
    current_height: u32,
    max_inputs: usize,
}

impl AssetLockFundingPool {
    pub(super) fn from_wallet(
        wallet: &Wallet,
        info: &ManagedWalletInfo,
        max_inputs: usize,
    ) -> Self {
        Self {
            accounts: pooled_funding_accounts(
                wallet,
                &info.accounts,
                &platform_wallet::ASSET_LOCK_FUNDING_SOURCES,
                DEFAULT_BIP44_ACCOUNT,
            ),
            current_height: asset_lock_builder_height(info),
            max_inputs,
        }
    }

    /// A pool of one account, for tests that fund a single account by hand.
    #[cfg(test)]
    pub(super) fn for_account(
        managed_account: &ManagedCoreFundsAccount,
        account: &Account,
        current_height: u32,
        max_inputs: usize,
    ) -> Self {
        Self {
            accounts: vec![(managed_account.clone(), account.clone())],
            current_height,
            max_inputs,
        }
    }

    pub(super) fn accounts(&self) -> &[(ManagedCoreFundsAccount, Account)] {
        &self.accounts
    }

    pub(super) fn current_height(&self) -> u32 {
        self.current_height
    }

    /// Value of every input an asset lock may spend: final and mature.
    fn final_input_values(&self) -> impl Iterator<Item = u64> + '_ {
        self.accounts
            .iter()
            .flat_map(|(managed, _)| managed.utxos.values())
            .filter(|utxo| {
                (utxo.is_confirmed || utxo.is_instantlocked)
                    && utxo.is_spendable(self.current_height)
            })
            .map(|utxo| utxo.value())
    }

    /// Build `builder` unsigned against fresh clones of the pool's accounts,
    /// then release whatever the build reserved.
    ///
    /// `ManagedCoreFundsAccount::clone` shares the live `ReservationSet`
    /// (`Arc<Mutex<_>>`; source of truth: key-wallet's
    /// `managed_account/reservation.rs` doc comment), so every successful dry
    /// run reserves real wallet outpoints in each account it drew from. The
    /// release MUST run on every success path; dropping it silently strands
    /// real UTXOs for the 24-block TTL. The owner guard frees only what this
    /// build reserved, never a concurrent build's re-reservation.
    fn dry_build(
        &self,
        builder: TransactionBuilder,
    ) -> Result<dash_sdk::dpp::dashcore::Transaction, BuilderError> {
        let mut accounts = self.accounts.clone();
        let mut builder = builder
            .set_fee_rate(FeeRate::new(ASSET_LOCK_FEE_PER_KB))
            .set_current_height(self.current_height);
        for (managed, account) in accounts.iter_mut() {
            builder = builder.add_funding(managed, account);
        }
        let (transaction, _, reservation) =
            builder.require_final_inputs().build_unsigned_reserved()?;
        if let Some(token) = reservation {
            for (managed, _) in &accounts {
                managed.release_reservation_if_owner(&transaction, token);
            }
        }
        Ok(transaction)
    }
}

fn credit_output_payload(value: u64) -> TransactionPayload {
    TransactionPayload::AssetLockPayloadType(AssetLockPayload::new(vec![TxOut {
        value,
        // Credit-output script length affects the serialized payload fee.
        script_pubkey: ScriptBuf::from_bytes(vec![0; P2PKH_CREDIT_OUTPUT_SCRIPT_LEN]),
    }]))
}

fn dry_run_asset_lock_amount(
    pool: &AssetLockFundingPool,
    amount_duffs: u64,
) -> Result<AssetLockDryRun, BuilderError> {
    dry_run_asset_lock_amount_with_strategy(pool, amount_duffs, None)
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
    pool: &AssetLockFundingPool,
    amount_duffs: u64,
    selection_strategy: Option<SelectionStrategy>,
) -> Result<AssetLockDryRun, BuilderError> {
    let mut builder =
        TransactionBuilder::new().set_special_payload(credit_output_payload(amount_duffs));
    if let Some(strategy) = selection_strategy {
        builder = builder.set_selection_strategy(strategy);
    }

    match pool.dry_build(builder) {
        Ok(transaction) if transaction.input.len() > pool.max_inputs => {
            Ok(AssetLockDryRun::TooManyInputs {
                count: transaction.input.len(),
                max: pool.max_inputs,
            })
        }
        Ok(_) => Ok(AssetLockDryRun::Builds),
        Err(BuilderError::CoinSelection(SelectionError::NoUtxosAvailable)) => {
            Ok(AssetLockDryRun::Rejected {
                available: 0,
                required: amount_duffs,
            })
        }
        Err(BuilderError::CoinSelection(SelectionError::InsufficientFunds {
            available,
            required,
        }))
        | Err(BuilderError::InsufficientFunds {
            available,
            required,
        }) => Ok(AssetLockDryRun::Rejected {
            available,
            required,
        }),
        Err(BuilderError::TooManyInputs { count, max }) => Ok(AssetLockDryRun::TooManyInputs {
            count,
            max: max.min(pool.max_inputs),
        }),
        Err(source) => Err(source),
    }
}

fn asset_lock_drain_ceiling(
    pool: &AssetLockFundingPool,
    deadline: &ProbeDeadline,
) -> Result<AssetLockDrainSeed, BuilderError> {
    if deadline.expired() {
        return Ok(AssetLockDrainSeed::Unavailable);
    }
    let builder = TransactionBuilder::new()
        .set_selection_strategy(SelectionStrategy::All)
        .set_special_payload(credit_output_payload(1));

    match pool.dry_build(builder) {
        Ok(transaction) if transaction.input.len() > pool.max_inputs => {
            Ok(AssetLockDrainSeed::TooManyInputs {
                max: pool.max_inputs,
            })
        }
        Ok(transaction) => transaction
            .output
            .first()
            .map(|output| AssetLockDrainSeed::Ceiling(output.value))
            .ok_or_else(|| {
                BuilderError::InvalidData("asset-lock drain produced no credit output".into())
            }),
        Err(BuilderError::CoinSelection(SelectionError::NoUtxosAvailable))
        | Err(BuilderError::CoinSelection(SelectionError::InsufficientFunds { .. }))
        | Err(BuilderError::InsufficientFunds { .. }) => Ok(AssetLockDrainSeed::Unavailable),
        Err(BuilderError::TooManyInputs { max, .. }) => Ok(AssetLockDrainSeed::TooManyInputs {
            max: max.min(pool.max_inputs),
        }),
        Err(source) => Err(source),
    }
}

fn asset_lock_input_cap_upper_bound(pool: &AssetLockFundingPool, max_inputs: usize) -> u64 {
    let mut values: Vec<u64> = pool.final_input_values().collect();
    values.sort_unstable_by(|left, right| right.cmp(left));
    values
        .into_iter()
        .take(max_inputs)
        .fold(0_u64, u64::saturating_add)
}

fn largest_first_asset_lock_max_below(
    pool: &AssetLockFundingPool,
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
            pool,
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
    pool: &AssetLockFundingPool,
    seed: u64,
    upper_bound: u64,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    if deadline.expired() {
        return Ok(0);
    }
    if !matches!(
        dry_run_asset_lock_amount(pool, seed)?,
        AssetLockDryRun::Builds
    ) {
        return asset_lock_max_below_upper_bound(pool, seed, deadline);
    }

    let mut low = seed;
    let mut step = 1_u64;
    while low < upper_bound {
        if deadline.expired() {
            return Ok(low);
        }
        let candidate = seed.saturating_add(step).min(upper_bound);
        match dry_run_asset_lock_amount(pool, candidate)? {
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
                    match dry_run_asset_lock_amount(pool, midpoint)? {
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
    pool: &AssetLockFundingPool,
    max_inputs: usize,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    if deadline.expired() {
        return Ok(0);
    }
    let upper_bound = asset_lock_input_cap_upper_bound(pool, max_inputs);
    let largest_first_seed = largest_first_asset_lock_max_below(pool, upper_bound, deadline)?;
    // Branch-and-bound's exact-match sizing looks one P2PKH input ahead; this
    // is only a starting point, and default-strategy probes remain authoritative.
    let seed = largest_first_seed
        .saturating_sub(FeeRate::new(ASSET_LOCK_FEE_PER_KB).calculate_fee(P2PKH_INPUT_SIZE));
    default_strategy_max_from_seed(pool, seed, upper_bound, deadline)
}

fn asset_lock_max_below_upper_bound(
    pool: &AssetLockFundingPool,
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
        match dry_run_asset_lock_amount(pool, candidate)? {
            AssetLockDryRun::Builds => {
                let mut low = candidate;
                while low < high {
                    if deadline.expired() {
                        return Ok(low);
                    }
                    let midpoint = low + (high - low).div_ceil(2);
                    match dry_run_asset_lock_amount(pool, midpoint)? {
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

    full_range_asset_lock_max_amount(pool, None, deadline)
}

fn full_range_asset_lock_max_amount(
    pool: &AssetLockFundingPool,
    input_cap: Option<usize>,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    if let Some(max_inputs) = input_cap {
        return asset_lock_max_with_input_cap(pool, max_inputs, deadline);
    }

    if deadline.expired() {
        return Ok(0);
    }

    let mut high = match dry_run_asset_lock_amount(pool, MAX_MONEY.saturating_add(1))? {
        AssetLockDryRun::Rejected { available, .. } => available,
        AssetLockDryRun::Builds => MAX_MONEY,
        AssetLockDryRun::TooManyInputs { max, .. } => {
            return asset_lock_max_with_input_cap(pool, max, deadline);
        }
    };
    let mut low = 0;

    while low < high {
        if deadline.expired() {
            return Ok(low);
        }
        let candidate = low + (high - low).div_ceil(2);
        match dry_run_asset_lock_amount(pool, candidate)? {
            AssetLockDryRun::Builds => low = candidate,
            AssetLockDryRun::Rejected { .. } | AssetLockDryRun::TooManyInputs { .. } => {
                high = candidate - 1
            }
        }
    }

    Ok(low)
}

/// Return the largest credit-output amount the real asset-lock builder accepts
/// within the asset-lock input cap.
///
/// A drain build supplies a tight upper bound, then default-strategy probes find
/// the exact boundary without reproducing the selector's internal fee model.
fn asset_lock_max_amount_from_pool_until(
    pool: &AssetLockFundingPool,
    deadline: &ProbeDeadline,
) -> Result<u64, BuilderError> {
    let drain_ceiling = match asset_lock_drain_ceiling(pool, deadline)? {
        AssetLockDrainSeed::Ceiling(ceiling) => ceiling,
        AssetLockDrainSeed::Unavailable => return Ok(0),
        AssetLockDrainSeed::TooManyInputs { max } => {
            return full_range_asset_lock_max_amount(pool, Some(max), deadline);
        }
    };
    asset_lock_max_below_upper_bound(pool, drain_ceiling, deadline)
}

#[cfg(test)]
fn asset_lock_max_amount_from_pool(pool: &AssetLockFundingPool) -> Result<u64, BuilderError> {
    asset_lock_max_amount_from_pool_until(pool, &ProbeDeadline::unbounded())
}

impl WalletBackend {
    /// The protocol's asset-lock input cap, at the SDK's platform version.
    fn asset_lock_input_cap(&self) -> usize {
        asset_lock_input_cap(self.sdk().version())
    }

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
        let max_inputs = self.asset_lock_input_cap();
        // Read before the lock: an operation starting meanwhile leaves this
        // quote stale rather than current.
        let spend_key = self.spend_quote_key(seed_hash)?;
        // Every dry run performs one reservation read→reserve→release cycle.
        // Keep them all under the same exclusive boundary as real builds.
        let wallet_manager = Arc::clone(wallet.wallet_manager()).write_owned().await;
        let pool = {
            let (key_wallet, info) = wallet_manager
                .get_wallet_and_info(&wallet_id)
                .ok_or(TaskError::WalletStateInconsistent)?;
            AssetLockFundingPool::from_wallet(key_wallet, &info.core_wallet, max_inputs)
        };

        tokio::task::spawn_blocking(move || {
            // Started here — not on the async side — so blocking-pool queueing
            // delay is not charged against the builder-work budget.
            let deadline = ProbeDeadline::after(timeout);
            let Some(observed_inputs) = asset_lock_final_input_state(&pool, &deadline)? else {
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
            let amount_duffs = asset_lock_max_amount_from_pool_until(&pool, &deadline)?;
            let is_partial = deadline.timed_out();
            drop(wallet_manager);
            Ok(AssetLockMaxAmountQuote {
                amount_duffs,
                observed_inputs: observed_inputs.with_spend_key(spend_key),
                is_partial,
            })
        })
        .await?
        .map_err(|source| TaskError::AssetLockBalanceQueryFailed {
            source: Box::new(source),
        })
    }

    /// The largest amount, in credits, the upstream builder for `kind`
    /// accepts from this wallet now — the amount behind that operation's Max.
    ///
    /// Both quotes come from upstream and price the same rules the real build
    /// applies: a Core send pools the accounts `SEND_FUNDING_SOURCES` names,
    /// with no change and the input cap, narrowed to what the build accepts
    /// around inputs other builds have reserved; shielding from Platform
    /// follows the shield planner's address order, minimum input and
    /// input-count limit.
    pub async fn spend_limit_credits(
        &self,
        seed_hash: &WalletSeedHash,
        kind: SpendLimitKind,
    ) -> Result<u64, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let quote_failed = |source| TaskError::SpendLimitQueryFailed {
            source: Arc::new(source),
        };
        match kind {
            SpendLimitKind::CoreSend { destination } => {
                // The UI keys this quote on the revision it read before
                // dispatch, so an operation opening after this check leaves
                // the result stale rather than current.
                self.spend_quote_key(seed_hash)?;
                let core = wallet.core();
                let upper_duffs = core
                    .pooled_max_sendable(
                        &platform_wallet::SEND_FUNDING_SOURCES,
                        DEFAULT_BIP44_ACCOUNT,
                        None,
                    )
                    .await
                    .map_err(quote_failed)?;
                if upper_duffs == 0 {
                    // No coin is worth its input: none spendable at all, or
                    // too small to pay for its own spend.
                    let spendable_duffs = core
                        .pooled_spendable_balance(
                            &platform_wallet::SEND_FUNDING_SOURCES,
                            DEFAULT_BIP44_ACCOUNT,
                        )
                        .await
                        .map_err(quote_failed)?;
                    let reason = if spendable_duffs == 0 {
                        CoreSendShortfall::NothingSpendable
                    } else {
                        CoreSendShortfall::BelowFee
                    };
                    return Err(TaskError::CoreSendMaxUnavailable { reason });
                }
                let (proven_duffs, timed_out, input_count) = self
                    .core_send_max_within_reservations(&wallet, upper_duffs, destination)
                    .await?;
                let duffs = core_send_search_outcome(proven_duffs, timed_out)?;
                match crate::model::fee_estimation::core_send_max_duffs(duffs, input_count) {
                    0 => Err(TaskError::CoreSendMaxUnavailable {
                        reason: CoreSendShortfall::BelowFee,
                    }),
                    max_duffs => Ok(max_duffs.saturating_mul(CREDITS_PER_DUFF)),
                }
            }
            SpendLimitKind::ShieldFromPlatform => {
                // As for a Core send: the UI keys this quote on the revision
                // it read before dispatch, so a shield opening after this
                // check leaves the result stale rather than current.
                self.platform_shield_quote_key(seed_hash)?;
                wallet
                    // The payment account `ShieldFromBalance` shields from.
                    .shielded_shield_preflight(super::SHIELD_FROM_BALANCE_PAYMENT_ACCOUNT)
                    .await
                    .map(|preflight| {
                        if preflight.can_shield {
                            preflight.max_shieldable_credits
                        } else {
                            0
                        }
                    })
                    .map_err(quote_failed)
            }
        }
    }

    /// Narrow upstream's Core-send quote to what the pooled build accepts now
    /// for a payment to a `destination` script.
    ///
    /// `pooled_max_sendable` prices a P2PKH output, while the build prices
    /// the real one: a smaller script raises the ceiling by the fee its saved
    /// bytes cost. It also counts inputs another in-flight build has
    /// reserved, which can only make the real ceiling lower. The quote is
    /// confirmed with the same reservation-aware dry run a send runs; when a
    /// reservation bites, the largest amount that builds is searched for
    /// below it, under the wallet-manager write lock and a deadline that
    /// settles on the largest amount already proven. Also returns how many
    /// coins the pooled send of the whole balance spends, which prices its fee.
    // TODO(upstream): make `CoreWallet::pooled_max_sendable` leave out
    // reserved inputs so this search can go (no dashpay/platform issue yet).
    async fn core_send_max_within_reservations(
        &self,
        wallet: &platform_wallet::PlatformWallet,
        upper_duffs: u64,
        destination: CoreDestinationScript,
    ) -> Result<(u64, bool, usize), TaskError> {
        if upper_duffs == 0 {
            return Ok((0, false, 0));
        }
        let wallet_id = wallet.wallet_id();
        let network = self.inner.network;
        let recipient = core_quote_recipient(network, destination);
        let upper_duffs = upper_duffs.saturating_add(fee_saved_against_p2pkh(network, &recipient));
        let wallet_manager = Arc::clone(wallet.wallet_manager()).write_owned().await;
        tokio::task::spawn_blocking(move || {
            let deadline = ProbeDeadline::after(CORE_SEND_MAX_PROBE_LOCK_HOLD_DEADLINE);
            let (key_wallet, info) = wallet_manager
                .get_wallet_and_info(&wallet_id)
                .ok_or(TaskError::WalletStateInconsistent)?;
            let largest = largest_core_payment_at_most(
                key_wallet,
                &info.core_wallet,
                &recipient,
                upper_duffs,
                &deadline,
            );
            let input_count = pooled_send_input_count(key_wallet, &info.core_wallet);
            Ok((largest, deadline.timed_out(), input_count))
        })
        .await?
    }

    /// Refuse an exact-amount asset lock that would need more inputs than the
    /// protocol accepts.
    ///
    /// The upstream exact-amount builder enforces only the standard-transaction
    /// limit, and Dash Core silently drops a larger asset lock, leaving its
    /// inputs reserved and the lock unconfirmed. The same dry run the Max probe
    /// uses decides it; other funding kinds are left to upstream.
    ///
    /// Every asset-lock path runs it twice: before the secret session, so a
    /// doomed lock never prompts, and again inside the session immediately
    /// before the upstream build, because the wallet can change while the user
    /// types a password. Upstream takes the wallet-manager lock itself and
    /// builds and broadcasts in one call, so the second check is as close to
    /// the build as DET can get; a concurrent spend or sync landing between
    /// the two lock acquisitions can still change the selection.
    // TODO(dashpay/platform#5513): once upstream enforces
    // `max_asset_lock_inputs` for `Exact` builds in
    // `broadcast_funded_asset_lock_with_operation` (its abandon arm covers
    // only `DrainAll`), the cap is atomic with the build and this guard can go
    // (DET follow-up draft: asset-lock-exact-input-cap).
    pub(super) async fn ensure_asset_lock_within_input_cap(
        &self,
        seed_hash: &WalletSeedHash,
        funding: &platform_wallet::wallet::asset_lock::AssetLockFunding,
    ) -> Result<(), TaskError> {
        let platform_wallet::wallet::asset_lock::AssetLockFunding::FromWalletBalance {
            amount_duffs,
            ..
        } = funding
        else {
            return Ok(());
        };
        self.ensure_asset_lock_amount_within_input_cap(seed_hash, *amount_duffs)
            .await
    }

    async fn ensure_asset_lock_amount_within_input_cap(
        &self,
        seed_hash: &WalletSeedHash,
        amount_duffs: u64,
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let max_inputs = self.asset_lock_input_cap();
        let wallet_manager = Arc::clone(wallet.wallet_manager()).write_owned().await;
        let pool = {
            let (key_wallet, info) = wallet_manager
                .get_wallet_and_info(&wallet_id)
                .ok_or(TaskError::WalletStateInconsistent)?;
            AssetLockFundingPool::from_wallet(key_wallet, &info.core_wallet, max_inputs)
        };
        let outcome = tokio::task::spawn_blocking(move || {
            let outcome = dry_run_asset_lock_amount(&pool, amount_duffs);
            drop(wallet_manager);
            outcome
        })
        .await?;
        match outcome {
            Ok(AssetLockDryRun::TooManyInputs { count, max }) => {
                Err(TaskError::AssetLockTooManyInputs { count, max })
            }
            // The pool shares the live reservations, so a shortfall here is
            // one the real build would hit too — refuse it before the unlock.
            Ok(AssetLockDryRun::Rejected {
                available,
                required,
            }) => Err(TaskError::InsufficientFunds {
                available,
                required,
            }),
            // Any other build failure is the real build's to report, with the
            // error it already maps.
            Ok(AssetLockDryRun::Builds) | Err(_) => Ok(()),
        }
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

    /// Build, sign, and broadcast a payment to `recipients` (`(address,
    /// duffs)`), funded from the wallet's pooled accounts. Returns the txid.
    ///
    /// The build goes through upstream's atomic finalizer with
    /// [`platform_wallet::SEND_FUNDING_SOURCES`] — BIP-44, BIP-32 and every
    /// DashPay receiving account, CoinJoin excluded, change back to BIP-44 —
    /// the same set [`Self::spend_limit_credits`] quotes Max against. A dry
    /// run of the same build runs first, before any secret is opened, so a
    /// payment no build can fund is refused with a typed error and no prompt.
    ///
    /// One [`SecretAccess::with_secret_session`](super::SecretAccess::with_secret_session) scope wraps the signing:
    /// the seed is decrypted just-in-time (one prompt for a passphrase-
    /// protected wallet, none for a no-password wallet), borrowed by the
    /// [`DetSigner`] for every input sign, and zeroized when the scope ends.
    pub async fn send_payment(
        &self,
        seed_hash: &WalletSeedHash,
        recipients: Vec<(dash_sdk::dpp::dashcore::Address, u64)>,
    ) -> Result<dash_sdk::dpp::dashcore::Txid, TaskError> {
        let _spend = self.begin_core_spend(seed_hash);
        self.ensure_core_payment_builds(seed_hash, &recipients)
            .await?;
        let requested = requested_duffs(&recipients);
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                // Funding and reservation happen in one critical section
                // under the wallet-manager write lock inside upstream; the
                // signer runs after it drops.
                let transaction = wallet
                    .core()
                    .finalize_transaction(
                        core_payment_builder(&recipients),
                        &platform_wallet::SEND_FUNDING_SOURCES,
                        DEFAULT_BIP44_ACCOUNT,
                        &signer,
                    )
                    .await
                    .map_err(|source| map_core_finalize_error(source, requested))?;
                let txid = transaction.transaction().txid();
                // A definitive rejection releases the reservation on every
                // funding account; an ambiguous outcome keeps it.
                wallet
                    .core()
                    .broadcast_finalized_transaction(&transaction)
                    .await
                    .map_err(|e| map_core_broadcast_error(Some(txid), e))?;
                Ok(txid)
            })
            .await
    }

    /// Refuse a payment no build can fund, with the builder's own typed
    /// reason, by running the same pooled build unsigned and releasing it.
    ///
    /// Upstream's finalizer reports a too-many-inputs refusal only as text, so
    /// this dry run is what keeps that case typed. A shortfall, or an input
    /// count above the standard-transaction limit, gives balance or
    /// consolidation advice; anything else falls back to the generic
    /// payment-build error.
    // TODO(upstream): type `BuilderError::TooManyInputs` in
    // `CoreWallet::finalize_transaction`'s error so this dry run can go.
    async fn ensure_core_payment_builds(
        &self,
        seed_hash: &WalletSeedHash,
        recipients: &[(dash_sdk::dpp::dashcore::Address, u64)],
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let wallet_manager = wallet.wallet_manager().write().await;
        let (key_wallet, info) = wallet_manager
            .get_wallet_and_info(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        let requested = requested_duffs(recipients);
        dry_run_core_payment(key_wallet, &info.core_wallet, recipients).map_err(|source| {
            match source {
                BuilderError::InsufficientFunds {
                    available,
                    required,
                }
                | BuilderError::CoinSelection(SelectionError::InsufficientFunds {
                    available,
                    required,
                }) => TaskError::InsufficientFunds {
                    available,
                    required,
                },
                BuilderError::CoinSelection(SelectionError::NoUtxosAvailable) => {
                    TaskError::InsufficientFunds {
                        available: 0,
                        required: requested,
                    }
                }
                BuilderError::TooManyInputs { count, max } => {
                    TaskError::WalletPaymentTooManyInputs { count, max }
                }
                other => TaskError::WalletPaymentBuildFailed {
                    source: Box::new(other),
                },
            }
        })
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

        let _spend = self.begin_core_spend(seed_hash);
        self.ensure_asset_lock_amount_within_input_cap(seed_hash, amount_duffs)
            .await?;
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
                // The wallet may have changed during the prompt.
                self.ensure_asset_lock_amount_within_input_cap(seed_hash, amount_duffs)
                    .await?;
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
                    // Upstream keeps both the UTXO reservation and the
                    // resumable Built row when this broadcast is ambiguous, so
                    // the generic envelope's "please retry" would advise a
                    // resend that cannot go through. It builds and broadcasts
                    // internally, so no id reaches us to watch.
                    .map_err(|e| map_core_broadcast_error(None, e))?;
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

/// The unfunded builder a Core payment to `recipients` starts from, shared by
/// the dry run and the real build so the two cannot disagree.
fn core_payment_builder(
    recipients: &[(dash_sdk::dpp::dashcore::Address, u64)],
) -> TransactionBuilder {
    recipients
        .iter()
        .fold(TransactionBuilder::new(), |builder, (address, amount)| {
            builder.add_output(address, *amount)
        })
}

/// Build the payment to `recipients` unsigned from the pooled send accounts,
/// the way upstream's finalizer funds it, then release what it reserved.
///
/// The accounts are clones sharing the live `ReservationSet`, so the release
/// MUST run on success; the owner guard frees only this build's reservation.
fn dry_run_core_payment(
    wallet: &Wallet,
    info: &ManagedWalletInfo,
    recipients: &[(dash_sdk::dpp::dashcore::Address, u64)],
) -> Result<(), BuilderError> {
    let mut accounts = pooled_funding_accounts(
        wallet,
        &info.accounts,
        &platform_wallet::SEND_FUNDING_SOURCES,
        DEFAULT_BIP44_ACCOUNT,
    );
    let mut builder =
        core_payment_builder(recipients).set_current_height(asset_lock_builder_height(info));
    for (managed, account) in accounts.iter_mut() {
        builder = builder.add_funding(managed, account);
    }
    let (transaction, _, reservation) = builder.build_unsigned_reserved()?;
    if let Some(token) = reservation {
        for (managed, _) in &accounts {
            managed.release_reservation_if_owner(&transaction, token);
        }
    }
    Ok(())
}

/// The largest single payment of at most `upper_duffs` the pooled send
/// build accepts now, reservations included. Buildability falls with the
/// amount, so a binary search finds it; when `deadline` expires first, the
/// largest amount already proven to build is returned, never a guess.
fn largest_core_payment_at_most(
    wallet: &Wallet,
    info: &ManagedWalletInfo,
    recipient: &dash_sdk::dpp::dashcore::Address,
    upper_duffs: u64,
    deadline: &ProbeDeadline,
) -> u64 {
    let builds =
        |amount: u64| dry_run_core_payment(wallet, info, &[(recipient.clone(), amount)]).is_ok();
    if builds(upper_duffs) {
        return upper_duffs;
    }
    let (mut low, mut high) = (0, upper_duffs - 1);
    while low < high {
        if deadline.expired() {
            break;
        }
        let candidate = low + (high - low).div_ceil(2);
        if builds(candidate) {
            low = candidate;
        } else {
            high = candidate - 1;
        }
    }
    low
}

/// The Core send Max a reservation-aware search settled on. Zero is never
/// offered as Max: with time to spare it means every spendable coin is held
/// by another build; out of time, it means nothing was proven yet.
fn core_send_search_outcome(proven_duffs: u64, timed_out: bool) -> Result<u64, TaskError> {
    match proven_duffs {
        0 if timed_out => Err(TaskError::SpendLimitTimedOut),
        0 => Err(TaskError::CoreSendMaxUnavailable {
            reason: CoreSendShortfall::HeldByPendingTransaction,
        }),
        proven => Ok(proven),
    }
}

/// A stand-in payee paid through a `destination` script: the output's size,
/// not its payee, sets the fee.
fn core_quote_recipient(
    network: dash_sdk::dpp::dashcore::Network,
    destination: CoreDestinationScript,
) -> dash_sdk::dpp::dashcore::Address {
    use dash_sdk::dpp::dashcore::address::Payload;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    let payload = match destination {
        CoreDestinationScript::PubkeyHash => {
            Payload::PubkeyHash(dash_sdk::dpp::dashcore::PubkeyHash::all_zeros())
        }
        CoreDestinationScript::ScriptHash => {
            Payload::ScriptHash(dash_sdk::dpp::dashcore::ScriptHash::all_zeros())
        }
    };
    dash_sdk::dpp::dashcore::Address::new(network, payload)
}

/// The fee a payment to `recipient` saves against the P2PKH output
/// `pooled_max_sendable` prices, at the rate the send builds with.
fn fee_saved_against_p2pkh(
    network: dash_sdk::dpp::dashcore::Network,
    recipient: &dash_sdk::dpp::dashcore::Address,
) -> u64 {
    let p2pkh = core_quote_recipient(network, CoreDestinationScript::PubkeyHash);
    crate::model::fee_estimation::core_output_script_fee_saved_duffs(
        p2pkh.script_pubkey().len(),
        recipient.script_pubkey().len(),
    )
}

/// The total a payment to `recipients` asks for, in duffs.
fn requested_duffs(recipients: &[(dash_sdk::dpp::dashcore::Address, u64)]) -> u64 {
    recipients
        .iter()
        .fold(0_u64, |total, (_, amount)| total.saturating_add(*amount))
}

/// Classify a failed pooled payment build of `requested` duffs. A shortfall
/// the dry run did not see (the wallet changed in between) keeps its balance
/// advice whenever upstream names the available figure. Upstream leaves the
/// required figure out when no unreserved UTXO is left; the requested amount
/// stands in for it, as in the dry run's mapping of the same case.
fn map_core_finalize_error(
    source: platform_wallet::error::PlatformWalletError,
    requested: u64,
) -> TaskError {
    match source {
        platform_wallet::error::PlatformWalletError::CorePooledInsufficientFunds {
            available: Some(available),
            required,
            ..
        } => TaskError::InsufficientFunds {
            available,
            required: required.unwrap_or(requested),
        },
        other => TaskError::WalletBackend {
            source: Arc::new(other),
        },
    }
}

/// Classify a broadcast failure from the payment path. Pure — unit-testable.
///
/// An ambiguous outcome gets its own [`TaskError::TransactionConfirmationUnknown`]
/// rather than the generic envelope: the payment may already be on the network,
/// and upstream deliberately keeps its inputs reserved so it cannot be sent
/// twice, so telling the user to retry would be both wrong and unactionable.
/// Every definitive failure keeps the generic envelope.
///
/// `txid` is the broadcast transaction's id when the caller holds it, so the UI
/// can watch that transaction for a late confirmation; `None` where the
/// transaction was assembled and sent inside an upstream orchestrator that
/// returns no id.
pub(super) fn map_core_broadcast_error(
    txid: Option<Txid>,
    source: platform_wallet::error::PlatformWalletError,
) -> TaskError {
    match source {
        platform_wallet::error::PlatformWalletError::TransactionBroadcastUnconfirmed(_) => {
            TaskError::TransactionConfirmationUnknown {
                txid,
                source: Box::new(source),
            }
        }
        other => TaskError::WalletBackend {
            source: Arc::new(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ASSET_LOCK_FEE_PER_KB, AssetLockFundingPool, MAX_MONEY, asset_lock_builder_height,
        asset_lock_max_amount_from_pool, map_core_broadcast_error, map_core_finalize_error,
        unbound_topup_lock_eligible,
    };
    use crate::backend_task::error::TaskError;
    use crate::model::fee_estimation::estimate_core_send_fee_duffs;
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

    /// key-wallet's standard-transaction input limit, for tests that price a
    /// single hand-funded account against the builder alone.
    const STANDARD_TRANSACTION_INPUT_CAP: usize = 500;

    /// A Max send folds its zero/dust remainder into the fee (DET #909,
    /// rust-dashcore #911).
    #[test]
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
        // The balance less the pooled send fee the Max is priced with.
        let max_amount = BALANCE_DUFFS - estimate_core_send_fee_duffs(1);

        assert_eq!(max_amount, 9_999_808);
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

    /// A wallet's payments spent whole into a funding lose the Core fee. The
    /// model's estimate of that fee sizes deposit requests, so it must never be
    /// below what the builder charges.
    #[test]
    fn asset_lock_core_fee_estimate_covers_what_the_builder_charges() {
        use crate::model::fee_estimation::estimate_asset_lock_core_fee_duffs;
        const PAYMENT_DUFFS: u64 = 430_000;
        const CURRENT_HEIGHT: u32 = 200;

        // (payments spent whole, what the builder is known to charge for them)
        for (payments, known_charge) in [(1usize, Some(229)), (2, Some(377)), (5, None)] {
            let wallet =
                Wallet::new_random(Network::Testnet, WalletAccountCreationOptions::Default)
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
            for index in 0..payments {
                let outpoint = OutPoint::new(Txid::from_byte_array([index as u8 + 1; 32]), 0);
                let mut utxo = Utxo::new(
                    outpoint,
                    TxOut {
                        value: PAYMENT_DUFFS,
                        script_pubkey: funding_address.script_pubkey(),
                    },
                    funding_address.clone(),
                    100,
                    false,
                );
                utxo.is_confirmed = true;
                managed_account.utxos.insert(outpoint, utxo);
            }

            let ceiling = asset_lock_max_amount_from_pool(&AssetLockFundingPool::for_account(
                managed_account,
                account,
                CURRENT_HEIGHT,
                STANDARD_TRANSACTION_INPUT_CAP,
            ))
            .expect("asset-lock maximum");
            let charged = PAYMENT_DUFFS * payments as u64 - ceiling;
            let estimate = estimate_asset_lock_core_fee_duffs(payments);
            if let Some(known_charge) = known_charge {
                assert_eq!(charged, known_charge, "{payments} payments");
            }
            assert!(
                charged <= estimate,
                "{payments} payments: the builder charges {charged} duffs, the estimate is {estimate}"
            );
        }
    }

    /// The Core fee a send shows is the fee the pooled build charges when it
    /// sends the Max: the same pool (BIP44 and BIP32, not CoinJoin), the same
    /// rate, and no coin that costs more than it brings.
    #[test]
    fn core_send_fee_preview_is_what_the_pooled_build_charges_at_max() {
        use crate::model::fee_estimation::estimate_core_send_fee_duffs;
        const COIN_DUFFS: u64 = 700_000;
        const DUST_DUFFS: u64 = 100;
        const CURRENT_HEIGHT: u32 = 200;

        let wallet = Wallet::new_random(Network::Testnet, WalletAccountCreationOptions::Default)
            .expect("test wallet");
        let mut info = ManagedWalletInfo::from_wallet_with_name(&wallet, "Test".to_string(), 0);
        info.update_last_processed_height(CURRENT_HEIGHT);
        let bip44 = wallet.get_bip44_account(0).expect("BIP44 account");
        let bip32 = wallet
            .accounts
            .standard_bip32_accounts
            .get(&0)
            .expect("BIP32 account");
        let coin = |address: Address, tag: u8, value: u64| {
            let outpoint = OutPoint::new(Txid::from_byte_array([tag; 32]), 0);
            let mut utxo = Utxo::new(
                outpoint,
                TxOut {
                    value,
                    script_pubkey: address.script_pubkey(),
                },
                address,
                100,
                false,
            );
            utxo.is_confirmed = true;
            (outpoint, utxo)
        };
        let bip44_managed = info
            .accounts
            .standard_bip44_accounts
            .get_mut(&0)
            .expect("managed BIP44 account");
        for (tag, value) in [(1, COIN_DUFFS), (2, COIN_DUFFS), (3, DUST_DUFFS)] {
            let address = bip44_managed
                .next_receive_address(Some(&bip44.account_xpub), true)
                .expect("BIP44 address");
            let (outpoint, utxo) = coin(address, tag, value);
            bip44_managed.utxos.insert(outpoint, utxo);
        }
        let bip32_managed = info
            .accounts
            .standard_bip32_accounts
            .get_mut(&0)
            .expect("managed BIP32 account");
        let address = bip32_managed
            .next_receive_address(Some(&bip32.account_xpub), true)
            .expect("BIP32 address");
        let (outpoint, utxo) = coin(address, 4, COIN_DUFFS);
        bip32_managed.utxos.insert(outpoint, utxo);
        // CoinJoin coins never fund a plain send; only its value matters here.
        let elsewhere = Address::p2pkh(
            &PublicKey::from_slice(&[0x02; 33]).expect("valid compressed public key"),
            Network::Testnet,
        );
        let (outpoint, utxo) = coin(elsewhere.clone(), 5, COIN_DUFFS);
        info.accounts
            .coinjoin_accounts
            .get_mut(&0)
            .expect("managed CoinJoin account")
            .utxos
            .insert(outpoint, utxo);

        let input_count = super::pooled_send_input_count(&wallet, &info);
        let preview = estimate_core_send_fee_duffs(input_count);
        let max = 3 * COIN_DUFFS - preview;

        let recipient = elsewhere;
        let mut pooled = super::pooled_funding_accounts(
            &wallet,
            &info.accounts,
            &platform_wallet::SEND_FUNDING_SOURCES,
            super::DEFAULT_BIP44_ACCOUNT,
        );
        let mut builder = super::core_payment_builder(&[(recipient, max)])
            .set_current_height(asset_lock_builder_height(&info));
        for (managed, account) in pooled.iter_mut() {
            builder = builder.add_funding(managed, account);
        }
        let (transaction, charged, reservation) = builder
            .build_unsigned_reserved()
            .expect("the pooled build accepts the Max");
        if let Some(token) = reservation {
            for (managed, _) in &pooled {
                managed.release_reservation_if_owner(&transaction, token);
            }
        }
        assert_eq!(transaction.input.len(), 3, "BIP44 and BIP32 coins, no dust");
        assert_eq!(input_count, 3);
        assert_eq!(preview, charged, "the preview is the fee the build charges");
    }

    /// A credit output script as long as the P2PKH one a real asset lock pays
    /// to. The builder sizes the asset-lock payload, and so its fee, from the
    /// actual credit outputs, so a shorter script would make the fee cheaper
    /// than in production and let a larger amount build.
    fn credit_output_script() -> ScriptBuf {
        ScriptBuf::from_bytes(vec![0; super::P2PKH_CREDIT_OUTPUT_SCRIPT_LEN])
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
        let max_amount = asset_lock_max_amount_from_pool(&AssetLockFundingPool::for_account(
            managed_account,
            account,
            CURRENT_HEIGHT,
            STANDARD_TRANSACTION_INPUT_CAP,
        ))
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
                    script_pubkey: credit_output_script(),
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
                    script_pubkey: credit_output_script(),
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
                    script_pubkey: credit_output_script(),
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

        let max_amount = asset_lock_max_amount_from_pool(&AssetLockFundingPool::for_account(
            managed_account,
            account,
            CURRENT_HEIGHT,
            STANDARD_TRANSACTION_INPUT_CAP,
        ))
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
                    script_pubkey: credit_output_script(),
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
                    script_pubkey: credit_output_script(),
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
        use crate::backend_task::TaskResult;
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
        use crate::backend_task::TaskResult;
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
        use crate::backend_task::TaskResult;
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

    /// A send whose broadcast outcome is ambiguous must not be reported with
    /// the generic wallet-backend message, which tells the user to retry: the
    /// payment may already be on the network, and upstream keeps its inputs
    /// reserved so it must not be sent a second time.
    #[test]
    fn an_ambiguous_broadcast_does_not_ask_the_user_to_retry() {
        let err = map_core_broadcast_error(
            None,
            platform_wallet::error::PlatformWalletError::TransactionBroadcastUnconfirmed(
                "peer timed out after send".to_string(),
            ),
        );
        assert!(
            matches!(err, TaskError::TransactionConfirmationUnknown { .. }),
            "expected TransactionConfirmationUnknown, got: {err:?}"
        );
    }

    /// An unambiguous broadcast rejection keeps the generic envelope — the
    /// carve-out above must not swallow the ordinary failure path.
    #[test]
    fn a_definitive_broadcast_rejection_keeps_the_generic_envelope() {
        let err = map_core_broadcast_error(
            None,
            platform_wallet::error::PlatformWalletError::TransactionBroadcast(
                "rejected by peer".to_string(),
            ),
        );
        assert!(
            matches!(err, TaskError::WalletBackend { .. }),
            "expected WalletBackend, got: {err:?}"
        );
    }

    /// The in-limbo payment's id has to survive the classification, or nothing
    /// downstream can tell whether that transaction ever landed — the user is
    /// left to reconcile a funds movement by hand.
    #[test]
    fn map_core_broadcast_error_carries_the_txid_of_an_ambiguous_send() {
        let sent = Txid::from_byte_array([7u8; 32]);
        let err = map_core_broadcast_error(
            Some(sent),
            platform_wallet::error::PlatformWalletError::TransactionBroadcastUnconfirmed(
                "peer timed out after send".to_string(),
            ),
        );
        assert!(
            matches!(
                err,
                TaskError::TransactionConfirmationUnknown {
                    txid: Some(seen),
                    ..
                } if seen == sent
            ),
            "expected the broadcast txid to reach the error, got: {err:?}"
        );
    }

    /// A definitive rejection must not smuggle the txid into an envelope that
    /// invites a resend: those inputs are released, so there is nothing to
    /// watch and a retry is the correct advice.
    #[test]
    fn map_core_broadcast_error_drops_the_txid_on_a_definitive_rejection() {
        let err = map_core_broadcast_error(
            Some(Txid::from_byte_array([7u8; 32])),
            platform_wallet::error::PlatformWalletError::TransactionBroadcast(
                "rejected by peer".to_string(),
            ),
        );
        assert!(
            matches!(err, TaskError::WalletBackend { .. }),
            "expected WalletBackend, got: {err:?}"
        );
    }

    /// Upstream reports a pooled build that found no unreserved UTXO as a
    /// shortfall with no required figure. It keeps its balance advice, with
    /// the requested amount as the figure needed, as the dry run gives it.
    #[test]
    fn a_finalize_with_no_utxos_left_keeps_the_balance_advice() {
        let err = map_core_finalize_error(
            platform_wallet::error::PlatformWalletError::CorePooledInsufficientFunds {
                sources: platform_wallet::SEND_FUNDING_SOURCES.to_vec(),
                available: Some(0),
                required: None,
            },
            250_000,
        );
        assert!(
            matches!(
                err,
                TaskError::InsufficientFunds {
                    available: 0,
                    required: 250_000,
                }
            ),
            "expected InsufficientFunds for the requested amount, got: {err:?}"
        );
    }

    /// A test backend with no wallet registered yet.
    async fn backend_without_wallets() -> (
        tempfile::TempDir,
        std::sync::Arc<crate::context::AppContext>,
        std::sync::Arc<crate::wallet_backend::WalletBackend>,
    ) {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        app_context
            .ensure_wallet_backend(SenderAsync::new(sender, app_context.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = app_context.wallet_backend().expect("wired backend");
        (temp_dir, app_context, backend)
    }

    /// A test backend with one registered, unfunded wallet.
    async fn backend_with_registered_wallet() -> (
        tempfile::TempDir,
        std::sync::Arc<crate::context::AppContext>,
        std::sync::Arc<crate::wallet_backend::WalletBackend>,
        crate::model::wallet::WalletSeedHash,
        std::sync::Arc<platform_wallet::PlatformWallet>,
    ) {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        app_context
            .ensure_wallet_backend(SenderAsync::new(sender, app_context.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = app_context.wallet_backend().expect("wired backend");
        // Generated per run: no wallet secret is ever committed.
        let seed: [u8; 64] = rand::random();
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("Funding pool test".to_string()),
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
        (temp_dir, app_context, backend, seed_hash, platform_wallet)
    }

    /// Accounts named by several funding sources are pooled once, at the
    /// position of their first source, so the funding order is kept.
    #[test]
    fn pooled_account_types_keep_their_first_position_once() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let bip44 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        };
        let bip32 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP32Account,
        };
        let coinjoin = AccountType::CoinJoin { index: 0 };

        assert_eq!(
            super::first_occurrences([bip32, bip44, bip32, coinjoin, bip44, bip32]),
            vec![bip32, bip44, coinjoin]
        );
        assert!(super::first_occurrences([]).is_empty());
    }

    /// Give `account_type` one confirmed UTXO per value, at height 200.
    async fn fund_account(
        platform_wallet: &platform_wallet::PlatformWallet,
        account_type: dash_sdk::dpp::key_wallet::account::AccountType,
        txid_byte: u8,
        values: &[u64],
    ) {
        let wallet_id = platform_wallet.wallet_id();
        let mut manager = platform_wallet.wallet_manager().write().await;
        let account = manager
            .get_wallet_and_info(&wallet_id)
            .and_then(|(key_wallet, _)| key_wallet.accounts.account_of_type(account_type))
            .expect("funding account keys")
            .clone();
        let info = manager
            .get_wallet_info_mut(&wallet_id)
            .expect("wallet info");
        info.core_wallet.update_last_processed_height(200);
        let managed_account = info
            .core_wallet
            .accounts
            .funds_account_mut(&account_type)
            .expect("managed funding account");
        let funding_address = managed_account
            .next_receive_address(Some(&account.account_xpub), true)
            .expect("funding address");
        for (index, value) in values.iter().enumerate() {
            let outpoint = OutPoint::new(Txid::from_byte_array([txid_byte; 32]), index as u32);
            let mut utxo = Utxo::new(
                outpoint,
                TxOut {
                    value: *value,
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

    /// Asset locks fund from every pooled account, so funds held only in the
    /// BIP-32 account still count toward Max.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asset_lock_max_counts_funds_outside_the_bip44_account() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP32Account,
            },
            0x62,
            &[1_000_000],
        )
        .await;

        let quote = backend
            .asset_lock_max_amount(&seed_hash)
            .await
            .expect("asset-lock maximum");

        assert!(
            quote.amount_duffs > 990_000 && quote.amount_duffs < 1_000_000,
            "the BIP-32 funds must be lockable, less the network fee: {}",
            quote.amount_duffs
        );
        assert_eq!(
            quote.observed_inputs.final_funds_duffs, 1_000_000,
            "the composition key must cover the pooled account too"
        );
    }

    /// Dash Core relays at most 100 asset-lock inputs, so Max must not offer an
    /// amount only a larger lock could fund.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asset_lock_max_stays_within_the_asset_lock_input_cap() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        const UTXO_DUFFS: u64 = 100_000;
        const CAP: u64 = 100;

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x64,
            &[UTXO_DUFFS; 150],
        )
        .await;

        let quote = backend
            .asset_lock_max_amount(&seed_hash)
            .await
            .expect("asset-lock maximum");

        assert!(
            quote.amount_duffs <= CAP * UTXO_DUFFS,
            "Max must fit in {CAP} inputs: {}",
            quote.amount_duffs
        );
        assert!(
            quote.amount_duffs > (CAP - 1) * UTXO_DUFFS,
            "Max must still use the full cap: {}",
            quote.amount_duffs
        );
    }

    /// An exact amount only a lock above the cap could fund is refused before
    /// any secret is opened or anything is built.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_asset_lock_above_the_input_cap_is_refused_before_building() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        const UTXO_DUFFS: u64 = 100_000;

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x66,
            &[UTXO_DUFFS; 150],
        )
        .await;

        let within = platform_wallet::wallet::asset_lock::AssetLockFunding::FromWalletBalance {
            amount_duffs: 50 * UTXO_DUFFS,
            account_index: 0,
        };
        backend
            .ensure_asset_lock_within_input_cap(&seed_hash, &within)
            .await
            .expect("an amount within the cap passes");

        let result = backend
            .create_asset_lock_proof(
                &seed_hash,
                120 * UTXO_DUFFS,
                platform_wallet::AssetLockFundingType::AssetLockAddressTopUp,
                0,
            )
            .await;
        assert!(
            matches!(
                result,
                Err(TaskError::AssetLockTooManyInputs { count, max: 100 }) if count > 100
            ),
            "{result:?}"
        );
    }

    /// Every operation that can take or release Core input reservations
    /// moves both Max quote keys — the Core send key and the asset-lock input
    /// composition — so a quote made before it is never offered after it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reserving_operation_invalidates_both_max_quotes() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x71,
            &[1_000_000],
        )
        .await;
        let keys = |backend: &crate::wallet_backend::WalletBackend| {
            (
                backend.core_send_quote_revision(&seed_hash),
                backend.asset_lock_probe_snapshot(&seed_hash).1,
            )
        };
        let before = keys(&backend);

        // Both refuse a shortfall before any prompt; each still ran as a
        // reserving operation.
        let send = backend
            .send_payment(&seed_hash, vec![(payment_recipient(), 50_000_000)])
            .await;
        assert!(send.is_err(), "{send:?}");
        let after_send = keys(&backend);
        assert_ne!(after_send.0, before.0, "a send moves the Core Max key");
        assert_ne!(after_send.1, before.1, "a send moves the asset-lock key");

        let lock = backend
            .create_asset_lock_proof(
                &seed_hash,
                50_000_000,
                platform_wallet::AssetLockFundingType::AssetLockAddressTopUp,
                0,
            )
            .await;
        assert!(lock.is_err(), "{lock:?}");
        let after_lock = keys(&backend);
        assert_ne!(
            after_lock.0, after_send.0,
            "an asset lock moves the Core Max key"
        );
        assert_ne!(
            after_lock.1, after_send.1,
            "an asset lock moves the asset-lock key"
        );
    }

    /// A shield from Platform above what the planner can shield is refused
    /// before the wallet is unlocked, with the planner's own verdict — here,
    /// a wallet with no Platform balance can shield nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_shield_above_the_planner_maximum_is_refused_before_unlocking() {
        let (_dir, _ctx, backend, seed_hash, _platform_wallet) =
            backend_with_registered_wallet().await;
        let det_wallet = crate::model::wallet::Wallet::new_from_seed(
            rand::random(),
            Network::Testnet,
            None,
            None,
        )
        .expect("DET wallet");
        let path_index =
            crate::wallet_backend::PlatformPathIndex::from_wallet(&det_wallet, Network::Testnet);
        let before = backend.platform_shield_quote_revision(&seed_hash);

        let refused = backend
            .shield_from_balance(
                &seed_hash,
                &path_index,
                0,
                crate::wallet_backend::SHIELD_FROM_BALANCE_PAYMENT_ACCOUNT,
                100_000_000_000,
            )
            .await;

        assert!(
            matches!(
                refused,
                Err(TaskError::ShieldFromPlatformNothingShieldable { .. })
            ),
            "{refused:?}"
        );
        // The refused shield still ran as a shield: it moved the shield Max
        // key and, once settled, leaves the wallet free to quote again.
        assert_ne!(backend.platform_shield_quote_revision(&seed_hash), before);
        let quote = backend
            .spend_limit_credits(
                &seed_hash,
                crate::model::spend_limit::SpendLimitKind::ShieldFromPlatform,
            )
            .await;
        assert!(
            !matches!(quote, Err(TaskError::SpendInProgress)),
            "{quote:?}"
        );
    }

    /// A shield from Platform spends credits the balances its Max is keyed
    /// on keep showing until a sync pass reads them back, so while one is in
    /// flight no shield Max is quoted. Its key moves when the shield opens and
    /// again when it settles, and Core quotes are untouched.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_shield_max_is_quoted_while_a_shield_from_platform_is_in_flight() {
        let (_dir, _ctx, backend, seed_hash, _platform_wallet) =
            backend_with_registered_wallet().await;
        let kind = crate::model::spend_limit::SpendLimitKind::ShieldFromPlatform;
        let idle_key = backend.platform_shield_quote_revision(&seed_hash);
        let core_key = backend.core_send_quote_revision(&seed_hash);
        let idle = backend.spend_limit_credits(&seed_hash, kind).await;
        assert!(!matches!(idle, Err(TaskError::SpendInProgress)), "{idle:?}");

        let shielding = backend.begin_platform_shield(&seed_hash);
        let busy = backend.spend_limit_credits(&seed_hash, kind).await;
        assert!(matches!(busy, Err(TaskError::SpendInProgress)), "{busy:?}");
        let in_flight_key = backend.platform_shield_quote_revision(&seed_hash);
        assert_ne!(in_flight_key, idle_key, "opening a shield moves the key");
        assert_eq!(
            backend.core_send_quote_revision(&seed_hash),
            core_key,
            "a shield from Platform reserves no Core input"
        );
        drop(shielding);

        assert_ne!(
            backend.platform_shield_quote_revision(&seed_hash),
            in_flight_key,
            "settling a shield moves the key"
        );
        let settled = backend.spend_limit_credits(&seed_hash, kind).await;
        assert!(
            !matches!(settled, Err(TaskError::SpendInProgress)),
            "{settled:?}"
        );
    }

    /// After a spend from Platform addresses a sync pass runs, so the shield
    /// planner and DET's per-address balances stop showing the spent credits
    /// and the next shield Max is quoted from the new balances.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_spend_from_platform_addresses_runs_an_address_sync_pass() {
        let (_dir, _ctx, backend, _seed_hash, _platform_wallet) =
            backend_with_registered_wallet().await;
        let coordinator = backend.inner.pwm.platform_address_sync_arc();
        assert_eq!(coordinator.last_sync_unix_seconds(), None, "precondition");

        backend
            .sync_platform_addresses_after_spend(std::time::Duration::from_secs(60))
            .await;

        assert!(
            coordinator.last_sync_unix_seconds().is_some(),
            "no Platform-address sync pass ran after the spend"
        );
    }

    /// The Core send Max quoted for a wallet whose funds sit only in
    /// `account_type`, as the error it is refused with.
    async fn core_send_max_refusal(
        account_type: dash_sdk::dpp::key_wallet::account::AccountType,
        values: &[u64],
    ) -> Result<u64, TaskError> {
        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(&platform_wallet, account_type, 0x73, values).await;
        backend
            .spend_limit_credits(
                &seed_hash,
                crate::model::spend_limit::SpendLimitKind::CoreSend {
                    destination: Default::default(),
                },
            )
            .await
    }

    /// Funds held only in CoinJoin are not "too low for the fee": the form
    /// must say they cannot be sent from here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_names_coinjoin_only_funds_as_nothing_spendable() {
        use crate::model::spend_limit::CoreSendShortfall;
        use dash_sdk::dpp::key_wallet::account::AccountType;
        let coinjoin = AccountType::CoinJoin { index: 0 };
        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        {
            // CoinJoin accounts have no receive chain; the coin's address
            // does not decide where a send may spend it from.
            let wallet_id = platform_wallet.wallet_id();
            let mut manager = platform_wallet.wallet_manager().write().await;
            let info = manager
                .get_wallet_info_mut(&wallet_id)
                .expect("wallet info");
            info.core_wallet.update_last_processed_height(200);
            let managed = info
                .core_wallet
                .accounts
                .funds_account_mut(&coinjoin)
                .expect("CoinJoin account");
            let outpoint = OutPoint::new(Txid::from_byte_array([0x77; 32]), 0);
            let mut utxo = Utxo::new(
                outpoint,
                TxOut {
                    value: 5_000_000,
                    script_pubkey: payment_recipient().script_pubkey(),
                },
                payment_recipient(),
                100,
                false,
            );
            utxo.is_confirmed = true;
            managed.utxos.insert(outpoint, utxo);
        }
        let quote = backend
            .spend_limit_credits(
                &seed_hash,
                crate::model::spend_limit::SpendLimitKind::CoreSend {
                    destination: Default::default(),
                },
            )
            .await;
        assert!(
            matches!(
                quote,
                Err(TaskError::CoreSendMaxUnavailable {
                    reason: CoreSendShortfall::NothingSpendable
                })
            ),
            "{quote:?}"
        );
    }

    /// Coins too small to pay for their own input leave a fee shortfall.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_names_dust_only_funds_as_below_the_fee() {
        use crate::model::spend_limit::CoreSendShortfall;
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        let bip44 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        };
        let quote = core_send_max_refusal(bip44, &[100, 120]).await;
        assert!(
            matches!(
                quote,
                Err(TaskError::CoreSendMaxUnavailable {
                    reason: CoreSendShortfall::BelowFee
                })
            ),
            "{quote:?}"
        );
    }

    /// When every spendable coin is held by a build the wallet has not
    /// finished, the form says so instead of blaming the fee.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_names_reserved_funds_as_held() {
        use crate::model::spend_limit::{CoreSendShortfall, SpendLimitKind};
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        let bip44 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        };
        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(&platform_wallet, bip44, 0x74, &[2_000_000]).await;
        {
            let wallet_id = platform_wallet.wallet_id();
            let mut manager = platform_wallet.wallet_manager().write().await;
            let account = manager
                .get_wallet_and_info(&wallet_id)
                .and_then(|(key_wallet, _)| key_wallet.accounts.account_of_type(bip44))
                .expect("funding account keys")
                .clone();
            let managed = manager
                .get_wallet_info_mut(&wallet_id)
                .and_then(|info| info.core_wallet.accounts.funds_account_mut(&bip44))
                .expect("managed funding account");
            let (_, _, reservation) = TransactionBuilder::new()
                .set_current_height(200)
                .set_selection_strategy(SelectionStrategy::LargestFirst)
                .add_output(&payment_recipient(), 1_000_000)
                .add_funding(managed, &account)
                .build_unsigned_reserved()
                .expect("in-flight build");
            assert!(reservation.is_some(), "the in-flight build holds the coin");
        }

        let quote = backend
            .spend_limit_credits(
                &seed_hash,
                SpendLimitKind::CoreSend {
                    destination: Default::default(),
                },
            )
            .await;
        assert!(
            matches!(
                quote,
                Err(TaskError::CoreSendMaxUnavailable {
                    reason: CoreSendShortfall::HeldByPendingTransaction
                })
            ),
            "{quote:?}"
        );
    }

    /// A search that ran out of time before proving any amount is not a
    /// zero Max; one that proved an amount keeps it.
    #[test]
    fn a_timed_out_core_send_search_with_nothing_proven_is_not_a_zero_max() {
        assert!(matches!(
            super::core_send_search_outcome(0, true),
            Err(TaskError::SpendLimitTimedOut)
        ));
        assert!(matches!(
            super::core_send_search_outcome(5_000, true),
            Ok(5_000)
        ));
    }

    /// Wallet events that leave the coins a send can spend unchanged (a new
    /// block, a chainlock) must not re-key the Core Max quote; a new coin
    /// must.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_quote_revision_follows_the_coins_not_the_events() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        let bip44 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        };
        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        let wallet_id = platform_wallet.wallet_id();
        fund_account(&platform_wallet, bip44, 0x75, &[1_000_000]).await;
        backend.inner.snapshots.recompute(&wallet_id);
        let funded = backend.core_send_quote_revision(&seed_hash);

        backend.inner.snapshots.recompute(&wallet_id);
        assert_eq!(
            backend.core_send_quote_revision(&seed_hash),
            funded,
            "a republish of the same coins keeps the quote"
        );

        fund_account(&platform_wallet, bip44, 0x76, &[2_000_000]).await;
        backend.inner.snapshots.recompute(&wallet_id);
        assert_ne!(
            backend.core_send_quote_revision(&seed_hash),
            funded,
            "a new coin re-keys the quote"
        );
    }

    /// While an operation that can reserve Core inputs is in flight, its
    /// reservation lands at an instant no quote can see, so neither Max is
    /// quoted: both report the operation instead of a number, without a dry
    /// run. Once it ends, both quote again under a new key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_max_is_quoted_while_a_reserving_operation_is_in_flight() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x72,
            &[1_000_000],
        )
        .await;
        let core_kind = crate::model::spend_limit::SpendLimitKind::CoreSend {
            destination: Default::default(),
        };

        let spend = backend.begin_core_spend(&seed_hash);
        let core = backend.spend_limit_credits(&seed_hash, core_kind).await;
        assert!(matches!(core, Err(TaskError::SpendInProgress)), "{core:?}");
        let lock = backend.asset_lock_max_amount(&seed_hash).await;
        assert!(
            matches!(lock, Err(TaskError::SpendInProgress)),
            "{:?}",
            lock.map(|quote| quote.amount_duffs)
        );
        let in_flight_key = backend.core_send_quote_revision(&seed_hash);
        drop(spend);

        assert_ne!(backend.core_send_quote_revision(&seed_hash), in_flight_key);
        let core = backend
            .spend_limit_credits(&seed_hash, core_kind)
            .await
            .expect("Core Max after the operation");
        assert!(core > 0, "{core}");
        let lock = backend
            .asset_lock_max_amount(&seed_hash)
            .await
            .expect("asset-lock Max after the operation");
        assert!(lock.amount_duffs > 0, "{}", lock.amount_duffs);
    }

    /// Busy withholds only the Max quotes: an explicit amount started while
    /// another reserving operation is open still gets the reservation-aware
    /// dry run, not `SpendInProgress`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_explicit_amount_is_checked_while_a_reserving_operation_is_in_flight() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x74,
            &[1_000_000],
        )
        .await;
        let _other_operation = backend.begin_core_spend(&seed_hash);

        backend
            .ensure_asset_lock_amount_within_input_cap(&seed_hash, 500_000)
            .await
            .expect("an affordable explicit asset lock passes");
        backend
            .ensure_core_payment_builds(&seed_hash, &[(payment_recipient(), 500_000)])
            .await
            .expect("an affordable explicit payment passes");
        let short = backend
            .ensure_asset_lock_amount_within_input_cap(&seed_hash, 5_000_000)
            .await;
        assert!(
            matches!(short, Err(TaskError::InsufficientFunds { .. })),
            "{short:?}"
        );
    }

    /// A P2SH output is two bytes smaller than a P2PKH one, so the payment
    /// to a P2SH address can be larger by that much fee. Its Max is quoted
    /// for that script and is exactly the largest amount the build accepts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_is_quoted_for_the_destination_script() {
        use crate::model::spend_limit::{CoreDestinationScript, SpendLimitKind};
        use dash_sdk::dpp::dashcore::hashes::Hash;
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x73,
            &[1_000_000],
        )
        .await;
        let quote = |destination| {
            let backend = backend.clone();
            async move {
                backend
                    .spend_limit_credits(&seed_hash, SpendLimitKind::CoreSend { destination })
                    .await
                    .expect("Core send maximum")
                    / dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF
            }
        };
        let p2pkh_max = quote(CoreDestinationScript::PubkeyHash).await;
        let p2sh_max = quote(CoreDestinationScript::ScriptHash).await;
        assert!(p2sh_max > p2pkh_max, "{p2sh_max} <= {p2pkh_max}");

        let p2sh = Address::new(
            Network::Testnet,
            dash_sdk::dpp::dashcore::address::Payload::ScriptHash(
                dash_sdk::dpp::dashcore::ScriptHash::from_byte_array(rand::random()),
            ),
        );
        backend
            .ensure_core_payment_builds(&seed_hash, &[(p2sh.clone(), p2sh_max)])
            .await
            .expect("the P2SH Max must build");
        let over = backend
            .ensure_core_payment_builds(&seed_hash, &[(p2sh, p2sh_max + 1)])
            .await;
        assert!(
            matches!(over, Err(TaskError::InsufficientFunds { .. })),
            "one duff above the P2SH Max must be a shortfall: {over:?}"
        );
    }

    /// An exact asset lock the wallet cannot fund is refused by the guard,
    /// before any secret is opened, with the balance advice a shortfall
    /// deserves — not left to fail inside the build after the unlock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unaffordable_asset_lock_is_refused_before_unlocking() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        const FUNDED_DUFFS: u64 = 1_000_000;
        const AMOUNT_DUFFS: u64 = 5_000_000;

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x70,
            &[FUNDED_DUFFS],
        )
        .await;

        let refused = backend
            .ensure_asset_lock_amount_within_input_cap(&seed_hash, AMOUNT_DUFFS)
            .await;

        assert!(
            matches!(
                refused,
                Err(TaskError::InsufficientFunds { available, required })
                    if available <= FUNDED_DUFFS && required >= AMOUNT_DUFFS
            ),
            "{refused:?}"
        );
    }

    /// A password prompt that, while the "user" types, spends one UTXO from
    /// the wallet — the wallet changing between the pre-check and the build.
    struct SpendWhilePrompting {
        wallet: std::sync::OnceLock<std::sync::Arc<platform_wallet::PlatformWallet>>,
        account_type: dash_sdk::dpp::key_wallet::account::AccountType,
        spent: OutPoint,
        password: String,
        asks: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl crate::wallet_backend::secret_prompt::SecretPrompt for SpendWhilePrompting {
        async fn request(
            &self,
            _request: crate::wallet_backend::secret_prompt::SecretPromptRequest,
        ) -> Result<
            crate::wallet_backend::secret_prompt::SecretPromptReply,
            crate::wallet_backend::secret_prompt::SecretPromptCancelled,
        > {
            self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let wallet = self.wallet.get().expect("wallet wired before the prompt");
            let wallet_id = wallet.wallet_id();
            let mut manager = wallet.wallet_manager().write().await;
            let removed = manager
                .get_wallet_info_mut(&wallet_id)
                .and_then(|info| {
                    info.core_wallet
                        .accounts
                        .funds_account_mut(&self.account_type)
                })
                .and_then(|account| account.utxos.remove(&self.spent));
            assert!(removed.is_some(), "the prompt must spend the large UTXO");
            Ok(
                crate::wallet_backend::secret_prompt::SecretPromptReply::new(
                    platform_wallet_storage::secrets::SecretString::new(self.password.clone()),
                    crate::wallet_backend::secret_prompt::RememberPolicy::None,
                ),
            )
        }

        fn is_interactive(&self) -> bool {
            true
        }
    }

    /// The cap pre-check runs before the password prompt so a doomed lock
    /// never asks for it. The wallet can change while the user types, so
    /// the cap is checked again right before the build: an amount that now
    /// needs more inputs than an asset lock may carry is refused, never
    /// built and broadcast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_asset_lock_pushed_over_the_input_cap_while_prompting_is_refused() {
        use crate::app::TaskResult;
        use crate::context::test_support::test_app_context;
        use crate::utils::egui_mpsc::SenderAsync;
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        const SMALL_DUFFS: u64 = 100_000;
        const LARGE_DUFFS: u64 = 10_000_000;
        // With the large UTXO about 22 inputs; without it about 121.
        const AMOUNT_DUFFS: u64 = 12_000_000;
        let bip44 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        };

        // Generated per run: no wallet secret is ever committed.
        let password = hex::encode(rand::random::<[u8; 16]>());
        let seed: [u8; 64] = rand::random();
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let prompt = std::sync::Arc::new(SpendWhilePrompting {
            wallet: std::sync::OnceLock::new(),
            account_type: bip44,
            spent: OutPoint::new(Txid::from_byte_array([0x68; 32]), 0),
            password: password.clone(),
            asks: std::sync::atomic::AtomicUsize::new(0),
        });
        app_context.install_secret_prompt(prompt.clone());
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(16);
        app_context
            .ensure_wallet_backend(SenderAsync::new(sender, app_context.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = app_context.wallet_backend().expect("wired backend");
        let seed_hash = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("Cap race test".to_string()),
            None,
        )
        .expect("DET wallet")
        .seed_hash();
        backend
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register wallet");
        crate::wallet_backend::wallet_seed_store::WalletSeedView::new(&app_context.secret_store())
            .set_protected(
                &seed_hash,
                &seed,
                &platform_wallet_storage::secrets::SecretString::new(password),
            )
            .expect("protected seed");
        let platform_wallet = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("platform wallet");
        prompt
            .wallet
            .set(std::sync::Arc::clone(&platform_wallet))
            .unwrap_or_else(|_| panic!("wallet set once"));
        fund_account(&platform_wallet, bip44, 0x68, &[LARGE_DUFFS]).await;
        fund_account(&platform_wallet, bip44, 0x69, &[SMALL_DUFFS; 150]).await;
        backend
            .ensure_asset_lock_amount_within_input_cap(&seed_hash, AMOUNT_DUFFS)
            .await
            .expect("precondition: the amount fits the cap before the prompt");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            backend.create_asset_lock_proof(
                &seed_hash,
                AMOUNT_DUFFS,
                platform_wallet::AssetLockFundingType::AssetLockAddressTopUp,
                0,
            ),
        )
        .await
        .expect("the refusal must not wait on the network");

        assert_eq!(
            prompt.asks.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the pre-check passed, so the password was asked once"
        );
        assert!(
            matches!(
                result,
                Err(TaskError::AssetLockTooManyInputs { count, max: 100 }) if count > 100
            ),
            "{result:?}"
        );
    }

    fn payment_recipient() -> Address {
        Address::p2pkh(
            &PublicKey::from_slice(&[0x02; 33]).expect("valid compressed public key"),
            Network::Testnet,
        )
    }

    /// Core sends pool BIP-44, BIP-32 and DashPay receiving funds, so Max
    /// covers all of them and is exactly the largest amount the pooled
    /// payment build accepts: one duff more is refused as a shortfall.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_is_the_largest_amount_the_pooled_build_accepts() {
        use crate::model::spend_limit::SpendLimitKind;
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            0x68,
            &[700_000, 1_234_567],
        )
        .await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP32Account,
            },
            0x69,
            // The 100-duff output costs more to spend than it brings.
            &[500_000, 100],
        )
        .await;

        let max_credits = backend
            .spend_limit_credits(
                &seed_hash,
                SpendLimitKind::CoreSend {
                    destination: Default::default(),
                },
            )
            .await
            .expect("Core send maximum");
        assert_eq!(max_credits % CREDITS_PER_DUFF, 0);
        let max_duffs = max_credits / CREDITS_PER_DUFF;
        assert!(
            max_duffs > 2_400_000 && max_duffs < 2_434_567,
            "Max must cover the BIP-44 and BIP-32 funds, less the fee: {max_duffs}"
        );

        backend
            .ensure_core_payment_builds(&seed_hash, &[(payment_recipient(), max_duffs)])
            .await
            .expect("the quoted Max must build");
        let over = backend
            .ensure_core_payment_builds(&seed_hash, &[(payment_recipient(), max_duffs + 1)])
            .await;
        assert!(
            matches!(over, Err(TaskError::InsufficientFunds { .. })),
            "one duff above Max must be a shortfall: {over:?}"
        );
    }

    /// Inputs reserved by another in-flight build cannot fund this send, so
    /// Max is what the reservation-aware pooled build accepts now — not
    /// upstream's quote, which counts reserved inputs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_leaves_out_inputs_reserved_by_another_build() {
        use crate::model::spend_limit::SpendLimitKind;
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        const RESERVED_DUFFS: u64 = 1_234_567;
        const FREE_DUFFS: u64 = 700_000;
        let bip44 = AccountType::Standard {
            index: 0,
            standard_account_type: StandardAccountType::BIP44Account,
        };

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(&platform_wallet, bip44, 0x6f, &[FREE_DUFFS, RESERVED_DUFFS]).await;
        // Another build takes the large input and has not broadcast yet.
        {
            let wallet_id = platform_wallet.wallet_id();
            let mut manager = platform_wallet.wallet_manager().write().await;
            let account = manager
                .get_wallet_and_info(&wallet_id)
                .and_then(|(key_wallet, _)| key_wallet.accounts.account_of_type(bip44))
                .expect("funding account keys")
                .clone();
            let managed = manager
                .get_wallet_info_mut(&wallet_id)
                .and_then(|info| info.core_wallet.accounts.funds_account_mut(&bip44))
                .expect("managed funding account");
            let (_, _, reservation) = TransactionBuilder::new()
                .set_current_height(200)
                .set_selection_strategy(SelectionStrategy::LargestFirst)
                .add_output(&payment_recipient(), 1_000_000)
                .add_funding(managed, &account)
                .build_unsigned_reserved()
                .expect("in-flight build");
            assert!(reservation.is_some(), "the in-flight build holds its input");
        }

        let max_duffs = backend
            .spend_limit_credits(
                &seed_hash,
                SpendLimitKind::CoreSend {
                    destination: Default::default(),
                },
            )
            .await
            .expect("Core send maximum")
            / dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;

        assert!(
            max_duffs > 0 && max_duffs < FREE_DUFFS,
            "Max must come from the unreserved input only: {max_duffs}"
        );
        backend
            .ensure_core_payment_builds(&seed_hash, &[(payment_recipient(), max_duffs)])
            .await
            .expect("the quoted Max must build around the reservation");
        let over = backend
            .ensure_core_payment_builds(&seed_hash, &[(payment_recipient(), max_duffs + 1)])
            .await;
        assert!(
            matches!(over, Err(TaskError::InsufficientFunds { .. })),
            "one duff above Max must be a shortfall: {over:?}"
        );
    }

    /// The quoted Max survives the real pooled finalize — signing included —
    /// and only the broadcast, with no network here, is what stops it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_send_max_is_signed_by_the_pooled_finalizer() {
        use crate::model::spend_limit::SpendLimitKind;
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, ctx, backend) = backend_without_wallets().await;
        // Registered through the app, so the seed is in the vault and the
        // signer can open it without a prompt.
        // Generated per run: no wallet secret is ever committed.
        let seed: [u8; 64] = rand::random();
        let wallet = crate::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("Signing test".to_string()),
            None,
        )
        .expect("DET wallet");
        let (seed_hash, _) = ctx
            .register_wallet(
                wallet,
                &seed,
                crate::model::wallet::birth_height::WalletOrigin::Fresh,
            )
            .expect("register wallet");
        // Offline registration with the backend is slow; wait for it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !backend.is_wallet_registered(&seed_hash) {
            assert!(
                std::time::Instant::now() < deadline,
                "the wallet must register with the backend"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let platform_wallet = backend
            .resolve_wallet(&seed_hash)
            .await
            .expect("platform wallet");
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP32Account,
            },
            0x6d,
            &[1_000_000, 250_000],
        )
        .await;
        let max_duffs = backend
            .spend_limit_credits(
                &seed_hash,
                SpendLimitKind::CoreSend {
                    destination: Default::default(),
                },
            )
            .await
            .expect("Core send maximum")
            / CREDITS_PER_DUFF;

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            backend.send_payment(&seed_hash, vec![(payment_recipient(), max_duffs)]),
        )
        .await
        .expect("an offline send settles");
        assert!(
            matches!(
                &result,
                Err(TaskError::WalletBackend { source })
                    if matches!(
                        source.as_ref(),
                        platform_wallet::error::PlatformWalletError::TransactionBroadcast(_)
                    )
            ),
            "the pooled finalize must build and sign Max; only the broadcast may fail: {result:?}"
        );
    }

    /// A payment no pooled build can fund is refused before any secret is
    /// opened, with balance advice rather than a generic failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unfundable_payment_is_refused_before_signing() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let (_dir, _ctx, backend, seed_hash, platform_wallet) =
            backend_with_registered_wallet().await;
        fund_account(
            &platform_wallet,
            AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP32Account,
            },
            0x6b,
            &[1_000_000],
        )
        .await;

        let result = backend
            .send_payment(&seed_hash, vec![(payment_recipient(), 2_000_000)])
            .await;
        assert!(
            matches!(result, Err(TaskError::InsufficientFunds { .. })),
            "{result:?}"
        );
    }
}
