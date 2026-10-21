//! Per-wallet balance figures behind wallet selectors.
//!
//! Every balance kind is held in credits, the finest unit any kind uses
//! (1 duff = 1000 credits). Kinds are added in credits and rounded down to
//! duffs once, so sub-duff remainders of several kinds are not lost one by one.
//! A figure given in duffs has no remainder left to add: the Platform address
//! balance arrives in whole duffs, so a total counting it can be one duff
//! below the exact sum.

use crate::model::address::AddressKind;
use crate::model::wallet::single_key::SingleKeyHash;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::wallet_backend::poison::RwLockRecover;
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits, Duffs};
use std::sync::RwLock;

/// A wallet a selector can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WalletChoice {
    /// A recovery-phrase wallet, by seed hash.
    Hd(WalletSeedHash),
    /// An imported single-key wallet, by key hash.
    SingleKey(SingleKeyHash),
}

impl WalletChoice {
    /// The choice standing for a loaded recovery-phrase wallet.
    pub fn of_hd_wallet(wallet: &RwLock<Wallet>) -> Self {
        Self::Hd(wallet.read_recover().seed_hash())
    }
}

/// Which Core figure a selector counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CoreFigure {
    /// Everything the wallet holds, including funds that cannot be spent yet.
    #[default]
    Total,
    /// Confirmed or instantly locked funds that funding an identity can spend
    /// now, before the network fee.
    Usable,
}

/// One wallet's balances by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalletBalanceSummary {
    core_total: Credits,
    core_usable: Credits,
    platform: Credits,
    shielded: Credits,
    identities: Credits,
}

impl WalletBalanceSummary {
    /// Set both Core figures, given in duffs.
    pub fn with_core_duffs(mut self, total: Duffs, usable: Duffs) -> Self {
        self.core_total = total.saturating_mul(CREDITS_PER_DUFF);
        self.core_usable = usable.saturating_mul(CREDITS_PER_DUFF);
        self
    }

    /// Set the Platform address balance, given in whole duffs; any sub-duff
    /// remainder is already gone.
    pub fn with_platform_duffs(mut self, duffs: Duffs) -> Self {
        self.platform = duffs.saturating_mul(CREDITS_PER_DUFF);
        self
    }

    /// Set the shielded balance, given in credits.
    pub fn with_shielded_credits(mut self, credits: Credits) -> Self {
        self.shielded = credits;
        self
    }

    /// Set the summed balance of the wallet's identities, given in credits.
    pub fn with_identity_credits(mut self, credits: Credits) -> Self {
        self.identities = credits;
        self
    }

    fn kind_credits(&self, kind: AddressKind, core: CoreFigure) -> Credits {
        match (kind, core) {
            (AddressKind::Core, CoreFigure::Total) => self.core_total,
            (AddressKind::Core, CoreFigure::Usable) => self.core_usable,
            (AddressKind::Platform, _) => self.platform,
            (AddressKind::Shielded, _) => self.shielded,
            (AddressKind::Identity, _) => self.identities,
        }
    }

    /// One kind's balance in duffs, rounded down.
    pub fn kind_duffs(&self, kind: AddressKind, core: CoreFigure) -> Duffs {
        self.kind_credits(kind, core) / CREDITS_PER_DUFF
    }

    /// The sum of `kinds` in duffs: added in credits, rounded down once.
    /// A kind listed twice counts once.
    pub fn total_duffs(&self, kinds: &[AddressKind], core: CoreFigure) -> Duffs {
        AddressKind::ALL
            .into_iter()
            .filter(|kind| kinds.contains(kind))
            .fold(0, |sum: Credits, kind| {
                sum.saturating_add(self.kind_credits(kind, core))
            })
            / CREDITS_PER_DUFF
    }
}

/// A balance as a wallet selector shows it: DASH with exactly four decimal
/// places, zero-padded. Finer digits are cut off, never rounded up, so the
/// text cannot show more than the wallet holds; an amount below 0.0001 DASH
/// reads as `0.0000 DASH`.
pub fn format_selector_balance(duffs: Duffs) -> String {
    /// Duffs in 0.0001 DASH, and such steps in one DASH.
    const STEP: Duffs = 10_000;
    let steps = duffs / STEP;
    format!("{}.{:04} DASH", steps / STEP, steps % STEP)
}

/// One selector row: which wallet, its name, and its balances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletBalanceEntry {
    pub choice: WalletChoice,
    /// The user's name for the wallet, when it has one.
    pub name: Option<String>,
    pub balances: WalletBalanceSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> WalletBalanceSummary {
        WalletBalanceSummary::default()
            .with_core_duffs(1_000, 400)
            .with_platform_duffs(200)
            .with_shielded_credits(30_000)
            .with_identity_credits(5_000)
    }

    #[test]
    fn total_counts_only_the_requested_kinds() {
        let s = summary();
        assert_eq!(
            s.total_duffs(&[AddressKind::Core], CoreFigure::Total),
            1_000
        );
        assert_eq!(
            s.total_duffs(&[AddressKind::Platform], CoreFigure::Total),
            200
        );
        assert_eq!(
            s.total_duffs(&[AddressKind::Shielded], CoreFigure::Total),
            30
        );
        assert_eq!(
            s.total_duffs(&[AddressKind::Identity], CoreFigure::Total),
            5
        );
        assert_eq!(s.total_duffs(&AddressKind::ALL, CoreFigure::Total), 1_235);
    }

    #[test]
    fn core_figure_picks_total_or_usable_funds() {
        let s = summary();
        assert_eq!(s.total_duffs(&[AddressKind::Core], CoreFigure::Usable), 400);
        assert_eq!(s.kind_duffs(AddressKind::Core, CoreFigure::Usable), 400);
        assert_eq!(s.kind_duffs(AddressKind::Core, CoreFigure::Total), 1_000);
        assert_eq!(s.total_duffs(&AddressKind::ALL, CoreFigure::Usable), 635);
    }

    #[test]
    fn kinds_are_added_in_credits_before_rounding_down_to_duffs() {
        let s = WalletBalanceSummary::default()
            .with_shielded_credits(600)
            .with_identity_credits(600);
        assert_eq!(s.kind_duffs(AddressKind::Shielded, CoreFigure::Total), 0);
        assert_eq!(s.kind_duffs(AddressKind::Identity, CoreFigure::Total), 0);
        assert_eq!(
            s.total_duffs(
                &[AddressKind::Shielded, AddressKind::Identity],
                CoreFigure::Total
            ),
            1,
            "two half-duff remainders add up to a whole duff"
        );
    }

    #[test]
    fn a_kind_listed_twice_counts_once() {
        let s = summary();
        assert_eq!(
            s.total_duffs(
                &[AddressKind::Platform, AddressKind::Platform],
                CoreFigure::Total
            ),
            200
        );
    }

    #[test]
    fn no_kinds_means_a_zero_total() {
        assert_eq!(summary().total_duffs(&[], CoreFigure::Total), 0);
    }

    #[test]
    fn huge_balances_saturate_instead_of_overflowing() {
        let s = WalletBalanceSummary::default()
            .with_core_duffs(u64::MAX, u64::MAX)
            .with_platform_duffs(u64::MAX)
            .with_shielded_credits(u64::MAX);
        assert_eq!(
            s.total_duffs(&AddressKind::ALL, CoreFigure::Total),
            u64::MAX / CREDITS_PER_DUFF
        );
    }

    #[test]
    fn selector_balance_is_padded_to_four_decimal_places() {
        assert_eq!(format_selector_balance(50_000_000), "0.5000 DASH");
        assert_eq!(format_selector_balance(1_200_000_000), "12.0000 DASH");
        assert_eq!(format_selector_balance(0), "0.0000 DASH");
    }

    /// The text must never show more than the wallet holds.
    #[test]
    fn selector_balance_cuts_finer_digits_off_instead_of_rounding_up() {
        assert_eq!(format_selector_balance(12_349_999), "0.1234 DASH");
        assert_eq!(format_selector_balance(99_999_999), "0.9999 DASH");
    }

    #[test]
    fn selector_balance_below_the_smallest_step_reads_as_zero() {
        assert_eq!(format_selector_balance(9_999), "0.0000 DASH");
        assert_eq!(format_selector_balance(10_000), "0.0001 DASH");
    }

    #[test]
    fn selector_balance_of_a_large_amount_keeps_every_whole_digit() {
        assert_eq!(
            format_selector_balance(2_100_000_012_345_678),
            "21000000.1234 DASH"
        );
        assert_eq!(format_selector_balance(u64::MAX), "184467440737.0955 DASH");
    }
}
