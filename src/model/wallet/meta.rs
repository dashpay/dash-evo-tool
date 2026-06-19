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

/// On-disk version tag for the bincode-encoded [`WalletMeta`] payload, framed
/// by [`WalletMetaView`](crate::wallet_backend::WalletMetaView) as
/// `[ WALLET_META_VERSION (1B) | bincode(WalletMeta) ]`.
///
/// `WalletMeta` is positional bincode, so adding a field is format-breaking for
/// already-stored blobs — `#[serde(default)]` alone does NOT make a stored blob
/// forward-compatible (it only supplies a value at the Rust layer when a field
/// is genuinely absent from the encoded stream, which positional bincode never
/// reports). This explicit version byte is the gate: v1 is the original shape
/// (no `uses_password` / `password_hint`); v2 adds them. The reader detects the
/// version and migrates a v1 blob to v2 with the new fields defaulted, rather
/// than positionally misparsing it.
pub const WALLET_META_VERSION: u8 = 2;

/// The original (pre-`uses_password`) [`WalletMeta`] on-disk shape. Retained
/// decode-only so a v1 blob (or a pre-version-byte legacy blob) migrates into
/// the current shape instead of being misread.
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
    /// a format-breaking change for already-stored blobs. Evolve the shape
    /// only by bumping [`WALLET_META_VERSION`] and migrating old blobs (see
    /// [`WalletMetaV1`]).
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

/// Encode a [`WalletMeta`] for storage as `[ WALLET_META_VERSION | bincode ]`.
/// The leading version byte lets the reader migrate older shapes instead of
/// positionally misparsing them.
pub fn encode_versioned(meta: &WalletMeta) -> Result<Vec<u8>, bincode::error::EncodeError> {
    let body = bincode::serde::encode_to_vec(meta, bincode::config::standard())?;
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(WALLET_META_VERSION);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode a stored [`WalletMeta`] payload, handling every on-disk shape:
///
/// * leading [`WALLET_META_VERSION`] (current v2) → decode directly;
/// * leading version byte `1` → decode as [`WalletMetaV1`] and migrate;
/// * no recognised version byte (pre-version-byte legacy blob) → try v1 bare
///   bincode and migrate.
///
/// A blob that matches none of these is a decode error — never a positional
/// misparse.
pub fn decode_versioned(bytes: &[u8]) -> Result<WalletMeta, bincode::error::DecodeError> {
    let cfg = bincode::config::standard();
    if let Some((&tag, rest)) = bytes.split_first() {
        if tag == WALLET_META_VERSION {
            let (meta, _) = bincode::serde::decode_from_slice::<WalletMeta, _>(rest, cfg)?;
            return Ok(meta);
        }
        if tag == 1
            && let Ok((v1, _)) = bincode::serde::decode_from_slice::<WalletMetaV1, _>(rest, cfg)
        {
            return Ok(v1.into());
        }
    }
    // Pre-version-byte legacy blob: bare v1 bincode.
    let (v1, _) = bincode::serde::decode_from_slice::<WalletMetaV1, _>(bytes, cfg)?;
    Ok(v1.into())
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

    /// TS-META-01 — the new v2 shape round-trips through the versioned framing
    /// field-for-field, and an OLD v1 blob is detected by its version byte and
    /// migrated (NOT positionally misparsed). The migrated meta defaults
    /// `uses_password=false` / `password_hint=None` and carries every v1 field.
    #[test]
    fn ts_meta_01_versioned_frame_round_trip_and_v1_migration() {
        let v2 = WalletMeta {
            alias: "paycheque".into(),
            is_main: true,
            core_wallet_name: Some("dev-wallet".into()),
            xpub_encoded: vec![0xCD; 78],
            uses_password: true,
            password_hint: Some("hint".into()),
        };
        let framed = encode_versioned(&v2).expect("encode v2");
        assert_eq!(
            framed[0], WALLET_META_VERSION,
            "frame starts with the version tag"
        );
        assert_eq!(decode_versioned(&framed).expect("decode v2"), v2);

        // A v1 blob: framed with version byte 1 over the old shape.
        let v1 = WalletMetaV1 {
            alias: "legacy".into(),
            is_main: false,
            core_wallet_name: None,
            xpub_encoded: vec![0x22; 78],
        };
        let v1_body =
            bincode::serde::encode_to_vec(&v1, bincode::config::standard()).expect("encode v1");
        let mut v1_framed = vec![1u8];
        v1_framed.extend_from_slice(&v1_body);
        let migrated = decode_versioned(&v1_framed).expect("decode + migrate v1");
        assert_eq!(migrated.alias, "legacy");
        assert_eq!(migrated.xpub_encoded, vec![0x22; 78]);
        assert!(
            !migrated.uses_password,
            "v1 migrates with uses_password defaulted false"
        );
        assert!(migrated.password_hint.is_none());

        // A pre-version-byte legacy blob (bare v1 bincode) also migrates.
        let bare = decode_versioned(&v1_body).expect("decode + migrate bare v1");
        assert_eq!(bare.alias, "legacy");
        assert_eq!(WalletMeta::from(v1), bare);
    }
}
