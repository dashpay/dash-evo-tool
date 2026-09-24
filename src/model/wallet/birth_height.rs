//! Birth-height policy for upstream wallet registration.
//!
//! When DET registers a wallet with the upstream SPV backend, the persisted
//! `birth_height` decides which compact-filter range the wallet's addresses
//! are matched against. Get it wrong and pre-existing deposits stay invisible
//! at 100% sync (a known funds-visibility trap).
//!
//! The decision is a pure function of how the wallet entered DET, so it lives
//! here as a single source of truth shared by the create/import write path and
//! the cold-boot reconciliation path.

/// How a wallet first entered DET — the only input the birth-height policy
/// needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletOrigin {
    /// A brand-new recovery phrase DET just generated. It has never held funds
    /// before this moment, so there is nothing to scan for in the past.
    Fresh,
    /// A pre-existing recovery phrase brought in from elsewhere — typed at
    /// import, recovered, or carried over by migration. It may already hold
    /// deposits that predate registration.
    Imported,
}

/// Resolve the `birth_height_override` to pass to the upstream
/// `create_wallet_from_seed_bytes` call for a wallet of the given origin.
///
/// - [`WalletOrigin::Fresh`] → `None`: the upstream resolves this to the
///   current SPV tip, scanning only from now forward. Correct and cheap — a
///   freshly generated phrase cannot have prior deposits.
/// - [`WalletOrigin::Imported`] → `Some(0)`: full historical scan from
///   genesis. The only setting that guarantees a deposit made before
///   registration (e.g. a balance restored from a recovery phrase) is found.
///
/// `Some(0)` costs filter-*matching* over the wallet's address set across the
/// whole chain, not a second network download: the SPV client already fetches
/// compact filters from genesis regardless of any wallet's birth height. The
/// match pass populates balances progressively under the normal "syncing"
/// state. A per-network floor below the earliest DET-producible address could
/// trim that cost, but genesis is the safe default and the floor is deferred.
//
// TODO(birth-height-floor): replace genesis with a per-network constant just
// below the earliest height DET could have produced an address, once a
// DET-release-era height is pinned per network. Optimisation only — genesis is
// always correct.
pub fn registration_birth_height(origin: WalletOrigin) -> Option<u32> {
    match origin {
        WalletOrigin::Fresh => None,
        WalletOrigin::Imported => Some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_wallet_scans_from_tip() {
        assert_eq!(registration_birth_height(WalletOrigin::Fresh), None);
    }

    #[test]
    fn imported_wallet_scans_from_genesis() {
        assert_eq!(registration_birth_height(WalletOrigin::Imported), Some(0));
    }
}
