//! Per-network selected-wallet pointer.
//!
//! Captures which HD wallet (`hd_wallet_hash`) and/or single-key wallet
//! (`single_key_hash`) the user last interacted with on the active
//! network. Persisted as a single bincode blob at [`SelectedWallet::KV_KEY`]
//! in the per-network wallet k/v store (the upstream `SqlitePersister`
//! that [`crate::wallet_backend::WalletBackend`] owns), under the global
//! (`None`) wallet scope.
//!
//! Per-network because wallet selection is meaningful only within a
//! network: the wallet you had selected on testnet is not the wallet
//! you had selected on mainnet. The blob therefore lives inside the
//! network's own persister rather than the cross-network `det-app.sqlite`
//! from C3.
//!
//! On first launch after the C4 upgrade the blob is absent and the
//! [`Default`] (both fields `None`) is returned, matching the
//! "empty start" policy of the data.db unwire.

use serde::{Deserialize, Serialize};

/// SHA-256 seed-hash identifier reused from the wallet model.
pub type SelectedWalletHash = [u8; 32];

/// Persisted "which wallet did the user pick last" pointer for a single
/// network. Both fields are optional and independent — a fresh install
/// (or a never-touched network) deserialises to `Default` (both `None`),
/// the wallet picker lands on its empty state, and any later user
/// selection writes the relevant field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedWallet {
    /// Seed hash of the HD wallet the user last selected, if any.
    pub hd_wallet_hash: Option<SelectedWalletHash>,
    /// Key hash of the single-key wallet the user last selected, if any.
    pub single_key_hash: Option<SelectedWalletHash>,
}

impl SelectedWallet {
    /// Canonical k/v key under which the blob is stored inside the
    /// per-network wallet persister. Global (`None`) wallet scope.
    pub const KV_KEY: &'static str = "det:selected_wallet:v1";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip the blob through bincode so the persisted shape is
    /// covered the same way C3's `AppSettings` is — a future field
    /// addition that breaks decoding will surface here.
    #[test]
    fn selected_wallet_round_trips_through_bincode() {
        let original = SelectedWallet {
            hd_wallet_hash: Some([0x11; 32]),
            single_key_hash: Some([0x22; 32]),
        };
        let encoded =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (SelectedWallet, _) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
                .expect("decode");
        assert_eq!(decoded, original);
    }

    /// Default is both-`None` so the empty-start path matches the
    /// previous "no selection persisted" behaviour the data.db row
    /// produced when fresh.
    #[test]
    fn default_is_both_none() {
        let s = SelectedWallet::default();
        assert!(s.hd_wallet_hash.is_none());
        assert!(s.single_key_hash.is_none());
    }
}
