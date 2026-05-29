//! DET-side metadata for an imported single-key wallet entry.
//!
//! The actual private-key bytes live in the encrypted secret store; this
//! struct is the public-facing handle that backend tasks and the UI use
//! to list, label, and address-route imported keys.

use dash_sdk::dpp::dashcore::Network;
use serde::{Deserialize, Serialize};

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
}
