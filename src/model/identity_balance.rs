//! Ordering of identity credit balance observations by Platform block height.
//!
//! A stored identity's balance can be written by many paths: a write's proof,
//! a fetch, a screen saving a copy it read earlier. Each verified observation
//! is a snapshot at the block height of its response, and the newest snapshot
//! wins: an older one never replaces a newer balance. Mirrors the rule
//! `platform-wallet` applies to the balances it keeps for wallet identities.

use dash_sdk::dpp::balances::credits::Credits;
use serde::{Deserialize, Serialize};

/// The newest height-tagged balance applied to a stored identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceWatermark {
    pub credits: Credits,
    /// Platform block height the balance is a snapshot at.
    pub height: u64,
}

/// Where the balance an identity write carries comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceProvenance {
    /// Verified by a Platform response at this block height.
    ObservedAt(u64),
    /// Reported by a completed write whose response height is not available.
    /// Taken as current, but it cannot advance the height a later
    /// observation is ordered against.
    ConfirmedUnordered,
    /// Not a new observation: a copy of a balance read earlier.
    Unobserved,
}

/// What [`reconcile_balance`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciledBalance {
    /// The balance to store.
    pub credits: Credits,
    /// The watermark to store; `None` leaves the stored one as it is.
    pub watermark: Option<BalanceWatermark>,
}

/// The balance to keep when a write carrying `incoming` credits from
/// `provenance` meets the `stored` watermark.
///
/// Only a strictly higher height replaces a watermark: two responses at one
/// height cannot be ordered, so the first one stays. Without a watermark every
/// balance is accepted, as before heights were tracked.
pub fn reconcile_balance(
    stored: Option<BalanceWatermark>,
    incoming: Credits,
    provenance: BalanceProvenance,
) -> ReconciledBalance {
    let accept = |height: Option<u64>| ReconciledBalance {
        credits: incoming,
        watermark: height.map(|height| BalanceWatermark {
            credits: incoming,
            height,
        }),
    };
    let Some(stored) = stored else {
        return match provenance {
            BalanceProvenance::ObservedAt(height) => accept(Some(height)),
            BalanceProvenance::ConfirmedUnordered | BalanceProvenance::Unobserved => accept(None),
        };
    };
    match provenance {
        BalanceProvenance::ObservedAt(height) if height > stored.height => accept(Some(height)),
        BalanceProvenance::ConfirmedUnordered => accept(Some(stored.height)),
        BalanceProvenance::ObservedAt(_) | BalanceProvenance::Unobserved => ReconciledBalance {
            credits: stored.credits,
            watermark: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORED: BalanceWatermark = BalanceWatermark {
        credits: 1_000,
        height: 50,
    };

    fn kept(credits: Credits) -> ReconciledBalance {
        ReconciledBalance {
            credits,
            watermark: None,
        }
    }

    fn advanced(credits: Credits, height: u64) -> ReconciledBalance {
        ReconciledBalance {
            credits,
            watermark: Some(BalanceWatermark { credits, height }),
        }
    }

    #[test]
    fn a_newer_observation_replaces_the_watermark() {
        assert_eq!(
            reconcile_balance(Some(STORED), 700, BalanceProvenance::ObservedAt(51)),
            advanced(700, 51)
        );
    }

    #[test]
    fn an_older_observation_keeps_the_newer_balance() {
        assert_eq!(
            reconcile_balance(Some(STORED), 1_200, BalanceProvenance::ObservedAt(49)),
            kept(1_000)
        );
    }

    #[test]
    fn an_observation_at_the_same_height_keeps_the_first_balance() {
        assert_eq!(
            reconcile_balance(Some(STORED), 900, BalanceProvenance::ObservedAt(50)),
            kept(1_000)
        );
    }

    #[test]
    fn an_unobserved_copy_never_replaces_a_watermarked_balance() {
        assert_eq!(
            reconcile_balance(Some(STORED), 5, BalanceProvenance::Unobserved),
            kept(1_000)
        );
    }

    #[test]
    fn a_confirmed_balance_without_height_is_kept_at_the_old_height() {
        assert_eq!(
            reconcile_balance(Some(STORED), 400, BalanceProvenance::ConfirmedUnordered),
            advanced(400, 50)
        );
    }

    #[test]
    fn the_first_observation_starts_a_watermark() {
        assert_eq!(
            reconcile_balance(None, 300, BalanceProvenance::ObservedAt(7)),
            advanced(300, 7)
        );
    }

    #[test]
    fn without_a_watermark_every_balance_is_accepted_as_before() {
        assert_eq!(
            reconcile_balance(None, 300, BalanceProvenance::Unobserved),
            kept(300)
        );
        assert_eq!(
            reconcile_balance(None, 300, BalanceProvenance::ConfirmedUnordered),
            kept(300)
        );
    }
}
