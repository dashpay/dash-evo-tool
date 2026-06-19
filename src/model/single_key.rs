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
    /// `true` when the key bytes inside the upstream vault are wrapped
    /// in DET's per-key AES-GCM envelope (SEC-002 Option C). The UI
    /// keys the unlock prompt off this flag — when `false`, callers can
    /// sign without prompting.
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
