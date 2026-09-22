//! Per-network selected-identity pointer.
//!
//! The selected identity is the app-scoped "who am I operating as" choice.
//! Persisted as a single bincode blob at [`SelectedIdentity::KV_KEY`] in the
//! per-network wallet k/v store (the upstream `SqlitePersister` that
//! [`crate::wallet_backend::WalletBackend`] owns), under the global (`None`)
//! wallet scope — mirroring [`crate::model::selected_wallet::SelectedWallet`].
//!
//! It is a **separate** blob from `SelectedWallet`: extending the wallet blob
//! with a new bincode field would break decoding of existing `:v1` wallet
//! blobs and silently drop the persisted wallet selection on upgrade.
//!
//! Per-network because identities are per-network: a fresh per-network
//! `AppContext` is built on a network switch, so the blob is per-network for
//! free with no key discriminator.

use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};

/// Persisted "which identity am I operating as" pointer for a single network.
/// A fresh install (or never-touched network) deserialises to `Default`
/// (`None`), and the hub lands on its count-based default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedIdentity {
    /// The selected identity, if any.
    pub identity_id: Option<Identifier>,
}

impl SelectedIdentity {
    /// Canonical k/v key under which the blob is stored inside the per-network
    /// wallet persister. Global (`None`) wallet scope.
    pub const KV_KEY: &'static str = "det:selected_identity:v1";
}

/// Keep `selected` only if it is still present in `loaded`. The restore /
/// refresh filter — mirrors the wallet pointer's "keep only if still loaded".
pub fn keep_if_loaded(selected: Option<Identifier>, loaded: &[Identifier]) -> Option<Identifier> {
    selected.filter(|id| loaded.contains(id))
}

/// Precedence for resolving the active identity: the selected id when it is
/// still loaded, otherwise the first loaded identity, otherwise `None`.
///
/// Used both for the "who am I" read (over all loaded identities) and for
/// reconciling the active identity to a newly selected wallet (over that
/// wallet's identities) — keep-if-present-else-first-else-none.
pub fn resolve_selected(selected: Option<Identifier>, loaded: &[Identifier]) -> Option<Identifier> {
    keep_if_loaded(selected, loaded).or_else(|| loaded.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> Identifier {
        Identifier::from([b; 32])
    }

    /// The persisted shape must survive a bincode round-trip, like the
    /// sibling `SelectedWallet` blob — a future field change that breaks
    /// decoding surfaces here.
    #[test]
    fn selected_identity_round_trips_through_bincode() {
        for original in [
            SelectedIdentity {
                identity_id: Some(id(0x11)),
            },
            SelectedIdentity { identity_id: None },
        ] {
            let encoded = bincode::serde::encode_to_vec(&original, bincode::config::standard())
                .expect("encode");
            let (decoded, _): (SelectedIdentity, _) =
                bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
                    .expect("decode");
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn default_is_none() {
        assert!(SelectedIdentity::default().identity_id.is_none());
    }

    /// `keep_if_loaded` drops a selection absent from the loaded set and
    /// keeps one that is present — the restore/refresh contract.
    #[test]
    fn keep_if_loaded_drops_unloaded_keeps_loaded() {
        let loaded = [id(1), id(2)];
        assert_eq!(keep_if_loaded(Some(id(2)), &loaded), Some(id(2)));
        assert_eq!(keep_if_loaded(Some(id(9)), &loaded), None);
        assert_eq!(keep_if_loaded(None, &loaded), None);
    }

    /// `resolve_selected` precedence: selected-if-loaded → first → none.
    #[test]
    fn resolve_selected_precedence() {
        let loaded = [id(1), id(2), id(3)];
        // Selected and loaded → keep it.
        assert_eq!(resolve_selected(Some(id(2)), &loaded), Some(id(2)));
        // Selected but not loaded → fall back to the first loaded.
        assert_eq!(resolve_selected(Some(id(9)), &loaded), Some(id(1)));
        // Nothing selected → first loaded.
        assert_eq!(resolve_selected(None, &loaded), Some(id(1)));
        // Nothing loaded → none.
        assert_eq!(resolve_selected(Some(id(1)), &[]), None);
        assert_eq!(resolve_selected(None, &[]), None);
    }

    /// Reconciling to a wallet's identity set uses the same precedence: keep
    /// the current id if the new wallet owns it, else that wallet's first
    /// identity, else none (→ picker).
    #[test]
    fn resolve_reconciles_to_wallet_identity_set() {
        let wallet_a = [id(1), id(2)];
        let wallet_b = [id(7), id(8)];
        // Current id owned by the new wallet → kept.
        assert_eq!(resolve_selected(Some(id(2)), &wallet_a), Some(id(2)));
        // Current id not owned by the new wallet → first of the new wallet.
        assert_eq!(resolve_selected(Some(id(2)), &wallet_b), Some(id(7)));
        // New wallet has no identities → none (caller routes to the picker).
        assert_eq!(resolve_selected(Some(id(2)), &[]), None);
    }
}
