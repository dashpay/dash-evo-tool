//! A "Max" transfer from the Core wallet to the shielded balance.
//!
//! Max moves the whole Core balance minus fees, an amount known only once the
//! funding transaction is built. Upstream tracks such a transfer by an
//! operation id the caller picks, and the id is the only handle that finds the
//! transfer again after a restart, so DET stores it before the transfer starts.

use serde::{Deserialize, Serialize};

/// The operation id of a Max transfer that has started and not yet settled.
///
/// The stored shape is positional: changing the fields needs a sidecar
/// schema-version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingShieldedMaxTransfer {
    /// The upstream operation id; finishing the transfer reuses it.
    pub operation_id: [u8; 16],
}

/// Where a Max transfer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShieldedMaxTransferState {
    /// Started and not finished. Finishing it continues with the same funds
    /// and never moves them twice.
    Pending,
    /// The funds reached the shielded balance.
    Finished,
    /// Nothing was sent; the funds are still in the Core wallet.
    NotSent,
    /// Another payment spent the transfer's coins first, so nothing reached the
    /// shielded balance and the transfer cannot continue.
    Conflicted,
}

impl ShieldedMaxTransferState {
    /// Whether the transfer has an outcome, so its stored id can go.
    pub fn is_settled(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pending_transfer_keeps_its_id() {
        assert!(!ShieldedMaxTransferState::Pending.is_settled());
        for state in [
            ShieldedMaxTransferState::Finished,
            ShieldedMaxTransferState::NotSent,
            ShieldedMaxTransferState::Conflicted,
        ] {
            assert!(state.is_settled(), "{state:?} must release the stored id");
        }
    }
}
