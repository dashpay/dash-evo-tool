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

#[cfg(test)]
use dash_sdk::dpp::dashcore::secp256k1::Message;
use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
#[cfg(test)]
use dash_sdk::dpp::dashcore::secp256k1::ecdsa::Signature;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};
use platform_wallet_storage::secrets::{
    SecretBytes, SecretStore, SecretStoreError, SecretString, WalletId as SecretWalletId,
};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::single_key::ImportedKey;
use crate::model::wallet::single_key::{
    ClosedSingleKey, OpenSingleKey, SingleKeyData, SingleKeyHash, SingleKeyWallet,
};
use crate::wallet_backend::kv::network_prefix;
use crate::wallet_backend::poison::{read_recover, write_recover};
use crate::wallet_backend::secret_seam::{SecretScheme, SecretSeam};
use crate::wallet_backend::single_key_entry::SingleKeyEntry;
use crate::wallet_backend::{DetKv, DetScope};

/// Minimum length (in characters) for a per-key passphrase. Re-exported
/// from the model so the rule has a single home; both this backend and
/// the import/restore dialogs share the same value.
pub use crate::model::wallet::passphrase::MIN_SINGLE_KEY_PASSPHRASE_LEN;

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
///
/// `Debug` is hand-written to redact the passphrase: deriving it would
/// dump the plaintext into logs or panic backtraces.
#[derive(Clone, Default)]
pub struct ImportPassphrase {
    /// User-supplied passphrase, kept in [`Zeroizing`] so it wipes on
    /// drop. Empty / `None` ⇒ no passphrase.
    pub passphrase: Option<Zeroizing<String>>,
    /// Optional hint shown next to the unlock prompt.
    pub hint: Option<String>,
}

impl std::fmt::Debug for ImportPassphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportPassphrase")
            .field(
                "passphrase",
                &self
                    .passphrase
                    .as_ref()
                    .map(|p| format!("[redacted; len {}]", p.len())),
            )
            .field("hint", &self.hint)
            .finish()
    }
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

    /// Per-key passphrase import — same as [`Self::import_wif`], plus an optional
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
        if let Some(alias) = alias.as_deref() {
            crate::model::wallet::validate_wallet_alias(alias)
                .map_err(|source| TaskError::InvalidWalletAliasLength { source })?;
        }
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
        // on drop instead of lingering after the entry is built.
        let raw: Zeroizing<[u8; 32]> = Zeroizing::new(
            priv_key.inner[..]
                .try_into()
                .map_err(|_| TaskError::SingleKeyCryptoFailure)?,
        );

        let pub_bytes = pub_key.inner.serialize().to_vec();
        let label = label_for_address(&address_str);

        // Both tiers route through the secret seam under the same label — no
        // DET-side `SingleKeyEntry` framing for new imports. An unprotected key
        // is stored as RAW 32 bytes (Tier-1); a protected key is sealed Tier-2
        // under the user's passphrase (Argon2id + XChaCha20-Poly1305) at import
        // time, so the storage chokepoint is a single shape from import onward
        // with no lazy first-unlock migration. The locked-render pubkey lives in
        // the `ImportedKey` sidecar either way.
        let (has_passphrase, passphrase_hint) =
            match passphrase.passphrase.as_ref().map(|p| p.as_str()) {
                Some(p) if !p.is_empty() => {
                    if p.chars().count() < MIN_SINGLE_KEY_PASSPHRASE_LEN {
                        return Err(TaskError::SingleKeyPassphraseTooShort {
                            min: MIN_SINGLE_KEY_PASSPHRASE_LEN as u32,
                        });
                    }
                    let pw = SecretString::new(p);
                    SecretSeam::new(self.secret_store).put_secret_protected(
                        &single_key_namespace_id(),
                        &label,
                        &SecretBytes::from_slice(&*raw),
                        &pw,
                    )?;
                    (true, passphrase.hint.clone())
                }
                _ => {
                    SecretSeam::new(self.secret_store).put_secret(
                        &single_key_namespace_id(),
                        &label,
                        &SecretBytes::from_slice(&*raw),
                    )?;
                    (false, None)
                }
            };

        let imported = ImportedKey {
            address: address_str.clone(),
            alias,
            network: self.network,
            has_passphrase,
            passphrase_hint,
            public_key_bytes: pub_bytes,
        };

        if let Some(kv) = self.app_kv {
            let key = meta_key_for(self.network, &address_str);
            kv.put(DetScope::Global, &key, &imported)
                .map_err(|source| TaskError::SingleKeyMetaStorage {
                    source: Box::new(source),
                })?;
        }

        write_recover(self.index).insert(address_str, imported.clone());
        Ok(imported)
    }

    /// Persist a new alias for the imported key at `address` to the
    /// modern sidecar and refresh the in-memory index. This is the
    /// single source of truth for single-key renames — it mirrors the
    /// HD-wallet rename path through `WalletMetaView::set`, so the new
    /// name survives a cold boot without touching the legacy
    /// `single_key_wallet` table. An empty `alias` clears the nickname.
    ///
    /// The index write guard spans persistence so same-address renames serialize.
    /// With a sidecar, the index changes only after a successful write. Without
    /// one, the transient in-memory index is updated directly.
    pub fn set_alias(&self, address: &str, alias: Option<String>) -> Result<(), TaskError> {
        if let Some(alias) = alias.as_deref() {
            crate::model::wallet::validate_wallet_alias(alias)
                .map_err(|source| TaskError::InvalidWalletAliasLength { source })?;
        }
        let mut idx = write_recover(self.index);
        let mut updated = idx
            .get(address)
            .cloned()
            .ok_or(TaskError::ImportedKeyNotFound)?;
        updated.alias = alias;

        if let Some(kv) = self.app_kv {
            let key = meta_key_for(self.network, address);
            kv.put(DetScope::Global, &key, &updated).map_err(|source| {
                TaskError::SingleKeyMetaStorage {
                    source: Box::new(source),
                }
            })?;
        }
        idx.insert(address.to_string(), updated);
        Ok(())
    }

    /// Confirm that `passphrase` unlocks the protected imported key at
    /// `address`, without retaining the decrypted key anywhere. The plaintext
    /// is decrypted just-in-time into a [`Zeroizing`] binding that is dropped
    /// (and wiped) before this returns — so a successful "Unlock" gesture
    /// leaves no plaintext in the long-lived session map.
    ///
    /// Returns [`TaskError::SingleKeyPassphraseIncorrect`] on a wrong
    /// passphrase (the same generic signal as the restore path — no oracle).
    /// For an unprotected entry the passphrase is irrelevant. A not-yet-migrated
    /// legacy protected entry that just unlocked is RE-WRAPPED to a Tier-2
    /// object-password envelope under the same password (protection KEPT;
    /// `has_passphrase` stays true) — so there is no downgrade to surface and no
    /// notice to show. An already-Tier-2 entry is verified by unsealing and
    /// needs no re-wrap.
    pub fn verify_passphrase(&self, address: &str, passphrase: &str) -> Result<(), TaskError> {
        let label = label_for_address(address);
        match SecretSeam::new(self.secret_store).scheme(&single_key_namespace_id(), &label)? {
            // Already Tier-2: verify by unsealing with the supplied password. A
            // wrong password maps to the generic incorrect signal (no oracle); a
            // correct one confirms without re-parking plaintext. No re-wrap.
            SecretScheme::Protected => {
                let pw = SecretString::new(passphrase);
                SecretSeam::new(self.secret_store)
                    .get_secret_protected(&single_key_namespace_id(), &label, &pw)
                    // A wrong password maps to the generic incorrect signal (no
                    // oracle), matched structurally on the seam's typed source.
                    .map_err(|e| match &e {
                        TaskError::SecretSeam { source }
                            if matches!(**source, SecretStoreError::WrongPassword) =>
                        {
                            TaskError::SingleKeyPassphraseIncorrect
                        }
                        _ => e,
                    })?
                    .ok_or(TaskError::ImportedKeyNotFound)?;
                Ok(())
            }
            SecretScheme::Absent => Err(TaskError::ImportedKeyNotFound),
            // Legacy `SingleKeyEntry` (or a migrated raw-32 key): decode, decrypt
            // to verify, then lazily re-wrap a protected entry to Tier-2 under the
            // SAME password. An unprotected entry ignores the passphrase.
            SecretScheme::Unprotected => {
                let payload = SecretSeam::new(self.secret_store)
                    .get_secret(&single_key_namespace_id(), &label)?
                    .ok_or(TaskError::ImportedKeyNotFound)?;
                let entry = SingleKeyEntry::decode(payload.expose_secret())?;
                // Decrypt to verify, then drop immediately — the binding is wiped
                // on drop, so the plaintext never crosses back out of this method.
                let verified: Zeroizing<[u8; 32]> = entry.decrypt(Some(passphrase))?;
                if entry.has_passphrase {
                    let pw = SecretString::new(passphrase);
                    SecretSeam::new(self.secret_store).put_secret_protected(
                        &single_key_namespace_id(),
                        &label,
                        &SecretBytes::from_slice(&*verified),
                        &pw,
                    )?;
                }
                Ok(())
            }
        }
    }

    /// List every imported key tracked by this backend, sorted by
    /// address. Reads the in-memory index only — does not touch the
    /// secret vault.
    pub fn list(&self) -> Vec<ImportedKey> {
        read_recover(self.index).values().cloned().collect()
    }

    /// Forget the imported key at `address`: drop its index entry, delete
    /// its secret-store row, and remove the k/v sidecar entry. Idempotent
    /// — absent addresses are an `Ok(())`.
    pub fn forget(&self, address: &str) -> Result<(), TaskError> {
        let label = label_for_address(address);
        SecretSeam::new(self.secret_store).delete_secret(&single_key_namespace_id(), &label)?;
        if let Some(kv) = self.app_kv {
            let key = meta_key_for(self.network, address);
            kv.delete(DetScope::Global, &key).map_err(|source| {
                TaskError::SingleKeyMetaStorage {
                    source: Box::new(source),
                }
            })?;
        }
        write_recover(self.index).remove(address);
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
            match self.read_imported_key(kv, &key) {
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

    /// Read one `ImportedKey` sidecar blob with a dual-format fallback. Tries
    /// the current shape first; on a decode failure (an old blob lacks the
    /// appended `public_key_bytes`) falls back to the legacy [`ImportedKeyV1`]
    /// shape and RE-STORES it in the current shape — so an imported key created
    /// before that field still appears in the picker instead of vanishing.
    /// Mirrors the `WalletMeta` dual-format reader.
    fn read_imported_key(
        &self,
        kv: &Arc<DetKv>,
        key: &str,
    ) -> Result<Option<ImportedKey>, crate::wallet_backend::KvAdapterError> {
        use crate::wallet_backend::KvAdapterError;
        match kv.get::<ImportedKey>(DetScope::Global, key) {
            Ok(opt) => return Ok(opt),
            Err(KvAdapterError::Decode(_)) => {}
            Err(e) => return Err(e),
        }
        let Some(v1) = kv.get::<crate::model::single_key::ImportedKeyV1>(DetScope::Global, key)?
        else {
            return Ok(None);
        };
        let migrated: ImportedKey = v1.into();
        if let Err(e) = kv.put(DetScope::Global, key, &migrated) {
            tracing::warn!(
                target = "wallet_backend::single_key",
                key = %key,
                error = ?e,
                "Could not re-store migrated single-key sidecar; will retry next read",
            );
        }
        Ok(Some(migrated))
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

    /// Rebuild the single display [`SingleKeyWallet`] for one imported key,
    /// reading the same vault + metadata the cold-boot
    /// [`Self::hydrate_wallets`] path uses. Routes through the shared
    /// [`Self::rebuild_wallet`], so the in-memory shape an import produces is
    /// byte-for-byte identical to the one a restart reconstructs: a
    /// passphrase-protected key comes back **closed** (no plaintext in the
    /// long-lived map), an unprotected key comes back open.
    ///
    /// `Ok(None)` mirrors `rebuild_wallet`'s skip-and-log contract (missing
    /// vault row, unparseable bytes) — the caller decides how to surface a
    /// freshly-imported key that could not be rebuilt.
    pub fn rebuild_display_wallet(
        &self,
        meta: &ImportedKey,
    ) -> Result<Option<SingleKeyWallet>, TaskError> {
        self.rebuild_wallet(meta)
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
        let mut idx = write_recover(self.index);
        for meta in metas {
            idx.entry(meta.address.clone()).or_insert(meta);
        }
        Ok(())
    }

    fn rebuild_wallet(&self, meta: &ImportedKey) -> Result<Option<SingleKeyWallet>, TaskError> {
        let label = label_for_address(&meta.address);
        // A key re-wrapped to a Tier-2 object-password envelope (keep-protection,
        // on the first unlock) reads back as Protected without the password.
        // Reconstruct it CLOSED from the public sidecar — the password is
        // intentionally unavailable at cold boot, so the secret is never read
        // here. Without the scheme probe a plain `get` would surface
        // `NeedsPassword` and the key would vanish from the picker.
        if matches!(
            SecretSeam::new(self.secret_store).scheme(&single_key_namespace_id(), &label)?,
            SecretScheme::Protected
        ) {
            return Ok(self.rebuild_closed_tier2_wallet(meta));
        }
        let secret = match SecretSeam::new(self.secret_store)
            .get_secret(&single_key_namespace_id(), &label)?
        {
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
        let (address, public_key) = parse_locked_address_and_pubkey(meta, &entry.public_key_bytes)?;

        // `compute_key_hash` is defined over the plaintext private key;
        // locked entries don't have it here, so the handle is SHA-256 of the
        // ciphertext under a domain-separation tag. The tag keeps this handle
        // in a different space from the plaintext `compute_key_hash`, so a
        // locked entry and an open one can never collide on the same BTreeMap
        // key. Two locked entries with the same plaintext but distinct salts
        // still hash apart — fine, the handle is only a per-entry map key.
        const LOCKED_HANDLE_DOMAIN: &[u8] = b"det-single-key-locked-handle-v1";
        let key_hash = locked_key_handle(LOCKED_HANDLE_DOMAIN, &entry.ciphertext);
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

    /// Build a closed [`SingleKeyWallet`] for a key whose secret is sealed in a
    /// Tier-2 object-password envelope — the steady-state shape after the first
    /// unlock. The ciphertext is unreachable without the password at cold boot,
    /// so the public material comes from the `ImportedKey` sidecar
    /// (`public_key_bytes` + `address`) and the per-entry handle is derived from
    /// the public key bytes. Returns `None` (skip + log) when the sidecar's
    /// public material is missing or unparseable.
    fn rebuild_closed_tier2_wallet(&self, meta: &ImportedKey) -> Option<SingleKeyWallet> {
        let (address, public_key) = parse_locked_address_and_pubkey(meta, &meta.public_key_bytes)?;

        // No ciphertext is reachable for a Tier-2 entry without the password, so
        // the per-entry handle is domain-separated over the public key bytes
        // (which uniquely identify the key) instead of the ciphertext.
        const LOCKED_TIER2_HANDLE_DOMAIN: &[u8] = b"det-single-key-locked-tier2-handle-v1";
        let key_hash = locked_key_handle(LOCKED_TIER2_HANDLE_DOMAIN, &meta.public_key_bytes);
        let closed = ClosedSingleKey {
            key_hash,
            encrypted_private_key: Vec::new(),
            salt: Vec::new(),
            nonce: Vec::new(),
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
}

/// Parse the stored address (network-checked) and the compressed public key
/// from `public_key_bytes` for a locked single-key render. `None` (skip + log)
/// when the address or the public key is missing or unparseable — without both
/// the rebuilt wallet would lack a usable address / [`PublicKey`]. Shared by the
/// legacy-AES-GCM and Tier-2 closed-render paths so they apply one policy.
fn parse_locked_address_and_pubkey(
    meta: &ImportedKey,
    public_key_bytes: &[u8],
) -> Option<(Address, PublicKey)> {
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

    if public_key_bytes.is_empty() {
        tracing::warn!(
            target = "wallet_backend::single_key",
            address = %meta.address,
            "Locked single-key entry has no stored public key; skipping (re-import to refresh)",
        );
        return None;
    }
    let inner = match dash_sdk::dpp::dashcore::secp256k1::PublicKey::from_slice(public_key_bytes) {
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
    Some((
        address,
        PublicKey {
            compressed: true,
            inner,
        },
    ))
}

/// SHA-256 of `domain || material`, used as a stable per-entry BTreeMap handle
/// for a locked single-key wallet. The domain tag keeps locked handles in a
/// different space from the plaintext `compute_key_hash`, so a locked entry and
/// an open one can never collide.
fn locked_key_handle(domain: &[u8], material: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(material);
    let out = hasher.finalize();
    let mut h = [0u8; 32];
    h.copy_from_slice(&out);
    h
}

/// Open or create the file-backed secret store at `path`. The parent
/// directory is created if missing; on Unix the vault file inherits its
/// initial mode from upstream's writer (the encrypted-file backend
/// refuses pre-existing modes looser than `0600`, so the secret-at-rest
/// floor is enforced at open time — see `SecretStoreError::InsecurePermissions`).
///
/// The vault file itself is opened **keyless** ([`SecretStore::file_unprotected`]).
/// Upstream documents this verbatim as **"obfuscation, not confidentiality"**: the
/// vault key derives from an empty passphrase under a public salt, so anyone who
/// can READ the vault file can re-derive it and recover every **Tier-1**
/// (unprotected) secret. Tier-1 at-rest protection is therefore **owner-only file
/// permissions ALONE** — it covers no-password seeds, raw imported keys, and
/// identity keys (prompt-free by design for headless signing).
///
/// Real at-rest **confidentiality** comes only from **Tier-2** *object* passwords:
/// each protected secret is sealed under its own password (Argon2id + XChaCha20)
/// via [`SecretStore::set_secret`] / read back with [`SecretStore::get_secret`]
/// BEFORE it reaches the backend, so a full vault-file compromise cannot reveal a
/// protected secret. (Upstream's [`SecretStore::file`] now rejects a blank
/// passphrase; `file_unprotected` is the explicit keyless door it documents for
/// exactly this per-secret-password model.) This Tier-1-is-obfuscation-only
/// residual is an accepted, documented risk — see the ADR under
/// `docs/ai-design/2026-06-19-secret-storage-seam/`. Hosts that can hold
/// a real key may instead use [`SecretStore::os`] (OS keyring) or a vault
/// passphrase via `EncryptedFileStore::rekey`.
pub fn open_secret_store(path: &std::path::Path) -> Result<SecretStore, SecretStoreError> {
    prepare_vault_dir(path)?;
    SecretStore::file_unprotected(path)
}

/// Open the file-backed secret store at `path` unlocked by `passphrase`.
///
/// The funds-safe, non-destructive recovery door for a **legacy vault** an
/// older build wrote with a real passphrase: the keyless
/// [`open_secret_store`] fails such a vault with
/// [`SecretStoreError::WrongPassphrase`], and the GUI boot seam falls through
/// to this with the user-supplied passphrase. It opens the SAME vault in
/// place — it never deletes, recreates, or rekeys it, so wallet seeds are
/// never at risk. Parent-directory creation and owner-only permissions match
/// [`open_secret_store`] exactly.
///
/// A passphrase shorter than the upstream minimum is rejected
/// ([`SecretStoreError::BlankPassphrase`]); the deliberately keyless door is
/// [`open_secret_store`].
pub fn open_secret_store_with_passphrase(
    path: &std::path::Path,
    passphrase: SecretString,
) -> Result<SecretStore, SecretStoreError> {
    prepare_vault_dir(path)?;
    SecretStore::file(path, passphrase)
}

/// Create the vault's parent directory and lock it to owner-only.
///
/// The upstream file backend refuses to open a vault whose parent dir is
/// group/other-writable (a rename-swap threat), so the secrets dir is forced
/// to `0700` before the vault is opened or created.
fn prepare_vault_dir(path: &std::path::Path) -> Result<(), SecretStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| SecretStoreError::MalformedVault)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| SecretStoreError::MalformedVault)?;
        }
    }
    Ok(())
}

/// Test-only signing helpers. Production single-key signing goes through the
/// JIT chokepoint ([`WalletBackend::sign_single_key`](super::WalletBackend::sign_single_key)),
/// which resolves the plaintext via [`SecretAccess`](crate::wallet_backend::SecretAccess)
/// and signs through [`DetSigner`](crate::wallet_backend::det_signer); these
/// direct-from-view paths exist only to exercise the unprotected key lookup in
/// unit tests.
#[cfg(test)]
impl<'a> SingleKeyView<'a> {
    /// Read the raw private-key bytes for an **unprotected** imported key.
    ///
    /// A protected key (Tier-2-sealed or a legacy passphrase-protected
    /// `SingleKeyEntry`) is never unlocked here — the direct call returns
    /// [`TaskError::SingleKeyPassphraseRequired`], mirroring the production
    /// chokepoint's typed signal.
    fn raw_key_bytes(&self, address: &str) -> Result<Zeroizing<[u8; 32]>, TaskError> {
        let label = label_for_address(address);
        // A Tier-2-sealed key cannot be read without the passphrase — surface the
        // typed "passphrase required" signal (the chokepoint is the unlock path),
        // mirroring the legacy protected `SingleKeyEntry` case below.
        if matches!(
            SecretSeam::new(self.secret_store).scheme(&single_key_namespace_id(), &label)?,
            SecretScheme::Protected
        ) {
            return Err(TaskError::SingleKeyPassphraseRequired {
                addr: address.to_string(),
            });
        }
        let payload = SecretSeam::new(self.secret_store)
            .get_secret(&single_key_namespace_id(), &label)?
            .ok_or(TaskError::ImportedKeyNotFound)?;
        let entry = SingleKeyEntry::decode(payload.expose_secret())?;
        if entry.has_passphrase {
            return Err(TaskError::SingleKeyPassphraseRequired {
                addr: address.to_string(),
            });
        }
        // `decrypt` returns the key wrapped in `Zeroizing`, so it wipes on
        // drop instead of lingering on the stack after the sign.
        entry.decrypt(None)
    }

    /// Sign a 32-byte message hash with the **unprotected** imported key
    /// registered at `address`. Pure ECDSA on secp256k1; no BIP-32
    /// derivation is touched (TC-SK-008). A protected key returns
    /// [`TaskError::SingleKeyPassphraseRequired`].
    fn sign_with(&self, address: &str, msg: &[u8; 32]) -> Result<Signature, TaskError> {
        let bytes = self.raw_key_bytes(address)?;
        sign_message_with_raw_key(&bytes, msg)
    }
}

/// Sign a 32-byte digest with raw secp256k1 private-key bytes. Test-only —
/// the production JIT path signs inline through
/// [`DetSigner`](crate::wallet_backend::det_signer::DetSigner).
#[cfg(test)]
fn sign_message_with_raw_key(bytes: &[u8; 32], msg: &[u8; 32]) -> Result<Signature, TaskError> {
    let sk = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_byte_array(bytes)
        .map_err(|_| TaskError::ImportedKeyNotFound)?;
    let message = Message::from_digest(*msg);
    Ok(Secp256k1::new().sign_ecdsa(&message, &sk))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-written `ImportPassphrase` `Debug` must redact the
    /// passphrase so it can never leak through `{:?}` into logs or panic
    /// backtraces.
    #[test]
    fn import_passphrase_debug_redacts_secret() {
        let pp = "correct-horse-battery-staple";
        let imp = ImportPassphrase {
            passphrase: Some(Zeroizing::new(pp.to_string())),
            hint: Some("the usual".into()),
        };
        let dbg = format!("{imp:?}");
        assert!(!dbg.contains(pp), "debug leaked the passphrase: {dbg}");
        assert!(
            dbg.contains("[redacted"),
            "debug must mark redaction: {dbg}"
        );
        // Non-secret hint stays visible for diagnostics.
        assert!(dbg.contains("the usual"));
    }

    /// A vault written WITH a real passphrase: the keyless boot open fails
    /// `WrongPassphrase`, but the passphrase-accepting open recovers the SAME
    /// vault and reads back the identical entry — proving the recovery door is
    /// non-destructive (no delete/recreate/rekey of the seed vault).
    #[test]
    fn legacy_passphrase_vault_round_trips_without_destroying_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::app_dir::ensure_data_dir_exists(dir.path()).expect("secure test data dir");
        let path = dir.path().join("secrets").join("det-secrets.pwsvault");
        let scope = single_key_namespace_id();
        let label = "single_key_priv.roundtrip";
        let secret = [7u8; 32];

        // Seed a passphrase-protected vault, then release its exclusive lock.
        {
            let store = open_secret_store_with_passphrase(&path, SecretString::new("legacy-pass"))
                .expect("create passphrase vault");
            store
                .set(&scope, label, &SecretBytes::from_slice(&secret))
                .expect("write entry");
        }

        // Boot opens keyless → the empty-passphrase verify-token fails.
        let err = open_secret_store(&path).expect_err("keyless open must fail a passphrase vault");
        assert!(
            matches!(err, SecretStoreError::WrongPassphrase),
            "expected WrongPassphrase, got {err:?}"
        );

        // The correct passphrase re-opens the same vault, data intact.
        let store = open_secret_store_with_passphrase(&path, SecretString::new("legacy-pass"))
            .expect("re-open with passphrase");
        let got = store
            .get(&scope, label)
            .expect("read entry")
            .expect("entry present");
        assert_eq!(got.expose_secret(), &secret, "recovered bytes must match");
    }

    /// The deliberately keyless door must reject opening with a passphrase only
    /// when the vault was sealed with one — and a freshly keyless vault must
    /// still open keyless (regression guard for the recovery refactor).
    #[test]
    fn keyless_vault_still_opens_keyless_after_refactor() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::app_dir::ensure_data_dir_exists(dir.path()).expect("secure test data dir");
        let path = dir.path().join("secrets").join("det-secrets.pwsvault");
        let scope = single_key_namespace_id();
        {
            let store = open_secret_store(&path).expect("create keyless vault");
            store
                .set(&scope, "k", &SecretBytes::from_slice(&[1u8; 32]))
                .expect("write");
        }
        // Re-open keyless — the default boot path is unchanged.
        let store = open_secret_store(&path).expect("re-open keyless");
        assert!(store.get(&scope, "k").expect("read").is_some());
    }

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

    #[test]
    fn import_wif_rejects_overlong_alias_before_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, index, network) = fresh_view(dir.path(), Network::Testnet);
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network,
            app_kv: None,
        };

        let error = view
            .import_wif(known_wif(), Some("w".repeat(65)))
            .expect_err("overlong alias must fail");

        assert!(matches!(error, TaskError::InvalidWalletAliasLength { .. }));
        assert!(view.list().is_empty());
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

    #[derive(Default)]
    struct AliasPutGateState {
        armed: bool,
        first_put_waiting: bool,
        release_first_put: bool,
    }

    #[derive(Default)]
    struct FirstAliasPutGate {
        inner: InMemoryKv,
        state: std::sync::Mutex<AliasPutGateState>,
        changed: std::sync::Condvar,
    }

    impl FirstAliasPutGate {
        fn arm(&self) {
            let mut state = self.state.lock().expect("gate state");
            *state = AliasPutGateState {
                armed: true,
                ..Default::default()
            };
        }

        fn wait_until_first_put(&self) {
            let state = self.state.lock().expect("gate state");
            let (state, timeout) = self
                .changed
                .wait_timeout_while(state, std::time::Duration::from_secs(5), |state| {
                    !state.first_put_waiting
                })
                .expect("gate wait");
            assert!(
                !timeout.timed_out() && state.first_put_waiting,
                "first alias put gate"
            );
        }

        fn release_first_put(&self) {
            let mut state = self.state.lock().expect("gate state");
            state.release_first_put = true;
            self.changed.notify_all();
        }
    }

    impl platform_wallet_storage::KvStore for FirstAliasPutGate {
        fn get(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            key: &str,
        ) -> Result<Option<Vec<u8>>, platform_wallet_storage::KvError> {
            self.inner.get(scope, key)
        }

        fn put(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            key: &str,
            value: &[u8],
        ) -> Result<(), platform_wallet_storage::KvError> {
            let mut state = self.state.lock().expect("gate state");
            if state.armed && !state.first_put_waiting && key.contains(SINGLE_KEY_META_INFIX) {
                state.first_put_waiting = true;
                self.changed.notify_all();
                state = self
                    .changed
                    .wait_while(state, |state| !state.release_first_put)
                    .expect("gate release");
            }
            drop(state);
            self.inner.put(scope, key, value)
        }

        fn delete(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            key: &str,
        ) -> Result<(), platform_wallet_storage::KvError> {
            self.inner.delete(scope, key)
        }

        fn list_keys(
            &self,
            scope: &platform_wallet_storage::ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, platform_wallet_storage::KvError> {
            self.inner.list_keys(scope, prefix)
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
            public_key_bytes: Vec::new(),
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

    /// Importing with a passphrase encrypts the in-vault
    /// payload (so a vault dump does not yield the raw key) and the
    /// sidecar records `has_passphrase = true` with the user's hint. A fresh
    /// protected import seals Tier-2 at import time, so the vault row reads back
    /// as [`SecretScheme::Protected`] (a password-free read fails).
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
                    passphrase: Some(Zeroizing::new("correcthorsebattery".into())),
                    hint: Some("xkcd 936".into()),
                },
            )
            .expect("import");
        assert!(imported.has_passphrase);
        assert_eq!(imported.passphrase_hint.as_deref(), Some("xkcd 936"));

        // The vault row is sealed Tier-2 at import — a password-free read fails
        // (NeedsPassword), so the at-rest value is never the plaintext key.
        let label = label_for_address(&imported.address);
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(&single_key_namespace_id(), &label)
                .expect("scheme"),
            SecretScheme::Protected,
            "a protected import must seal Tier-2 at import time",
        );
        assert!(
            store.get(&single_key_namespace_id(), &label).is_err(),
            "a password-free read of a Tier-2 single key must fail",
        );

        // No cache prime: a direct view sign on the protected key reports
        // that a passphrase is required (the chokepoint is the unlock path).
        let err = view
            .sign_with(&imported.address, &[0x42u8; 32])
            .expect_err("protected key has no cache to sign from");
        assert!(matches!(err, TaskError::SingleKeyPassphraseRequired { .. }));
    }

    /// Regression for the cold-boot disappearance of a Tier-2-protected single
    /// key: after the first unlock re-wraps the key to a Tier-2 object-password
    /// envelope (keep-protection), the cold-boot rebuild must still list it
    /// CLOSED instead of skipping it. Before the scheme-first branch,
    /// `rebuild_wallet` did a plain `get`, which surfaced `NeedsPassword` and the
    /// key vanished from the picker on every launch.
    #[test]
    fn tier2_protected_single_key_rebuilds_closed_and_is_listed() {
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

        let passphrase = "correct-horse-battery-staple";
        let imported = view
            .import_wif_with_passphrase(
                known_wif(),
                Some("savings".into()),
                crate::wallet_backend::single_key::ImportPassphrase {
                    passphrase: Some(Zeroizing::new(passphrase.into())),
                    hint: Some("xkcd 936".into()),
                },
            )
            .expect("import");
        let address = imported.address.clone();

        // First unlock re-wraps the legacy AES-GCM entry to a Tier-2 envelope
        // under the same password.
        view.verify_passphrase(&address, passphrase)
            .expect("verify + re-wrap");
        let label = label_for_address(&address);
        assert_eq!(
            SecretSeam::new(&store)
                .scheme(&single_key_namespace_id(), &label)
                .expect("scheme"),
            SecretScheme::Protected,
            "key must read back as Tier-2 protected without a password",
        );

        // Cold-boot rebuild returns Ok(Some(closed)) — never an Err that skips.
        let rebuilt = view
            .rebuild_display_wallet(&imported)
            .expect("no error from a protected single key")
            .expect("protected key must rebuild closed, not be skipped");
        assert!(
            matches!(rebuilt.private_key_data, SingleKeyData::Closed(_)),
            "a Tier-2 key must rebuild as a closed wallet",
        );
        assert!(rebuilt.uses_password);
        assert_eq!(rebuilt.address.to_string(), address);

        // And the full cold-boot enumeration lists it too.
        let listed = view.hydrate_wallets();
        assert!(
            listed.iter().any(|(_, w)| w.address.to_string() == address),
            "the Tier-2 single key must appear in the cold-boot listing",
        );
    }

    /// JIT-adapted protected sign — a protected imported key is signed through
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
                    passphrase: Some(Zeroizing::new("opensesame".into())),
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

    /// A passphrase shorter than the configured minimum is
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
                    passphrase: Some(Zeroizing::new("short".into())),
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

    /// #192 — renaming an imported single key persists the new alias to
    /// the modern sidecar (not the legacy DB), so the rename survives a
    /// cold-boot rehydration. Mirrors the HD-wallet rename path.
    #[test]
    fn set_alias_persists_to_sidecar_and_survives_rehydrate() {
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
            .import_wif(known_wif(), Some("old name".into()))
            .expect("import");

        view.set_alias(&imported.address, Some("new name".into()))
            .expect("rename");

        // In-memory index reflects the new alias immediately.
        assert_eq!(
            view.list()[0].alias.as_deref(),
            Some("new name"),
            "rename must update the in-memory index"
        );

        // Cold-boot analogue: drop the index and rehydrate from the
        // sidecar — the persisted alias must be the new one.
        index.write().unwrap().clear();
        view.rehydrate_index().expect("rehydrate");
        assert_eq!(
            view.list()[0].alias.as_deref(),
            Some("new name"),
            "renamed alias must survive a cold-boot rehydration"
        );
    }

    /// Renaming an address that was never imported surfaces the typed
    /// `ImportedKeyNotFound` rather than silently creating a ghost entry.
    #[test]
    fn set_alias_unknown_address_is_typed_not_found() {
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
            .set_alias("yNeverImported", Some("x".into()))
            .expect_err("unknown address must fail");
        assert!(matches!(err, TaskError::ImportedKeyNotFound), "got {err:?}");
    }

    #[test]
    fn set_alias_rejects_overlong_alias_before_persisting() {
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
            .import_wif(known_wif(), Some("old name".into()))
            .expect("import");

        let error = view
            .set_alias(&imported.address, Some("w".repeat(65)))
            .expect_err("overlong alias must fail");

        assert!(matches!(error, TaskError::InvalidWalletAliasLength { .. }));
        assert_eq!(view.list()[0].alias.as_deref(), Some("old name"));
    }

    #[test]
    fn overlapping_alias_updates_keep_index_and_sidecar_consistent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store =
            Arc::new(open_secret_store(&dir.path().join("secrets.pwsvault")).expect("open vault"));
        let index = Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
        let gated_store = Arc::new(FirstAliasPutGate::default());
        let kv = Arc::new(DetKv::from_store(gated_store.clone()));
        let view = SingleKeyView {
            secret_store: &store,
            index: &index,
            network: Network::Testnet,
            app_kv: Some(&kv),
        };
        let address = view
            .import_wif(known_wif(), Some("original".into()))
            .expect("import")
            .address;
        gated_store.arm();

        let first_store = store.clone();
        let first_index = index.clone();
        let first_kv = kv.clone();
        let first_address = address.clone();
        let first = std::thread::spawn(move || {
            SingleKeyView {
                secret_store: &first_store,
                index: &first_index,
                network: Network::Testnet,
                app_kv: Some(&first_kv),
            }
            .set_alias(&first_address, Some("first".into()))
        });
        gated_store.wait_until_first_put();

        let later_store = store.clone();
        let later_index = index.clone();
        let later_kv = kv.clone();
        let later_address = address.clone();
        let (later_tx, later_rx) = std::sync::mpsc::channel();
        let later = std::thread::spawn(move || {
            let result = SingleKeyView {
                secret_store: &later_store,
                index: &later_index,
                network: Network::Testnet,
                app_kv: Some(&later_kv),
            }
            .set_alias(&later_address, Some("later".into()));
            later_tx.send(result).expect("send later result");
        });

        let later_while_first_blocked = later_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .ok();
        gated_store.release_first_put();
        first
            .join()
            .expect("first rename thread")
            .expect("first rename");
        let later_result = match later_while_first_blocked {
            Some(result) => result,
            None => later_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("later rename completion"),
        };
        later_result.expect("later rename");
        later.join().expect("later rename thread");

        let indexed_alias = read_recover(&index)
            .get(&address)
            .and_then(|entry| entry.alias.as_deref())
            .map(str::to_owned);
        let persisted_alias = kv
            .get::<ImportedKey>(DetScope::Global, &meta_key_for(Network::Testnet, &address))
            .expect("read persisted alias")
            .expect("persisted entry")
            .alias;
        assert_eq!(indexed_alias.as_deref(), Some("later"));
        assert_eq!(
            persisted_alias.as_deref(),
            Some("later"),
            "the persisted sidecar and in-memory index must agree on the later alias"
        );
    }

    /// Legacy 32-byte raw vault payloads (pre per-key-passphrase)
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

        // Pretend a pre-per-key-passphrase install wrote a raw 32-byte
        // payload under the canonical label, with a matching sidecar entry.
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
            public_key_bytes: Vec::new(),
        };
        kv.put(DetScope::Global, &meta_key_for(network, &address), &meta)
            .expect("seed sidecar");
        view.rehydrate_index().expect("rehydrate");

        // No passphrase needed, signing works.
        view.sign_with(&address, &[0x11u8; 32])
            .expect("legacy sign without passphrase");
    }

    /// TS-RT-02 / TS-EAGER-02 (import half) — an unprotected import writes the
    /// RAW 32 bytes under the canonical label (no `SingleKeyEntry` framing),
    /// the sidecar carries the public key for locked render, and the key signs.
    #[test]
    fn unprotected_import_writes_raw_32_bytes_not_framed() {
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
            .import_wif(known_wif(), Some("raw".into()))
            .expect("import");
        assert!(!imported.has_passphrase);
        assert!(
            !imported.public_key_bytes.is_empty(),
            "sidecar carries the locked-render public key"
        );

        // Vault payload is exactly the raw 32 bytes — no version-tag framing.
        let label = label_for_address(&imported.address);
        let raw = store
            .get(&single_key_namespace_id(), &label)
            .expect("get")
            .expect("present");
        assert_eq!(
            raw.expose_secret().len(),
            32,
            "raw, not a versioned envelope"
        );
        let priv_key = PrivateKey::from_wif(known_wif()).unwrap();
        assert_eq!(raw.expose_secret(), &priv_key.inner[..]);

        // Signs with no passphrase.
        view.sign_with(&imported.address, &[0x42u8; 32])
            .expect("raw key signs");
    }

    /// Dual-format sidecar upgrade — an OLD `ImportedKey` sidecar blob written WITHOUT the
    /// appended `public_key_bytes` (the pre-this-PR 5-field shape) is read back
    /// through the view's dual-format fallback: it does NOT vanish from the
    /// picker, its fields are preserved, and it is re-stored in the new shape.
    #[test]
    fn old_imported_key_blob_decodes_and_restores() {
        use crate::model::single_key::ImportedKeyV1;

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

        // Write the OLD 5-field shape directly, the way the base branch did.
        let address = "yTestImportedAddr".to_string();
        let key = meta_key_for(network, &address);
        let v1 = ImportedKeyV1 {
            address: address.clone(),
            alias: Some("legacy key".into()),
            network,
            has_passphrase: true,
            passphrase_hint: Some("the usual".into()),
        };
        kv.put(DetScope::Global, &key, &v1).expect("write old blob");

        // The view lists it (dual-format fallback) — not skipped.
        let listed = view.list_persisted();
        assert_eq!(listed.len(), 1, "old key must not vanish from the picker");
        let got = &listed[0];
        assert_eq!(got.address, address);
        assert_eq!(got.alias.as_deref(), Some("legacy key"));
        assert!(got.has_passphrase);
        assert_eq!(got.passphrase_hint.as_deref(), Some("the usual"));
        assert!(
            got.public_key_bytes.is_empty(),
            "no stored pubkey pre-migration"
        );

        // It was re-stored in the new shape: a direct new-shape decode succeeds.
        let direct: Option<ImportedKey> = kv
            .get(DetScope::Global, &key)
            .expect("direct new-shape read");
        assert_eq!(direct.expect("present").address, address);
    }
}
