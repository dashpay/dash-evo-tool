//! Single-key (imported WIF) view backed by the upstream `SecretStore`.
//!
//! Each imported private key lives in the encrypted secret vault under the
//! label `single_key_priv.<base58_addr>`, scoped to a fixed per-backend
//! `WalletId` (built from [`SINGLE_KEY_NAMESPACE_BYTES`] via
//! [`single_key_namespace_id`]). The separator is a dot because the upstream
//! label allowlist is `^[A-Za-z0-9._-]{1,64}$` and rejects colons.
//!
//! `SingleKeyView` is the only doorway DET code uses to import, list,
//! forget, or sign with imported keys. WIF parsing goes through
//! `dash_sdk::dpp::dashcore::PrivateKey::from_wif` — DET does not
//! re-implement WIF.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::secp256k1::ecdsa::Signature;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1};
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};
use platform_wallet_storage::secrets::{
    SecretBytes, SecretStore, SecretStoreError, WalletId as SecretWalletId,
};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::single_key::ImportedKey;
use crate::model::wallet::single_key::{
    ClosedSingleKey, OpenSingleKey, SingleKeyData, SingleKeyHash, SingleKeyWallet,
};
use crate::wallet_backend::single_key_entry::SingleKeyEntry;
use crate::wallet_backend::{DetKv, DetScope};

/// Minimum length (in characters) for a per-key passphrase. Mirrors
/// NIST 800-63B / OWASP ASVS 6.2.1's minimum recommendation. The
/// import dialog enforces this client-side and the backend re-checks.
pub const MIN_SINGLE_KEY_PASSPHRASE_LEN: usize = 8;

/// Fixed per-backend namespace id for single-key entries.
///
/// Single-key wallets are not HD wallets, so they share a namespace
/// instead of each carrying a derived `WalletId`. The bytes are the
/// SHA-256 of the ASCII string `"det-single-key-namespace"`, picked so
/// the namespace is recognisable in a hex dump without colliding with
/// any plausible upstream-derived id.
const SINGLE_KEY_NAMESPACE_BYTES: [u8; 32] = [
    0x7a, 0x3c, 0x99, 0x88, 0xc2, 0x4a, 0x55, 0x6e, 0xf1, 0xa0, 0x06, 0xb4, 0x2d, 0x77, 0x90, 0x1e,
    0x4b, 0x2c, 0x68, 0xff, 0x33, 0xd1, 0x84, 0x09, 0xab, 0x57, 0xee, 0x10, 0x6c, 0x95, 0x71, 0x42,
];

/// Label prefix used to namespace single-key private-key entries inside
/// the secret store. Dot is required: the upstream label allowlist is
/// `^[A-Za-z0-9._-]{1,64}$`, so the original `single_key_priv:` design
/// is rewritten to `single_key_priv.` here.
pub const SINGLE_KEY_PRIV_LABEL_PREFIX: &str = "single_key_priv.";

/// Colon-separated namespace shared across networks. The full key is
/// `<network>:single_key_meta:<address>`. The DET-side k/v sidecar holds
/// the enumerable list of imported-key metadata so the cold-boot path can
/// reconstruct the in-memory index without enumerating the (non-enumerable)
/// secret store. Mirrors the [`WalletMetaView`](super::WalletMetaView)
/// shape (T-W-00) — same network-prefix convention.
pub(crate) const SINGLE_KEY_META_INFIX: &str = ":single_key_meta:";

/// Cross-network `<network>:` prefix matching the on-disk vocabulary in
/// `resolve_spv_storage_dir` and the wallet-meta sidecar. Co-located with
/// the secret-store helpers because every single-key key shape uses it.
fn network_prefix(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

/// Build the canonical sidecar key for `(network, address)`.
pub(crate) fn meta_key_for(network: Network, address: &str) -> String {
    format!(
        "{}{SINGLE_KEY_META_INFIX}{address}",
        network_prefix(network)
    )
}

/// Build the cross-network prefix used to enumerate imported keys for
/// `network`.
fn meta_prefix_for(network: Network) -> String {
    format!("{}{SINGLE_KEY_META_INFIX}", network_prefix(network))
}

/// Build the secret-store label for an imported key at `address`.
pub(crate) fn label_for_address(address: &str) -> String {
    format!("{SINGLE_KEY_PRIV_LABEL_PREFIX}{address}")
}

/// The fixed `WalletId` namespace scope for single-key entries.
pub(crate) fn single_key_namespace_id() -> SecretWalletId {
    SecretWalletId::from(SINGLE_KEY_NAMESPACE_BYTES)
}

/// Borrowed view exposing the imported-key operations of a
/// [`WalletBackend`](super::WalletBackend). Constructed via
/// [`WalletBackend::single_key`](super::WalletBackend::single_key).
pub struct SingleKeyView<'a> {
    pub(crate) secret_store: &'a Arc<SecretStore>,
    pub(crate) index: &'a std::sync::RwLock<std::collections::BTreeMap<String, ImportedKey>>,
    pub(crate) network: Network,
    /// Enumerable cross-network sidecar holding the imported-key
    /// metadata blobs. `None` ⇒ a transient view that does not persist
    /// (used by `WalletBackend::new` before the app k/v is wired and by
    /// the unit tests below).
    pub(crate) app_kv: Option<&'a Arc<DetKv>>,
}

/// Optional passphrase choice supplied by the import dialog. Kept as a
/// small struct so the import API has one parameter for "no passphrase
/// / passphrase + hint" rather than two parallel `Option`s.
#[derive(Debug, Clone, Default)]
pub struct ImportPassphrase {
    /// User-supplied passphrase. Empty / `None` ⇒ no passphrase.
    pub passphrase: Option<String>,
    /// Optional hint shown next to the unlock prompt.
    pub hint: Option<String>,
}

impl<'a> SingleKeyView<'a> {
    /// Borrow the moving parts of a [`SingleKeyView`] without going
    /// through [`WalletBackend::single_key`]. Kept `pub` so benches and
    /// downstream tooling can build the view from owned `Arc`s.
    pub fn from_views(
        secret_store: &'a Arc<SecretStore>,
        index: &'a std::sync::RwLock<std::collections::BTreeMap<String, ImportedKey>>,
        network: Network,
        app_kv: Option<&'a Arc<DetKv>>,
    ) -> Self {
        Self {
            secret_store,
            index,
            network,
            app_kv,
        }
    }

    /// Parse a WIF-encoded private key, store its raw secret bytes in the
    /// encrypted vault under `single_key_priv.<address>`, persist the
    /// derived [`ImportedKey`] metadata to the enumerable k/v sidecar,
    /// and refresh the in-memory index. Idempotent on the same WIF —
    /// re-import overwrites the alias and refreshes the stored bytes.
    ///
    /// Equivalent to
    /// [`Self::import_wif_with_passphrase`] with `ImportPassphrase::default()`.
    pub fn import_wif(&self, wif: &str, alias: Option<String>) -> Result<ImportedKey, TaskError> {
        self.import_wif_with_passphrase(wif, alias, ImportPassphrase::default())
    }

    /// SEC-002 Option C — same as [`Self::import_wif`], plus an optional
    /// per-key passphrase. When `passphrase.passphrase` is `Some(p)` and
    /// non-empty the raw key bytes are AES-GCM encrypted under `p`
    /// before being written to the vault; the metadata sidecar records
    /// `has_passphrase = true` so the unlock UI can prompt later. An
    /// empty / `None` passphrase falls back to the legacy
    /// unprotected-but-vault-encrypted shape.
    pub fn import_wif_with_passphrase(
        &self,
        wif: &str,
        alias: Option<String>,
        passphrase: ImportPassphrase,
    ) -> Result<ImportedKey, TaskError> {
        let priv_key = PrivateKey::from_wif(wif).map_err(|source| TaskError::InvalidWif {
            source: Box::new(source),
        })?;
        // The cold-boot rebuild reconstructs the key in compressed form
        // (`PrivateKey::from_byte_array` always sets `compressed = true`), so
        // an uncompressed import would change its address after a restart.
        // Reject it here to keep the displayed address stable.
        if !priv_key.compressed {
            return Err(TaskError::UncompressedWifUnsupported);
        }
        let secp = Secp256k1::new();
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&secp),
        };
        let address = Address::p2pkh(&pub_key, self.network);
        let address_str = address.to_string();

        // Extracted WIF bytes wrapped in `Zeroizing` so the stack copy wipes
        // on drop instead of lingering after the entry is built (SEC-103).
        let raw: Zeroizing<[u8; 32]> = Zeroizing::new(
            priv_key.inner[..]
                .try_into()
                .map_err(|_| TaskError::SingleKeyCryptoFailure)?,
        );

        let entry = match passphrase.passphrase.as_deref() {
            Some(p) if !p.is_empty() => {
                if p.chars().count() < MIN_SINGLE_KEY_PASSPHRASE_LEN {
                    return Err(TaskError::SingleKeyPassphraseTooShort {
                        min: MIN_SINGLE_KEY_PASSPHRASE_LEN as u32,
                    });
                }
                let pub_bytes = pub_key.inner.serialize().to_vec();
                SingleKeyEntry::protected(&raw, p, passphrase.hint.clone(), pub_bytes)?
            }
            _ => SingleKeyEntry::unprotected(*raw),
        };
        let payload = entry.encode()?;

        let label = label_for_address(&address_str);
        let bytes = SecretBytes::from_slice(&payload);
        self.secret_store
            .set(&single_key_namespace_id(), &label, &bytes)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?;

        let imported = ImportedKey {
            address: address_str.clone(),
            alias,
            network: self.network,
            has_passphrase: entry.has_passphrase,
            passphrase_hint: entry.passphrase_hint.clone(),
        };

        if let Some(kv) = self.app_kv {
            let key = meta_key_for(self.network, &address_str);
            kv.put(DetScope::Global, &key, &imported)
                .map_err(|source| TaskError::SingleKeyMetaStorage {
                    source: Box::new(source),
                })?;
        }

        self.index
            .write()
            .map_err(|_| TaskError::ImportedKeyNotFound)?
            .insert(address_str, imported.clone());
        Ok(imported)
    }

    /// Returns `true` when the imported key at `address` was stored
    /// with a per-key passphrase. The UI uses this to decide whether to
    /// prompt before signing.
    pub fn has_passphrase(&self, address: &str) -> bool {
        self.index
            .read()
            .map(|idx| idx.get(address).is_some_and(|k| k.has_passphrase))
            .unwrap_or(false)
    }

    /// Read the raw private-key bytes for an **unprotected** imported key.
    ///
    /// Passphrase-protected keys are not unlocked here — they are obtained
    /// through the JIT chokepoint
    /// ([`SecretAccess::with_secret`](crate::wallet_backend::SecretAccess::with_secret)
    /// with a [`SecretScope::SingleKey`](crate::wallet_backend::SecretScope)),
    /// which prompts for the passphrase and decrypts just-in-time. A direct
    /// call here for a protected key returns
    /// [`TaskError::SingleKeyPassphraseRequired`] so non-interactive callers
    /// get a typed signal rather than a silent failure.
    fn raw_key_bytes(&self, address: &str) -> Result<Zeroizing<[u8; 32]>, TaskError> {
        let label = label_for_address(address);
        let payload = self
            .secret_store
            .get(&single_key_namespace_id(), &label)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?
            .ok_or(TaskError::ImportedKeyNotFound)?;
        let entry = SingleKeyEntry::decode(payload.expose_secret())?;
        if entry.has_passphrase {
            return Err(TaskError::SingleKeyPassphraseRequired {
                addr: address.to_string(),
            });
        }
        // `decrypt` returns the key wrapped in `Zeroizing`, so it wipes on
        // drop instead of lingering on the stack after the sign (SEC-103).
        entry.decrypt(None)
    }

    /// List every imported key tracked by this backend, sorted by
    /// address. Reads the in-memory index only — does not touch the
    /// secret vault.
    pub fn list(&self) -> Vec<ImportedKey> {
        match self.index.read() {
            Ok(map) => map.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Forget the imported key at `address`: drop its index entry, delete
    /// its secret-store row, and remove the k/v sidecar entry. Idempotent
    /// — absent addresses are an `Ok(())`.
    pub fn forget(&self, address: &str) -> Result<(), TaskError> {
        let label = label_for_address(address);
        self.secret_store
            .delete(&single_key_namespace_id(), &label)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?;
        if let Some(kv) = self.app_kv {
            let key = meta_key_for(self.network, address);
            kv.delete(DetScope::Global, &key).map_err(|source| {
                TaskError::SingleKeyMetaStorage {
                    source: Box::new(source),
                }
            })?;
        }
        self.index
            .write()
            .map_err(|_| TaskError::ImportedKeyNotFound)?
            .remove(address);
        Ok(())
    }

    /// Enumerate every imported-key metadata blob persisted for the view's
    /// network from the k/v sidecar. Returns an empty vector when the
    /// view has no sidecar wired (transient construction path), when no
    /// entries exist, or when listing fails (logged).
    pub(crate) fn list_persisted(&self) -> Vec<ImportedKey> {
        let Some(kv) = self.app_kv else {
            return Vec::new();
        };
        let prefix = meta_prefix_for(self.network);
        let keys = match kv.list(DetScope::Global, Some(&prefix)) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    network = ?self.network,
                    error = ?e,
                    "Failed to list single-key sidecar entries; treating as empty",
                );
                return Vec::new();
            }
        };
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            match kv.get::<ImportedKey>(DetScope::Global, &key) {
                Ok(Some(meta)) => out.push(meta),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        target = "wallet_backend::single_key",
                        key = %key,
                        error = ?e,
                        "Skipping unreadable single-key sidecar blob",
                    );
                }
            }
        }
        out
    }

    /// Reconstruct DET-side [`SingleKeyWallet`] rows from the k/v sidecar
    /// plus the encrypted secret vault. Used by the cold-boot hydration
    /// path that replaces the legacy `db.get_single_key_wallets` read.
    ///
    /// Per-entry failures (missing vault row, malformed key bytes,
    /// address parse error) are logged and skipped so a single corrupt
    /// sidecar entry cannot prevent the wallet picker from listing the
    /// survivors. Balances and UTXOs start at zero / empty — the
    /// single-key SPV refresh path is stubbed
    /// ([`TaskError::SingleKeyWalletsUnsupported`]), so this matches the
    /// pre-refresh state the legacy reader produced on launch.
    pub fn hydrate_wallets(&self) -> Vec<(SingleKeyHash, SingleKeyWallet)> {
        let metas = self.list_persisted();
        let mut out = Vec::with_capacity(metas.len());
        for meta in metas {
            match self.rebuild_wallet(&meta) {
                Ok(Some(wallet)) => {
                    out.push((wallet.key_hash, wallet));
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        target = "wallet_backend::single_key",
                        address = %meta.address,
                        error = ?e,
                        "Failed to rebuild single-key wallet from sidecar; skipping",
                    );
                }
            }
        }
        out
    }

    /// Seed the in-memory index from the k/v sidecar. Idempotent: re-runs
    /// overwrite existing in-memory entries with the persisted view, so a
    /// cold-boot hydration cannot lose entries created in the same
    /// process before the backend was wired (mirrors the HD-wallet
    /// `entry().or_insert` pattern in
    /// [`hydrate_context_wallets`](super::WalletBackend::hydrate_context_wallets)).
    pub fn rehydrate_index(&self) -> Result<(), TaskError> {
        let metas = self.list_persisted();
        if metas.is_empty() {
            return Ok(());
        }
        let mut idx = self
            .index
            .write()
            .map_err(|_| TaskError::ImportedKeyNotFound)?;
        for meta in metas {
            idx.entry(meta.address.clone()).or_insert(meta);
        }
        Ok(())
    }

    fn rebuild_wallet(&self, meta: &ImportedKey) -> Result<Option<SingleKeyWallet>, TaskError> {
        let label = label_for_address(&meta.address);
        let secret = match self
            .secret_store
            .get(&single_key_namespace_id(), &label)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })? {
            Some(s) => s,
            None => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    address = %meta.address,
                    "Single-key sidecar entry has no matching vault secret; skipping",
                );
                return Ok(None);
            }
        };

        let entry = match SingleKeyEntry::decode(secret.expose_secret()) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    address = %meta.address,
                    error = ?e,
                    "Single-key vault entry could not be decoded; skipping",
                );
                return Ok(None);
            }
        };

        // For passphrase-protected entries the rebuilt wallet is closed
        // (no plaintext) — the picker still needs to render it so the
        // user can unlock it on demand. Open entries get rebuilt with
        // their raw bytes the way the legacy path did.
        if entry.has_passphrase {
            // Closed: derive identity from public material only.
            return Ok(self.rebuild_closed_passphrase_wallet(meta, &entry));
        }

        let key_bytes = match entry.decrypt(None) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    address = %meta.address,
                    error = ?e,
                    "Single-key entry plaintext recovery failed; skipping",
                );
                return Ok(None);
            }
        };

        let priv_key = match PrivateKey::from_byte_array(&key_bytes, meta.network) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    address = %meta.address,
                    error = %e,
                    "Single-key vault bytes are not a valid private key; skipping",
                );
                return Ok(None);
            }
        };
        let secp = Secp256k1::new();
        let public_key = priv_key.public_key(&secp);
        let address = Address::p2pkh(&public_key, meta.network);

        let key_hash = ClosedSingleKey::compute_key_hash(&key_bytes);
        let closed = ClosedSingleKey {
            key_hash,
            encrypted_private_key: key_bytes.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
        };
        let private_key_data = SingleKeyData::Open(OpenSingleKey {
            private_key: *key_bytes,
            key_info: closed,
        });

        Ok(Some(SingleKeyWallet {
            private_key_data,
            uses_password: false,
            public_key,
            address,
            alias: meta.alias.clone(),
            key_hash,
            confirmed_balance: 0,
            unconfirmed_balance: 0,
            total_balance: 0,
            utxos: std::collections::HashMap::new(),
            core_wallet_name: None,
        }))
    }

    /// Build a [`SingleKeyWallet`] for a passphrase-protected entry
    /// without touching the plaintext. The rebuilt wallet is closed
    /// (`uses_password = true`); the picker can still render the alias,
    /// address, and key_hash. Stores the ciphertext/salt/nonce on the
    /// closed payload so downstream "is this the same key" checks
    /// remain comparable. Returns `None` (skip and log) when the
    /// entry's stored public-key bytes are missing or unparseable —
    /// without them the rebuilt wallet would lack a usable
    /// [`PublicKey`].
    fn rebuild_closed_passphrase_wallet(
        &self,
        meta: &ImportedKey,
        entry: &SingleKeyEntry,
    ) -> Option<SingleKeyWallet> {
        use std::str::FromStr;
        let address = match Address::from_str(&meta.address) {
            Ok(a) => match a.require_network(meta.network) {
                Ok(a) => a,
                Err(_) => {
                    tracing::warn!(
                        target = "wallet_backend::single_key",
                        address = %meta.address,
                        network = ?meta.network,
                        "Locked single-key entry address does not match expected network; skipping",
                    );
                    return None;
                }
            },
            Err(_) => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    address = %meta.address,
                    "Locked single-key entry address is not parseable; skipping",
                );
                return None;
            }
        };

        if entry.public_key_bytes.is_empty() {
            tracing::warn!(
                target = "wallet_backend::single_key",
                address = %meta.address,
                "Locked single-key entry has no stored public key; skipping (re-import to refresh)",
            );
            return None;
        }
        let inner = match dash_sdk::dpp::dashcore::secp256k1::PublicKey::from_slice(
            &entry.public_key_bytes,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::single_key",
                    address = %meta.address,
                    error = %e,
                    "Locked single-key entry public-key bytes are unparseable; skipping",
                );
                return None;
            }
        };
        let public_key = PublicKey {
            compressed: true,
            inner,
        };

        // `compute_key_hash` is defined over the plaintext private
        // key; locked entries don't have access to that here, so we
        // use SHA-256 of the ciphertext as a stable per-entry handle.
        // Two locked entries with the same plaintext (but distinct
        // salts) hash to different values — acceptable since the
        // hash is only used as a map key inside the in-memory wallets
        // BTreeMap.
        let key_hash = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&entry.ciphertext);
            let out = hasher.finalize();
            let mut h = [0u8; 32];
            h.copy_from_slice(&out);
            h
        };
        let closed = ClosedSingleKey {
            key_hash,
            encrypted_private_key: entry.ciphertext.clone(),
            salt: entry.salt.clone(),
            nonce: entry.nonce.clone(),
        };
        Some(SingleKeyWallet {
            private_key_data: SingleKeyData::Closed(closed),
            uses_password: true,
            public_key,
            address,
            alias: meta.alias.clone(),
            key_hash,
            confirmed_balance: 0,
            unconfirmed_balance: 0,
            total_balance: 0,
            utxos: std::collections::HashMap::new(),
            core_wallet_name: None,
        })
    }

    /// Sign a 32-byte message hash with the **unprotected** imported key
    /// registered at `address`. Pure ECDSA on secp256k1; no BIP-32
    /// derivation is touched (TC-SK-008).
    ///
    /// Passphrase-protected keys must be signed through the JIT chokepoint
    /// ([`WalletBackend::sign_single_key`](super::WalletBackend::sign_single_key)),
    /// which prompts and decrypts just-in-time; a direct call here for a
    /// protected key returns [`TaskError::SingleKeyPassphraseRequired`].
    pub fn sign_with(&self, address: &str, msg: &[u8; 32]) -> Result<Signature, TaskError> {
        let bytes = self.raw_key_bytes(address)?;
        sign_message_with_raw_key(&bytes, msg)
    }
}

/// Sign a 32-byte digest with raw secp256k1 private-key bytes. Shared by the
/// unprotected [`SingleKeyView::sign_with`] path and the JIT chokepoint path
/// ([`WalletBackend::sign_single_key`](super::WalletBackend::sign_single_key)),
/// which decrypts the key just-in-time and hands the borrowed bytes here.
pub(crate) fn sign_message_with_raw_key(
    bytes: &[u8; 32],
    msg: &[u8; 32],
) -> Result<Signature, TaskError> {
    let sk = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_byte_array(bytes)
        .map_err(|_| TaskError::ImportedKeyNotFound)?;
    let message = Message::from_digest(*msg);
    Ok(Secp256k1::new().sign_ecdsa(&message, &sk))
}

/// Open or create the file-backed secret store at `path`. The parent
/// directory is created if missing; on Unix the vault file inherits its
/// initial mode from upstream's writer (the encrypted-file backend
/// refuses pre-existing modes looser than `0600`, so the secret-at-rest
/// floor is enforced at open time — see `SecretStoreError::InsecurePermissions`).
///
/// The passphrase is a fixed, non-secret per-process constant: at-rest
/// protection relies on file permissions (enforced by the upstream backend).
/// A user-supplied passphrase is a follow-up (T-SK-03 UX work). The design
/// choice is documented in the ADR under
/// `docs/ai-design/2026-05-18-platform-wallet-migration/`.
pub fn open_secret_store(path: &std::path::Path) -> Result<SecretStore, SecretStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| SecretStoreError::MalformedVault)?;
    }
    SecretStore::file(
        path,
        platform_wallet_storage::secrets::SecretString::new(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_view(
        dir: &std::path::Path,
        network: Network,
    ) -> (
        Arc<SecretStore>,
        std::sync::RwLock<std::collections::BTreeMap<String, ImportedKey>>,
        Network,
    ) {
        let path = dir.join("secrets.pwsvault");
        let store = Arc::new(open_secret_store(&path).expect("open vault"));
        let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
        (store, index, network)
    }

    fn known_wif() -> &'static str {
        // Testnet WIF with deterministic, all-zero-except-last-byte key
        // bytes. Generated locally with `PrivateKey::new(SecretKey::from_byte_array(&[0;31].chain(&[1])).unwrap(), Testnet).to_wif()`
        // and pinned here to keep tests offline + reproducible.
        "cMahea7zqjxrtgAbB7LSGbcQUr1uX1ojuat9jZodMN8rFTv2sfUK"
    }

    /// TC-SK-003: importing a WIF writes exactly one entry whose label
    /// matches `^single_key_priv\.[1-9A-HJ-NP-Za-km-z]{26,35}$` and is
    /// scoped to the per-backend single-key `WalletId` namespace.
    #[test]
    fn tc_sk_003_label_format_and_namespace_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, index, network) = fresh_view(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };

        let imported = view
            .import_wif(known_wif(), Some("primary".to_string()))
            .expect("import");

        // The label uses the dotted prefix (upstream allowlist rejects ':').
        let label = label_for_address(&imported.address);
        assert!(
            label.starts_with(SINGLE_KEY_PRIV_LABEL_PREFIX),
            "label {label} should start with {SINGLE_KEY_PRIV_LABEL_PREFIX}"
        );
        let addr_part = &label[SINGLE_KEY_PRIV_LABEL_PREFIX.len()..];
        let len_ok = (26..=35).contains(&addr_part.len());
        let charset_ok = addr_part
            .bytes()
            .all(|b| matches!(b, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'));
        assert!(
            len_ok && charset_ok,
            "address part {addr_part} should match base58 26-35 chars"
        );

        // Round-trip via the namespace WalletId proves the entry is
        // scoped where the spec requires it — read with a different id
        // and the entry is invisible.
        let got = store
            .get(&single_key_namespace_id(), &label)
            .expect("get scoped")
            .expect("present under namespace");
        assert!(!got.expose_secret().is_empty());

        let other_id = SecretWalletId::from([0u8; 32]);
        let absent = store.get(&other_id, &label).expect("get other");
        assert!(
            absent.is_none(),
            "entry must not be visible under a different WalletId scope"
        );

        // Exactly one entry tracked.
        assert_eq!(view.list().len(), 1);
    }

    /// TC-SK-008: `sign_with` looks the imported key up by its
    /// `single_key_priv.<addr>` label and signs locally with the
    /// recovered bytes. No BIP-32 derivation is touched — the only
    /// secret material reaches the signer through the secret-store
    /// lookup path.
    #[test]
    fn tc_sk_008_sign_uses_secret_store_path_not_bip32() {
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;

        let dir = tempfile::tempdir().expect("tempdir");
        let (store, index, network) = fresh_view(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };
        let imported = view.import_wif(known_wif(), None).expect("import");

        let msg = [0x42u8; 32];
        let sig = view
            .sign_with(&imported.address, &msg)
            .expect("sign with imported key");

        // Verify with the public key derived from the same WIF — proves
        // the signer hit the imported-key path (not some other key the
        // backend might fall back to) without referencing BIP-32 at all.
        let priv_key = PrivateKey::from_wif(known_wif()).expect("wif");
        let secp = Secp256k1::new();
        let pk = priv_key.inner.public_key(&secp);
        secp.verify_ecdsa(&Message::from_digest(msg), &sig, &pk)
            .expect("signature verifies against WIF-derived pubkey");

        // Forgetting the key removes both the index entry and the
        // secret store row, so a second sign attempt surfaces the
        // typed "not found" variant.
        view.forget(&imported.address).expect("forget");
        assert!(view.list().is_empty());
        let err = view
            .sign_with(&imported.address, &msg)
            .expect_err("post-forget sign must fail");
        assert!(
            matches!(err, TaskError::ImportedKeyNotFound),
            "expected ImportedKeyNotFound, got {err:?}"
        );
    }

    /// Invalid WIF surfaces the typed `InvalidWif` variant rather than
    /// the storage diagnostic. No secret-store write happens — the index
    /// stays empty.
    #[test]
    fn invalid_wif_rejected_without_secret_store_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, index, network) = fresh_view(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };
        let err = view
            .import_wif("not-a-valid-wif", None)
            .expect_err("invalid wif");
        assert!(
            matches!(err, TaskError::InvalidWif { .. }),
            "expected InvalidWif, got {err:?}"
        );
        assert!(view.list().is_empty());
    }

    /// Build an uncompressed-format WIF for the same key bytes `known_wif`
    /// encodes, so the reject test exercises the compression flag rather
    /// than a different key.
    fn uncompressed_wif() -> String {
        let mut compressed = PrivateKey::from_wif(known_wif()).expect("parse known wif");
        compressed.compressed = false;
        compressed.to_wif()
    }

    /// F10 — an uncompressed-format WIF is rejected at import with the typed
    /// `UncompressedWifUnsupported` variant; no vault or index write happens.
    #[test]
    fn f10_uncompressed_wif_rejected_at_import() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, index, network) = fresh_view(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };
        let err = view
            .import_wif(&uncompressed_wif(), None)
            .expect_err("uncompressed wif must be rejected");
        assert!(
            matches!(err, TaskError::UncompressedWifUnsupported),
            "expected UncompressedWifUnsupported, got {err:?}"
        );
        assert!(view.list().is_empty(), "no entry should be created");
    }

    /// F10 — round-trip: a compressed WIF import → persist → cold-boot
    /// rebuild yields the SAME address the import derived. Locks the
    /// no-divergence guarantee the uncompressed reject protects.
    #[test]
    fn f10_compressed_import_rebuild_preserves_address() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };
        let imported = view.import_wif(known_wif(), None).expect("import");

        // Simulate a fresh process: drop the in-memory index, rebuild.
        index.write().unwrap().clear();
        let rebuilt = view.hydrate_wallets();
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(
            rebuilt[0].1.address.to_string(),
            imported.address,
            "rebuilt address must match the import-time address"
        );
    }

    /// In-memory `KvStore` adapter shared by the sidecar tests below.
    /// The single-key sidecar is global-only, so the fake asserts every
    /// call lands in [`ObjectId::Global`] and stores under a flat map.
    #[derive(Default)]
    struct InMemoryKv {
        global: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    }

    impl platform_wallet_storage::KvStore for InMemoryKv {
        fn get(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            key: &str,
        ) -> Result<Option<Vec<u8>>, platform_wallet_storage::KvError> {
            assert_eq!(
                scope,
                &platform_wallet_storage::ObjectId::Global,
                "single-key sidecar uses global scope"
            );
            Ok(self.global.lock().unwrap().get(key).cloned())
        }
        fn put(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            key: &str,
            value: &[u8],
        ) -> Result<(), platform_wallet_storage::KvError> {
            assert_eq!(
                scope,
                &platform_wallet_storage::ObjectId::Global,
                "single-key sidecar uses global scope"
            );
            self.global
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_vec());
            Ok(())
        }
        fn delete(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            key: &str,
        ) -> Result<(), platform_wallet_storage::KvError> {
            assert_eq!(
                scope,
                &platform_wallet_storage::ObjectId::Global,
                "single-key sidecar uses global scope"
            );
            self.global.lock().unwrap().remove(key);
            Ok(())
        }
        fn list_keys(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, platform_wallet_storage::KvError> {
            assert_eq!(
                scope,
                &platform_wallet_storage::ObjectId::Global,
                "single-key sidecar uses global scope"
            );
            let g = self.global.lock().unwrap();
            let it = g
                .keys()
                .filter(|k| prefix.is_none_or(|p| k.starts_with(p)))
                .cloned();
            Ok(it.collect())
        }
    }

    /// Test fixture bundling the moving parts a [`SingleKeyView`] needs
    /// when wired against a fake `KvStore`. Returned as a struct to
    /// keep the constructor tuple-light (clippy `type_complexity`).
    struct ViewFixture {
        store: Arc<SecretStore>,
        index: std::sync::RwLock<std::collections::BTreeMap<String, ImportedKey>>,
        kv: Arc<DetKv>,
        network: Network,
    }

    fn fresh_view_with_kv(dir: &std::path::Path, network: Network) -> ViewFixture {
        let path = dir.join("secrets.pwsvault");
        let store = Arc::new(open_secret_store(&path).expect("open vault"));
        let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
        let kv = Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default())));
        ViewFixture {
            store,
            index,
            kv,
            network,
        }
    }

    /// TC-W-01b-A — `import_wif` writes a sidecar entry under the
    /// canonical `<network>:single_key_meta:<addr>` key, listable by the
    /// cross-network prefix.
    #[test]
    fn tc_w_01b_a_import_writes_sidecar_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };

        let imported = view
            .import_wif(known_wif(), Some("primary".into()))
            .expect("import");
        let prefix = meta_prefix_for(network);
        let keys = kv.list(DetScope::Global, Some(&prefix)).expect("list");
        assert_eq!(keys.len(), 1, "exactly one sidecar entry");
        assert_eq!(keys[0], meta_key_for(network, &imported.address));

        let stored: ImportedKey = kv
            .get(DetScope::Global, &keys[0])
            .expect("get")
            .expect("entry present");
        assert_eq!(stored, imported);
    }

    /// TC-W-01b-B — `forget` deletes the sidecar entry idempotently. A
    /// re-call on an already-forgotten address remains `Ok(())` and does
    /// not resurrect anything in the listing.
    #[test]
    fn tc_w_01b_b_forget_drops_sidecar_entry_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };

        let imported = view.import_wif(known_wif(), None).expect("import");
        view.forget(&imported.address).expect("forget");
        view.forget(&imported.address).expect("forget twice");

        let prefix = meta_prefix_for(network);
        let keys = kv.list(DetScope::Global, Some(&prefix)).expect("list");
        assert!(keys.is_empty(), "sidecar must be empty after forget");
        assert!(view.list().is_empty());
    }

    /// TC-W-01b-C — cold-boot hydration rebuilds a [`SingleKeyWallet`]
    /// from `(sidecar, secret-store)` with the alias preserved, the
    /// derived address matching the secret bytes, and the wallet opened
    /// in-process (no per-wallet password — vault scope is the gate).
    #[test]
    fn tc_w_01b_c_hydrate_round_trip_rebuilds_wallet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };
        let imported = view
            .import_wif(known_wif(), Some("savings".into()))
            .expect("import");

        // Drop the in-memory index to simulate a fresh process.
        index.write().unwrap().clear();
        let rebuilt = view.hydrate_wallets();
        assert_eq!(rebuilt.len(), 1);
        let (_, wallet) = &rebuilt[0];
        assert_eq!(wallet.address.to_string(), imported.address);
        assert_eq!(wallet.alias.as_deref(), Some("savings"));
        assert!(wallet.is_open(), "rehydrated wallet must be open");
        assert!(
            !wallet.uses_password,
            "vault-scoped (no per-wallet password)"
        );
        assert_eq!(wallet.confirmed_balance, 0);
        assert!(wallet.utxos.is_empty());

        // Re-seeding the index from the sidecar is idempotent — the
        // entry appears once and matches the original.
        view.rehydrate_index().expect("rehydrate");
        let listed = view.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], imported);
    }

    /// TC-W-01b-D — a sidecar entry whose vault row is missing is
    /// skipped (logged) so a single corrupt pair cannot block the
    /// picker. The hydration vector still yields healthy entries.
    #[test]
    fn tc_w_01b_d_orphan_sidecar_entry_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };

        // Healthy entry.
        view.import_wif(known_wif(), None).expect("import");

        // Orphan sidecar entry — sidecar row written, no vault row.
        let orphan = ImportedKey {
            address: "yNotARealAddress".into(),
            alias: Some("ghost".into()),
            network,
            has_passphrase: false,
            passphrase_hint: None,
        };
        kv.put(
            DetScope::Global,
            &meta_key_for(network, &orphan.address),
            &orphan,
        )
        .expect("put orphan");

        let rebuilt = view.hydrate_wallets();
        assert_eq!(
            rebuilt.len(),
            1,
            "orphan must be skipped, healthy entry preserved"
        );
    }

    /// SEC-002 — importing with a passphrase encrypts the in-vault
    /// payload (so a vault dump does not yield the raw key) and the
    /// sidecar records `has_passphrase = true` with the user's hint.
    ///
    /// JIT model: there is no unlock cache to prime at import, so a direct
    /// `sign_with` on the protected key returns the typed
    /// `SingleKeyPassphraseRequired` — protected signing flows through the
    /// chokepoint instead (see `sec_002_protected_sign_via_chokepoint`).
    #[test]
    fn sec_002_import_with_passphrase_encrypts_payload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };

        let imported = view
            .import_wif_with_passphrase(
                known_wif(),
                Some("secure".into()),
                crate::wallet_backend::single_key::ImportPassphrase {
                    passphrase: Some("correcthorsebattery".into()),
                    hint: Some("xkcd 936".into()),
                },
            )
            .expect("import");
        assert!(imported.has_passphrase);
        assert_eq!(imported.passphrase_hint.as_deref(), Some("xkcd 936"));

        // Vault payload is not the raw 32 bytes — it's the versioned
        // ciphertext envelope.
        let label = label_for_address(&imported.address);
        let raw = store
            .get(&single_key_namespace_id(), &label)
            .expect("get")
            .expect("present");
        assert_ne!(raw.expose_secret().len(), 32);
        let priv_key = PrivateKey::from_wif(known_wif()).unwrap();
        assert_ne!(
            raw.expose_secret(),
            &priv_key.inner[..],
            "ciphertext must not be the plaintext key bytes",
        );

        // No cache prime: a direct view sign on the protected key reports
        // that a passphrase is required (the chokepoint is the unlock path).
        let err = view
            .sign_with(&imported.address, &[0x42u8; 32])
            .expect_err("protected key has no cache to sign from");
        assert!(matches!(err, TaskError::SingleKeyPassphraseRequired { .. }));
    }

    /// SEC-002, JIT-adapted — a protected imported key is signed through
    /// the chokepoint. A direct view sign reports `SingleKeyPassphraseRequired`;
    /// then `SecretAccess::with_secret` prompts, re-asks on a wrong passphrase,
    /// decrypts just-in-time on the right one, and signs. The signature
    /// verifies against the WIF-derived public key.
    #[tokio::test]
    async fn sec_002_protected_sign_via_chokepoint() {
        use crate::wallet_backend::secret_access::SecretAccess;
        use crate::wallet_backend::secret_prompt::SecretScope;
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};

        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };
        let imported = view
            .import_wif_with_passphrase(
                known_wif(),
                None,
                crate::wallet_backend::single_key::ImportPassphrase {
                    passphrase: Some("opensesame".into()),
                    hint: None,
                },
            )
            .expect("import");

        // Re-seed the index from the sidecar (cold-boot analogue): no
        // plaintext is cached anywhere.
        view.rehydrate_index().expect("rehydrate");

        // Direct view sign on the protected key → typed "passphrase required".
        let err = view
            .sign_with(&imported.address, &[0u8; 32])
            .expect_err("locked sign must surface PassphraseRequired");
        match err {
            TaskError::SingleKeyPassphraseRequired { addr } => {
                assert_eq!(addr, imported.address);
            }
            other => panic!("expected SingleKeyPassphraseRequired, got {other:?}"),
        }

        // Chokepoint path: one wrong passphrase (re-ask) then the right one.
        let prompt = Arc::new(TestPrompt::new([
            ScriptedAnswer::once("wrong-one"),
            ScriptedAnswer::once("opensesame"),
        ]));
        let sa = SecretAccess::new(Arc::clone(&store), prompt.clone(), network);
        sa.set_single_key_index(index.read().unwrap().clone());
        let scope = SecretScope::SingleKey {
            address: imported.address.clone(),
        };
        let msg = [0u8; 32];
        let sig = sa
            .with_secret(&scope, |pt| {
                let bytes = pt
                    .expose_single_key()
                    .ok_or(TaskError::ImportedKeyNotFound)?;
                sign_message_with_raw_key(bytes, &msg)
            })
            .await
            .expect("chokepoint signs after the right passphrase");

        let priv_key = PrivateKey::from_wif(known_wif()).unwrap();
        let secp = Secp256k1::new();
        let pk = priv_key.inner.public_key(&secp);
        secp.verify_ecdsa(&Message::from_digest(msg), &sig, &pk)
            .expect("chokepoint signature verifies");
        assert_eq!(prompt.ask_count(), 2, "one wrong + one right passphrase");
    }

    /// SEC-002 — a passphrase shorter than the configured minimum is
    /// rejected at import time with the typed
    /// `SingleKeyPassphraseTooShort` variant; no vault write occurs.
    #[test]
    fn sec_002_short_passphrase_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };
        let err = view
            .import_wif_with_passphrase(
                known_wif(),
                None,
                crate::wallet_backend::single_key::ImportPassphrase {
                    passphrase: Some("short".into()),
                    hint: None,
                },
            )
            .expect_err("short passphrase rejected");
        match err {
            TaskError::SingleKeyPassphraseTooShort { min } => {
                assert_eq!(min, super::MIN_SINGLE_KEY_PASSPHRASE_LEN as u32);
            }
            other => panic!("expected SingleKeyPassphraseTooShort, got {other:?}"),
        }
        assert!(view.list().is_empty(), "no entry should be created");
    }

    /// SEC-002 — legacy 32-byte raw vault payloads (pre-Option C)
    /// still decode as `has_passphrase = false`, so a user who
    /// upgrades from a previous tag never loses their imported keys.
    #[test]
    fn sec_002_legacy_raw_payload_still_signs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ViewFixture {
            store,
            index,
            kv,
            network,
        } = fresh_view_with_kv(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: Some(&kv),
        };

        // Pretend a pre-SEC-002 install wrote a raw 32-byte payload
        // under the canonical label, with a matching sidecar entry.
        let priv_key = PrivateKey::from_wif(known_wif()).unwrap();
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&Secp256k1::new()),
        };
        let address = Address::p2pkh(&pub_key, network).to_string();
        let label = label_for_address(&address);
        let raw = SecretBytes::from_slice(&priv_key.inner[..]);
        store
            .set(&single_key_namespace_id(), &label, &raw)
            .expect("write legacy bytes");
        let meta = ImportedKey {
            address: address.clone(),
            alias: None,
            network,
            has_passphrase: false,
            passphrase_hint: None,
        };
        kv.put(DetScope::Global, &meta_key_for(network, &address), &meta)
            .expect("seed sidecar");
        view.rehydrate_index().expect("rehydrate");

        // No passphrase needed, signing works.
        view.sign_with(&address, &[0x11u8; 32])
            .expect("legacy sign without passphrase");
    }
}
