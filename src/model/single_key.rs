//! DET-side metadata for an imported single-key wallet entry.
//!
//! The actual private-key bytes live in the encrypted secret store; this
//! struct is the public-facing handle that backend tasks and the UI use
//! to list, label, and address-route imported keys.

use dash_sdk::dpp::dashcore::key::Error as KeyError;
use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};
use serde::{Deserialize, Serialize};

/// Validates a WIF-encoded private key and derives the P2PKH address.
///
/// Returns the derived address string on success, or the parse error on
/// failure. Pure format validation — no network I/O, no DB access.
/// Used by the import dialog for instant feedback (P8); the backend
/// task is the authoritative enforcement layer.
pub fn validate_wif(wif: &str, network: Network) -> Result<String, KeyError> {
    let priv_key = PrivateKey::from_wif(wif)?;
    let secp = Secp256k1::new();
    let pub_key = PublicKey {
        compressed: priv_key.compressed,
        inner: priv_key.inner.public_key(&secp),
    };
    let address = Address::p2pkh(&pub_key, network);
    Ok(address.to_string())
}

/// Display-side metadata for one imported single-key wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedKey {
    /// Base58-encoded P2PKH address derived from the imported private
    /// key. Used as the stable identifier in the UI and as the suffix in
    /// the secret-store label.
    pub address: String,
    /// Optional user-supplied nickname. `None` until the user edits it.
    pub alias: Option<String>,
    /// Network the address is valid on. Must match the active
    /// `WalletBackend` network — single-key entries are per-network by
    /// the secret store's per-network scoping.
    pub network: Network,
    /// `true` when the imported key requires a per-key passphrase to use.
    /// A protected key is sealed at import time in the upstream Tier-2 envelope
    /// (Argon2id + XChaCha20-Poly1305) under that passphrase — one on-disk shape
    /// from import onward, with no first-unlock migration. The flag is the
    /// prompt-UI signal: `false` means callers can sign without prompting.
    #[serde(default)]
    pub has_passphrase: bool,
    /// Optional user-supplied hint shown next to the passphrase prompt.
    /// `None` for legacy entries that pre-date the per-key passphrase.
    #[serde(default)]
    pub passphrase_hint: Option<String>,
    /// Compressed SEC1-encoded **public** key for this imported key. The
    /// locked-render cold-boot path needs it to rebuild a passphrase-protected
    /// key's display wallet without the secret (moved here from the
    /// `SingleKeyEntry` vault blob under the raw-seam migration). Empty for
    /// entries written before this field — the caller falls back to deriving
    /// from plaintext when the key is unlocked. NON-secret.
    #[serde(default)]
    pub public_key_bytes: Vec<u8>,
}

/// The pre-`public_key_bytes` [`ImportedKey`] on-disk shape, decode-only.
///
/// `ImportedKey` is a positional-bincode `DetKv` value; appending
/// `public_key_bytes` is format-breaking for blobs already written without it
/// (`#[serde(default)]` does not rescue a trailing positional field). The
/// single-key read path tries the current shape first, then falls back to this
/// legacy shape and re-stores in the new shape — the same dual-format treatment
/// `WalletMeta` gets, so the two sibling sidecars share one policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedKeyV1 {
    /// See [`ImportedKey::address`].
    pub address: String,
    /// See [`ImportedKey::alias`].
    pub alias: Option<String>,
    /// See [`ImportedKey::network`].
    pub network: Network,
    /// See [`ImportedKey::has_passphrase`].
    #[serde(default)]
    pub has_passphrase: bool,
    /// See [`ImportedKey::passphrase_hint`].
    #[serde(default)]
    pub passphrase_hint: Option<String>,
}

impl From<ImportedKeyV1> for ImportedKey {
    fn from(v1: ImportedKeyV1) -> Self {
        ImportedKey {
            address: v1.address,
            alias: v1.alias,
            network: v1.network,
            has_passphrase: v1.has_passphrase,
            passphrase_hint: v1.passphrase_hint,
            // Pre-this-field blobs have no stored pubkey; locked render falls
            // back to deriving from plaintext when the key is unlocked.
            public_key_bytes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::leak_test_support::{assert_no_leak_bytes, distinctive_secret_32};

    /// TS-NOLEAK-02 (ImportedKey) — the encoded sidecar blob carries NO private
    /// key, and the PUBLIC key (needed for locked render) IS present. The
    /// sidecar holds only the public key, never the secret; this is the canary.
    #[test]
    fn ts_noleak_02_imported_key_blob_has_no_private_key_but_has_public() {
        let private = distinctive_secret_32();
        // A distinctive PUBLIC-key placeholder we DO expect to find.
        let public = vec![0x02u8; 33];
        let imported = ImportedKey {
            address: "yTestAddr".into(),
            alias: Some("savings".into()),
            network: Network::Testnet,
            has_passphrase: true,
            passphrase_hint: Some("hint".into()),
            public_key_bytes: public.clone(),
        };
        let blob =
            bincode::serde::encode_to_vec(&imported, bincode::config::standard()).expect("encode");

        // The private key appears in neither hex nor decimal-array form.
        let rendered = format!("{blob:?}");
        assert_no_leak_bytes(&rendered, &private, "ImportedKey sidecar blob");

        // The public key IS present (locked render reads it back).
        let decimal = format!(
            "[{}]",
            public
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        // The 33-byte pubkey is embedded as a Vec — its bytes are in the blob.
        let blob_contains_pubkey = blob.windows(public.len()).any(|w| w == public.as_slice());
        assert!(
            blob_contains_pubkey || rendered.contains(&decimal),
            "the public key must be present in the sidecar for locked render"
        );
    }
}
