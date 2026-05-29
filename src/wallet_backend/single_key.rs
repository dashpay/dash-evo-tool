//! Single-key (imported WIF) view backed by the upstream `SecretStore`.
//!
//! Each imported private key lives in the encrypted secret vault under the
//! label `single_key_priv.<base58_addr>`, scoped to a fixed per-backend
//! `WalletId` (`SINGLE_KEY_NAMESPACE_ID`). The dot separator replaces the
//! original design's colon because the upstream label allowlist is
//! `^[A-Za-z0-9._-]{1,64}$` and rejects colons (see CMT-006 in
//! `platform-wallet-storage`).
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
    FileStoreError, SecretBytes, SecretStore, WalletId as SecretWalletId,
};

use crate::backend_task::error::TaskError;
use crate::model::single_key::ImportedKey;

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
}

impl<'a> SingleKeyView<'a> {
    /// Parse a WIF-encoded private key, store its raw secret bytes in the
    /// encrypted vault under `single_key_priv.<address>`, and remember the
    /// derived `ImportedKey` metadata in the in-memory index. Idempotent
    /// on the same WIF — re-import overwrites the existing entry's alias
    /// and refreshes the stored bytes.
    pub fn import_wif(&self, wif: &str, alias: Option<String>) -> Result<ImportedKey, TaskError> {
        let priv_key = PrivateKey::from_wif(wif).map_err(|source| TaskError::InvalidWif {
            source: Box::new(source),
        })?;
        let secp = Secp256k1::new();
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&secp),
        };
        let address = Address::p2pkh(&pub_key, self.network);
        let address_str = address.to_string();

        let label = label_for_address(&address_str);
        let bytes = SecretBytes::from_slice(&priv_key.inner[..]);
        self.secret_store
            .set(&single_key_namespace_id(), &label, &bytes)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?;

        let imported = ImportedKey {
            address: address_str.clone(),
            alias,
            network: self.network,
        };
        self.index
            .write()
            .map_err(|_| TaskError::ImportedKeyNotFound)?
            .insert(address_str, imported.clone());
        Ok(imported)
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

    /// Forget the imported key at `address`: remove its index entry and
    /// delete its secret-store row. Idempotent — absent addresses are an
    /// `Ok(())`.
    pub fn forget(&self, address: &str) -> Result<(), TaskError> {
        let label = label_for_address(address);
        self.secret_store
            .delete(&single_key_namespace_id(), &label)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?;
        self.index
            .write()
            .map_err(|_| TaskError::ImportedKeyNotFound)?
            .remove(address);
        Ok(())
    }

    /// Sign a 32-byte message hash with the imported key registered at
    /// `address`. Reads the key bytes from the secret store on every
    /// call — the secret never lives in DET memory between signs. Pure
    /// ECDSA on secp256k1; no BIP-32 derivation is touched (TC-SK-008).
    pub fn sign_with(&self, address: &str, msg: &[u8; 32]) -> Result<Signature, TaskError> {
        let label = label_for_address(address);
        let secret = self
            .secret_store
            .get(&single_key_namespace_id(), &label)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })?
            .ok_or(TaskError::ImportedKeyNotFound)?;
        let bytes: [u8; 32] = secret
            .expose_secret()
            .try_into()
            .map_err(|_| TaskError::ImportedKeyNotFound)?;
        let sk = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_byte_array(&bytes)
            .map_err(|_| TaskError::ImportedKeyNotFound)?;
        let message = Message::from_digest(*msg);
        Ok(Secp256k1::new().sign_ecdsa(&message, &sk))
    }
}

/// Open or create the file-backed secret store at `path`. The parent
/// directory is created if missing; on Unix the vault file inherits its
/// initial mode from upstream's writer (the encrypted-file backend
/// refuses pre-existing modes looser than `0600`, so the secret-at-rest
/// floor is enforced at open time — see `FileStoreError::InsecurePermissions`).
///
/// The passphrase is a fixed, non-secret per-process constant: this PR
/// relies on file permissions for at-rest protection. A user-supplied
/// passphrase is a follow-up (T-SK-03 UX work). The choice is documented
/// in the ADR.
pub(crate) fn open_secret_store(path: &std::path::Path) -> Result<SecretStore, FileStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| FileStoreError::MalformedVault)?;
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
}
