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

/// The original (pre-`uses_password`) [`WalletMeta`] on-disk shape, decode-only.
///
/// `WalletMeta` is a positional-bincode `DetKv` value, so appending the
/// `uses_password` / `password_hint` fields is format-breaking for blobs
/// already written in the 4-field shape — `#[serde(default)]` does NOT rescue
/// them (positional bincode never reports an absent trailing field; it reads
/// past the end and errors). The dual-format reader
/// ([`WalletMetaView::get`](crate::wallet_backend::WalletMetaView)) tries the
/// current shape first, then falls back to decoding this legacy shape, and
/// re-stores in the new shape. No version byte: the two shapes are told apart
/// by which one decodes, leaning on the `DetKv` schema envelope rather than a
/// hand-rolled tag that could collide with a bincode length varint.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletMetaV1 {
    /// See [`WalletMeta::alias`].
    pub alias: String,
    /// See [`WalletMeta::is_main`].
    pub is_main: bool,
    /// See [`WalletMeta::core_wallet_name`].
    pub core_wallet_name: Option<String>,
    /// See [`WalletMeta::xpub_encoded`].
    #[serde(default)]
    pub xpub_encoded: Vec<u8>,
}

impl From<WalletMetaV1> for WalletMeta {
    fn from(v1: WalletMetaV1) -> Self {
        WalletMeta {
            alias: v1.alias,
            is_main: v1.is_main,
            core_wallet_name: v1.core_wallet_name,
            xpub_encoded: v1.xpub_encoded,
            // A v1 blob predates the password sidecar. The unlock/migration
            // path reads the authoritative flag from the legacy envelope; this
            // default is the safe "ask nothing extra" starting point.
            uses_password: false,
            password_hint: None,
        }
    }
}

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
    /// a format-breaking change for already-stored blobs. Evolve the shape by
    /// adding a decode-only legacy shape (see [`WalletMetaV1`]) and a
    /// dual-format reader, never by relying on `#[serde(default)]` alone.
    #[serde(default)]
    pub xpub_encoded: Vec<u8>,
    /// `true` when the wallet's seed was stored under a user password. Moved
    /// out of the legacy seed envelope into this non-secret sidecar. After the
    /// raw-seam migration this flips to `false` (the password no longer gates
    /// the at-rest secret) — see the migration's lazy-unlock path.
    #[serde(default)]
    pub uses_password: bool,
    /// Optional user-set password hint, moved out of the legacy seed envelope.
    /// Shown next to the unlock prompt for a not-yet-migrated password wallet.
    #[serde(default)]
    pub password_hint: Option<String>,
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
            uses_password: true,
            password_hint: Some("granny's birthday".into()),
        };
        let bytes =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (WalletMeta, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
        assert_eq!(decoded, original);
    }

    /// W-META-002 — `Default` matches the "fresh install, never named"
    /// shape: empty alias, not main, no Dash Core wallet link, no xpub, no
    /// password.
    #[test]
    fn default_is_empty_unnamed_wallet() {
        let m = WalletMeta::default();
        assert!(m.alias.is_empty());
        assert!(!m.is_main);
        assert!(m.core_wallet_name.is_none());
        assert!(m.xpub_encoded.is_empty());
        assert!(!m.uses_password);
        assert!(m.password_hint.is_none());
    }

    /// TS-META-01 (model leg) — the dual-format decode contract the view's
    /// `read_meta` relies on: a legacy 4-field blob FAILS to decode as the new
    /// 6-field `WalletMeta` (runs out of bytes) but decodes as `WalletMetaV1`;
    /// a 6-field blob decodes as `WalletMeta`. This is why "try new, then V1"
    /// is correct and order-sensitive. Includes the SEC-003 collision case (a
    /// 1-char alias, whose bincode length varint is `1`) — the old leading-byte
    /// dispatch would have mis-routed it; the try-both reader does not.
    #[test]
    fn ts_meta_01_dual_format_decode_is_order_sensitive() {
        let cfg = bincode::config::standard();

        for alias in ["paycheque", "a", "ab"] {
            let v1 = WalletMetaV1 {
                alias: alias.into(),
                is_main: true,
                core_wallet_name: Some("dev".into()),
                xpub_encoded: vec![0x22; 78],
            };
            let old_blob = bincode::serde::encode_to_vec(&v1, cfg).expect("encode v1");

            // The new 6-field struct cannot decode the 4-field blob.
            assert!(
                bincode::serde::decode_from_slice::<WalletMeta, _>(&old_blob, cfg).is_err(),
                "legacy blob (alias {alias:?}) must NOT decode as the new shape",
            );
            // The legacy struct does.
            let (decoded, _): (WalletMetaV1, _) =
                bincode::serde::decode_from_slice(&old_blob, cfg).expect("decode v1");
            assert_eq!(decoded, v1);

            // Migration preserves the v1 fields, defaults the new ones.
            let migrated: WalletMeta = decoded.into();
            assert_eq!(migrated.alias, alias);
            assert_eq!(migrated.xpub_encoded, vec![0x22; 78]);
            assert!(!migrated.uses_password);
            assert!(migrated.password_hint.is_none());
        }

        // A new 6-field blob decodes as WalletMeta (and re-stores identically).
        let v2 = WalletMeta {
            alias: "paycheque".into(),
            is_main: true,
            core_wallet_name: Some("dev-wallet".into()),
            xpub_encoded: vec![0xCD; 78],
            uses_password: true,
            password_hint: Some("hint".into()),
        };
        let new_blob = bincode::serde::encode_to_vec(&v2, cfg).expect("encode v2");
        let (decoded, _): (WalletMeta, _) =
            bincode::serde::decode_from_slice(&new_blob, cfg).expect("decode v2");
        assert_eq!(decoded, v2);
    }

    /// TS-NOLEAK-02 (WalletMeta) — the encoded sidecar blob carries NO secret.
    /// `WalletMeta` structurally cannot hold a key (no secret field); this is
    /// canary coverage that a future field never smuggles one in. Asserted in
    /// both hex and decimal-array form via the shared helper.
    #[test]
    fn ts_noleak_02_wallet_meta_blob_has_no_secret() {
        use crate::wallet_backend::leak_test_support::{
            assert_no_leak_bytes, distinctive_secret_64,
        };
        // A distinctive seed that must NOT appear in the sidecar bytes.
        let secret = distinctive_secret_64();
        let meta = WalletMeta {
            alias: "paycheque".into(),
            is_main: true,
            core_wallet_name: Some("dev".into()),
            // The xpub is PUBLIC material, not the seed — unrelated bytes.
            xpub_encoded: vec![0xCD; 78],
            uses_password: true,
            password_hint: Some("hint".into()),
        };
        let blob =
            bincode::serde::encode_to_vec(&meta, bincode::config::standard()).expect("encode");
        let rendered = format!("{blob:?}");
        assert_no_leak_bytes(&rendered, &secret, "WalletMeta sidecar blob");
    }
}
