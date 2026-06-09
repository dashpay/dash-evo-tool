//! DET-owned wallet-metadata sidecar.
//!
//! Upstream `WalletMetadataEntry` only carries `network` and
//! `birth_height`. The fields DET needs in addition — the user-chosen
//! alias, the "main" flag the wallet picker uses to pre-select the
//! default wallet, and the legacy Dash Core wallet name link — live in
//! this DET-side struct. Persisted as a single bincode blob per
//! `(network, seed_hash)` pair in the cross-network `det-app.sqlite`
//! k/v store, behind the `DetKv` schema-version envelope.
//!
//! Two audiences for this struct:
//!
//! - The HD wallet listing path (T-W-01 cuts the legacy `db.get_wallets`
//!   readers) reads it to populate the "Alias" and "Main wallet"
//!   columns.
//! - The one-shot migration (T-W-00) drains the legacy `wallet` rows
//!   into the sidecar so an existing install keeps its names after
//!   the upgrade.

use serde::{Deserialize, Serialize};

/// DET-owned per-wallet metadata.
///
/// Lives next to the upstream wallet state, not inside it: upstream
/// owns balance / transactions / identities; DET owns these display
/// fields plus a pre-computed master BIP44 ECDSA xpub so the wallet
/// picker can render at cold boot without touching the encrypted seed
/// vault.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletMeta {
    /// User-visible label. Empty string when the user never named
    /// the wallet — matches the legacy "alias is NULL" column shape
    /// from the DET data.db.
    pub alias: String,
    /// Whether this is the "main" wallet on the active network — the
    /// wallet picker pre-selects it on launch. At most one wallet
    /// per network is expected to carry `true`; the picker does not
    /// enforce uniqueness — last-write-wins.
    pub is_main: bool,
    /// Optional link to a Dash Core wallet by name. Power-user feature
    /// for Devnet / Regtest installs that drive a local `dashd` from
    /// DET; `None` for the default cloud / SPV install.
    pub core_wallet_name: Option<String>,
    /// `ExtendedPubKey::encode()` bytes for `m/44'/coin'/0'`. Computed
    /// once when the wallet is first persisted; the picker reads it on
    /// every boot so locked seeds don't have to be unlocked to render
    /// the list. An empty vector means "xpub unknown" — the caller skips
    /// the operations that derive from it (currently only the picker).
    ///
    /// `#[serde(default)]` only supplies a value at the Rust layer; it does
    /// NOT make this blob forward-compatible. `WalletMeta` is stored as a
    /// positional `bincode::config::standard()` blob behind the `DetKv`
    /// schema envelope, so adding, removing, or reordering any field here is
    /// a format-breaking change for already-stored blobs. Evolve the shape
    /// only by bumping `crate::wallet_backend::kv::SCHEMA_VERSION` and
    /// migrating old blobs.
    #[serde(default)]
    pub xpub_encoded: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W-META-001 — round-trip through bincode so the persisted shape
    /// is covered the same way `AppSettings` / `SelectedWallet` are.
    /// Adding a field that breaks decoding surfaces here.
    #[test]
    fn wallet_meta_round_trips_through_bincode() {
        let original = WalletMeta {
            alias: "paycheque".into(),
            is_main: true,
            core_wallet_name: Some("dev-wallet".into()),
            xpub_encoded: vec![0xAB; 78],
        };
        let bytes =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (WalletMeta, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
        assert_eq!(decoded, original);
    }

    /// W-META-002 — `Default` matches the "fresh install, never named"
    /// shape: empty alias, not main, no Dash Core wallet link, no xpub.
    #[test]
    fn default_is_empty_unnamed_wallet() {
        let m = WalletMeta::default();
        assert!(m.alias.is_empty());
        assert!(!m.is_main);
        assert!(m.core_wallet_name.is_none());
        assert!(m.xpub_encoded.is_empty());
    }
}
