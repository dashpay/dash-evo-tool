//! Pure per-asset balance consistency check.
//!
//! A wallet's total for an asset can be computed two independent ways: the
//! wallet backend's authoritative live total, and the sum of the per-category
//! account view. When those disagree beyond a sub-unit rounding slop the
//! per-category view has missed some funded state. This module provides the
//! pure comparison; it holds no state and performs no IO. The end-of-sync
//! health check (`AppState`) drives it once per SPV sync completion.

/// Tolerance, in duffs, for the balance consistency check.
///
/// Absorbs the documented ≤1-duff sum-then-truncate rounding slop between the
/// authoritative total and the per-category total (see `event_bridge.rs`,
/// `QA-B2-001`): the per-category Platform figure truncates each address group
/// to whole duffs, so the two totals can legitimately differ by up to one duff.
pub const BALANCE_MISMATCH_TOLERANCE_DUFFS: u64 = 1;

/// Which asset's totals disagreed. Its [`label`](BalanceAsset::label) is the
/// user-facing name shared with the balance-breakdown UI and the log records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BalanceAsset {
    Core,
    Platform,
}

impl BalanceAsset {
    /// User-facing name, matching the "Balance breakdown" labels.
    pub fn label(self) -> &'static str {
        match self {
            BalanceAsset::Core => "Core",
            BalanceAsset::Platform => "Platform",
        }
    }
}

/// A detected disagreement between the authoritative backend total and the
/// total summed from the per-category account view.
///
/// Both totals are in the same unit (duffs). The difference is derivable via
/// [`BalanceMismatch::diff`], so it is not stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BalanceMismatch {
    /// Authoritative total reported by the wallet backend's live state.
    pub backend_total: u64,
    /// Total summed across the per-category account view.
    pub categorized_total: u64,
}

impl BalanceMismatch {
    /// Absolute difference between the two totals, in duffs.
    pub fn diff(&self) -> u64 {
        self.backend_total.abs_diff(self.categorized_total)
    }
}

/// Flag a mismatch when the backend total and the categorized total diverge by
/// more than `tolerance`.
///
/// Returns `None` when the two totals agree within `tolerance` — the expected
/// case, which absorbs the documented sub-duff rounding slop. The comparison is
/// symmetric: a categorized total exceeding the backend total is also flagged,
/// since the categorized figure is a subset of backend truth by construction
/// and should never legitimately be larger.
pub fn detect_mismatch(
    backend_total: u64,
    categorized_total: u64,
    tolerance: u64,
) -> Option<BalanceMismatch> {
    if backend_total.abs_diff(categorized_total) > tolerance {
        Some(BalanceMismatch {
            backend_total,
            categorized_total,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: u64 = BALANCE_MISMATCH_TOLERANCE_DUFFS;

    #[test]
    fn both_zero_is_no_mismatch() {
        assert_eq!(detect_mismatch(0, 0, TOL), None);
    }

    #[test]
    fn exact_match_is_no_mismatch() {
        assert_eq!(detect_mismatch(1_000, 1_000, TOL), None);
    }

    #[test]
    fn difference_at_tolerance_is_no_mismatch() {
        assert_eq!(detect_mismatch(1_001, 1_000, TOL), None);
        assert_eq!(detect_mismatch(1_000, 1_001, TOL), None);
    }

    #[test]
    fn just_over_tolerance_is_flagged() {
        let mismatch = detect_mismatch(1_002, 1_000, TOL).expect("should flag");
        assert_eq!(mismatch.backend_total, 1_002);
        assert_eq!(mismatch.categorized_total, 1_000);
        assert_eq!(mismatch.diff(), 2);
    }

    #[test]
    fn large_backend_surplus_is_flagged() {
        // The real-world case: categorized view lags far behind backend truth.
        let mismatch = detect_mismatch(7_562_000_000, 216_000_000, TOL).expect("should flag");
        assert_eq!(mismatch.diff(), 7_346_000_000);
    }

    #[test]
    fn categorized_exceeding_backend_is_flagged_defensively() {
        let mismatch = detect_mismatch(1_000, 5_000, TOL).expect("should flag");
        assert_eq!(mismatch.diff(), 4_000);
    }

    #[test]
    fn zero_tolerance_flags_any_difference() {
        assert_eq!(detect_mismatch(1_000, 1_000, 0), None);
        assert!(detect_mismatch(1_001, 1_000, 0).is_some());
    }

    #[test]
    fn asset_labels_match_breakdown_wording() {
        assert_eq!(BalanceAsset::Core.label(), "Core");
        assert_eq!(BalanceAsset::Platform.label(), "Platform");
    }
}
