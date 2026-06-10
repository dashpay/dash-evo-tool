//! T-SK-03 — password-protected single-key restore.
//!
//! The FinishUnwire migration ([`super::finish_unwire`]) preserves but
//! cannot migrate `uses_password=1` legacy `single_key_wallet` rows: each
//! is encrypted under the user's OLD per-wallet password, which the
//! migration code path does not have. Those rows are invisible until the
//! user supplies that password.
//!
//! This module is the bridge. Given the legacy password, it:
//!   1. legacy-decrypts the row's blob (the same AES-GCM + Argon2id scheme
//!      `model::wallet::single_key` used to write it),
//!   2. re-imports the recovered key into the MODERN secret-store vault via
//!      [`SingleKeyView::import_wif_with_passphrase`] (re-protected under a
//!      passphrase the user chooses), and
//!   3. leaves the key visible/usable in the picker.
//!
//! ## Security (Smythe review focus)
//! - **S1** — the recovered 32-byte key and the derived WIF are wrapped in
//!   [`Zeroizing`], never logged, never `Debug`-printed, dropped promptly.
//! - **S2** — a wrong legacy password and a corrupt blob both surface the
//!   same generic [`TaskError::SingleKeyPassphraseIncorrect`]: no oracle
//!   that distinguishes "close" passwords or aids a brute force.
//! - **S4** — re-encryption goes through `import_wif_with_passphrase` →
//!   `SingleKeyEntry::protected` → `encrypt_message`, which draws a FRESH
//!   random AES-GCM nonce + Argon2 salt every call (never reused).
//! - **S5** — the recovered key is re-imported compressed-only (the import
//!   path rejects uncompressed WIFs), so the derived address is identical
//!   before and after restore; a stability check enforces it.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::wallet_backend::single_key::ImportPassphrase;

use super::finish_unwire::MigrationError;

/// One legacy `uses_password=1` row still awaiting restore. Lists only
/// non-secret display fields — never the ciphertext, salt, or nonce — so
/// it is safe to hold in UI state and render in a banner/dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingProtectedRestore {
    /// Base58 P2PKH address of the protected key. Stable identifier the
    /// UI shows and the restore call routes by.
    pub address: String,
    /// User-chosen nickname from the legacy row, if any.
    pub alias: Option<String>,
    /// Network the address is valid on.
    pub network: Network,
}

/// Raw protected-row crypto fields read from the legacy table. Internal
/// only — never leaves this module, never logged. `Debug` is intentionally
/// NOT derived so the encrypted bytes cannot leak through `{:?}`.
struct LegacyProtectedBlob {
    address: String,
    alias: Option<String>,
    encrypted_private_key: Vec<u8>,
    salt: Vec<u8>,
    nonce: Vec<u8>,
}

/// List every protected legacy single-key row for the active network that
/// has NOT yet been restored into the modern vault. Restored = a modern
/// single-key sidecar entry exists at the same address.
///
/// Used by the boot/wallet-screen banner to decide whether to offer the
/// restore flow and how many keys are waiting.
pub fn list_pending_protected_restores(
    app_context: &Arc<AppContext>,
) -> Result<Vec<PendingProtectedRestore>, TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;
    let Some(path) = app_context.db.db_file_path() else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }

    // Addresses already present in the modern index are restored and must
    // be filtered out, so a re-opened banner does not re-offer them.
    let already_present: std::collections::BTreeSet<String> = backend
        .single_key()
        .list()
        .into_iter()
        .map(|k| k.address)
        .collect();

    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path.to_string_lossy().to_string(),
        source: e,
    })?;
    let rows = read_pending_protected_rows(&conn, app_context.network)?;
    Ok(rows
        .into_iter()
        .filter(|r| !already_present.contains(&r.address))
        .collect())
}

/// Pure read of protected pending rows from `conn` for `network`. Returns
/// only the non-secret display descriptor. A missing table is not an
/// error (fresh install / already cleaned up).
fn read_pending_protected_rows(
    conn: &Connection,
    network: Network,
) -> Result<Vec<PendingProtectedRestore>, MigrationError> {
    if !table_exists(conn, "single_key_wallet")? {
        return Ok(Vec::new());
    }
    let sql = "SELECT address, alias FROM single_key_wallet \
               WHERE network = ?1 AND uses_password = 1";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;
    let rows = stmt
        .query_map(rusqlite::params![network.to_string()], |row| {
            let address: String = row.get(0)?;
            let alias: Option<String> = row.get(1)?;
            Ok(PendingProtectedRestore {
                address,
                alias,
                network,
            })
        })
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;
    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok(r) => out.push(r),
            Err(e) => {
                tracing::warn!(
                    target = "migration::single_key_restore",
                    error = ?e,
                    "Skipping unreadable protected single_key_wallet row while listing",
                );
            }
        }
    }
    Ok(out)
}

/// Restore one password-protected single-key row into the modern vault.
///
/// `legacy_password` decrypts the legacy blob; `new_passphrase` re-protects
/// the key in the modern vault (it may be the same string). On success the
/// key becomes listable/usable for the active session and survives a cold
/// boot. Idempotent failure-safe: nothing is written until the legacy
/// decrypt succeeds, and the legacy row is left untouched so a wrong
/// password leaves it restorable.
///
/// Returns the derived address (== the legacy address; enforced) so the UI
/// can confirm which key was restored.
///
/// See the module docs for the S1–S5 security mapping.
pub fn restore_protected_single_key(
    app_context: &Arc<AppContext>,
    address: &str,
    legacy_password: &str,
    new_passphrase: ImportPassphrase,
) -> Result<String, TaskError> {
    let backend = app_context
        .wallet_backend()
        .map_err(|_| MigrationError::WalletBackendUnavailable)?;
    let path = app_context
        .db
        .db_file_path()
        .ok_or(TaskError::ProtectedSingleKeyRestoreTargetMissing)?;
    if !path.exists() {
        return Err(TaskError::ProtectedSingleKeyRestoreTargetMissing);
    }
    let conn = Connection::open(&path).map_err(|e| MigrationError::LegacyDbOpen {
        path: path.to_string_lossy().to_string(),
        source: e,
    })?;

    let network = app_context.network;
    let blob = read_protected_blob(&conn, address, network)?
        .ok_or(TaskError::ProtectedSingleKeyRestoreTargetMissing)?;

    // S1 — recovered key bytes wrapped in `Zeroizing`; they wipe on drop.
    // S2 — a wrong password and a corrupt/short blob both collapse to the
    // same generic `SingleKeyPassphraseIncorrect`, leaking no oracle.
    let wif = legacy_decrypt_to_wif(&blob, legacy_password, network)?;

    // S5 — re-import compressed-only (the import path rejects uncompressed
    // WIFs) so the derived address is stable. Confirm it matches the legacy
    // address before trusting the row; a mismatch means corruption.
    let derived = derive_p2pkh_address(&wif, network)?;
    if derived != blob.address {
        tracing::warn!(
            target = "migration::single_key_restore",
            "Restored single-key address does not match the legacy row; aborting",
        );
        return Err(TaskError::SingleKeyCryptoFailure);
    }

    // S4 — `import_wif_with_passphrase` re-encrypts under a FRESH random
    // nonce + salt (via `encrypt_message`). Re-import the recovered key,
    // preserving the legacy alias unless the row carried none.
    let imported = app_context.import_single_key_wif(&wif, blob.alias.clone(), new_passphrase)?;
    debug_assert_eq!(imported.0.address, blob.address);

    tracing::info!(
        target = "migration::single_key_restore",
        address = %blob.address,
        "Restored a password-protected single-key wallet into the modern vault",
    );
    let _ = backend; // backend presence already validated above.
    Ok(blob.address)
}

/// Read the full protected-row crypto fields for `address` on `network`.
/// `None` when no matching `uses_password=1` row exists. Secret fields are
/// loaded only here and never logged.
fn read_protected_blob(
    conn: &Connection,
    address: &str,
    network: Network,
) -> Result<Option<LegacyProtectedBlob>, MigrationError> {
    if !table_exists(conn, "single_key_wallet")? {
        return Ok(None);
    }
    let sql = "SELECT address, alias, encrypted_private_key, salt, nonce \
               FROM single_key_wallet \
               WHERE network = ?1 AND address = ?2 AND uses_password = 1";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;
    let mut rows = stmt
        .query(rusqlite::params![network.to_string(), address])
        .map_err(|e| MigrationError::LegacyDbRead {
            table: "single_key_wallet",
            source: e,
        })?;
    let Some(row) = rows.next().map_err(|e| MigrationError::LegacyDbRead {
        table: "single_key_wallet",
        source: e,
    })?
    else {
        return Ok(None);
    };
    let read_err = |e: rusqlite::Error| MigrationError::LegacyDbRead {
        table: "single_key_wallet",
        source: e,
    };
    Ok(Some(LegacyProtectedBlob {
        address: row.get(0).map_err(read_err)?,
        alias: row.get(1).map_err(read_err)?,
        encrypted_private_key: row.get(2).map_err(read_err)?,
        salt: row.get(3).map_err(read_err)?,
        nonce: row.get(4).map_err(read_err)?,
    }))
}

/// S1/S2 chokepoint: legacy-decrypt the protected blob and return the
/// compressed WIF, wrapped in [`Zeroizing`]. Every decrypt failure
/// (wrong password, malformed AES-GCM, wrong key length) collapses to the
/// generic [`TaskError::SingleKeyPassphraseIncorrect`] — no oracle.
fn legacy_decrypt_to_wif(
    blob: &LegacyProtectedBlob,
    legacy_password: &str,
    network: Network,
) -> Result<Zeroizing<String>, TaskError> {
    use crate::model::wallet::single_key::ClosedSingleKey;

    let closed = ClosedSingleKey {
        // `key_hash` is not consulted by `decrypt_private_key`; a zero
        // placeholder is fine and never persisted.
        key_hash: [0u8; 32],
        encrypted_private_key: blob.encrypted_private_key.clone(),
        salt: blob.salt.clone(),
        nonce: blob.nonce.clone(),
    };
    // S1 — recovered raw bytes wrapped immediately so they wipe on drop.
    // The legacy `decrypt_private_key` returns a bare `[u8; 32]`; box it
    // in `Zeroizing` at the boundary and never let a bare copy linger.
    let raw = Zeroizing::new(closed.decrypt_private_key(legacy_password).map_err(|_| {
        // S2 — do not branch on the failure shape; one generic error.
        tracing::debug!(
            target = "migration::single_key_restore",
            "Legacy single-key decrypt failed (wrong password or corrupt blob)",
        );
        TaskError::SingleKeyPassphraseIncorrect
    })?);

    let priv_key = PrivateKey::from_byte_array(&raw, network)
        .map_err(|_| TaskError::SingleKeyPassphraseIncorrect)?;
    // `to_wif()` on a `PrivateKey` from `from_byte_array` is compressed,
    // matching the import path's compressed-only requirement (S5).
    Ok(Zeroizing::new(priv_key.to_wif()))
}

/// Derive the P2PKH address for `wif` on `network`. Used to confirm the
/// restored key's address matches the legacy row before trusting it (S5).
fn derive_p2pkh_address(wif: &str, network: Network) -> Result<String, TaskError> {
    let priv_key = PrivateKey::from_wif(wif).map_err(|source| TaskError::InvalidWif {
        source: Box::new(source),
    })?;
    let secp = Secp256k1::new();
    let pub_key = PublicKey {
        compressed: priv_key.compressed,
        inner: priv_key.inner.public_key(&secp),
    };
    Ok(Address::p2pkh(&pub_key, network).to_string())
}

/// `SELECT 1 FROM sqlite_master` table-existence probe. Missing table is
/// not an error — fresh install / already cleaned up.
fn table_exists(conn: &Connection, name: &str) -> Result<bool, MigrationError> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        rusqlite::params![name],
        |row| row.get::<_, i64>(0).map(|c| c > 0),
    )
    .map_err(|e| MigrationError::LegacyDbRead {
        table: "single_key_wallet",
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::single_key::SingleKeyWallet;

    /// Build a legacy table and seed one protected row encrypted under
    /// `password`, returning its derived address. Mirrors the legacy
    /// writer's protected shape (salt + nonce + AES-GCM ciphertext).
    fn seed_protected_row(
        conn: &Connection,
        raw_key: &[u8; 32],
        password: &str,
        alias: Option<&str>,
        network: Network,
    ) -> String {
        use crate::model::wallet::single_key::ClosedSingleKey;
        conn.execute(
            "CREATE TABLE single_key_wallet (
                key_hash BLOB NOT NULL PRIMARY KEY,
                encrypted_private_key BLOB NOT NULL,
                salt BLOB NOT NULL,
                nonce BLOB NOT NULL,
                public_key BLOB NOT NULL,
                address TEXT NOT NULL,
                alias TEXT,
                uses_password INTEGER NOT NULL,
                network TEXT NOT NULL
            )",
            [],
        )
        .expect("create legacy table");

        let (ciphertext, salt, nonce) =
            ClosedSingleKey::encrypt_private_key(raw_key, password).expect("encrypt");
        let priv_key = PrivateKey::from_byte_array(raw_key, network).expect("priv");
        let secp = Secp256k1::new();
        let pub_key = PublicKey {
            compressed: priv_key.compressed,
            inner: priv_key.inner.public_key(&secp),
        };
        let address = Address::p2pkh(&pub_key, network).to_string();
        let key_hash = ClosedSingleKey::compute_key_hash(raw_key);
        conn.execute(
            "INSERT INTO single_key_wallet
                (key_hash, encrypted_private_key, salt, nonce, public_key,
                 address, alias, uses_password, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
            rusqlite::params![
                key_hash.as_slice(),
                ciphertext,
                salt,
                nonce,
                pub_key.inner.serialize().to_vec(),
                address,
                alias,
                network.to_string(),
            ],
        )
        .expect("insert protected row");
        address
    }

    /// The correct legacy password decrypts the blob to the compressed
    /// WIF, and the derived address matches what the legacy row stored —
    /// the no-divergence guarantee the restore round-trip depends on (S5).
    #[test]
    fn correct_password_recovers_compressed_wif_with_stable_address() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("db");
        let mut raw = [0u8; 32];
        raw[31] = 7;
        let address = seed_protected_row(
            &conn,
            &raw,
            "legacy-secret",
            Some("savings"),
            Network::Testnet,
        );

        let blob = read_protected_blob(&conn, &address, Network::Testnet)
            .expect("read")
            .expect("present");
        let wif = legacy_decrypt_to_wif(&blob, "legacy-secret", Network::Testnet).expect("decrypt");

        // Compressed WIF round-trips to the SAME address (S5).
        let parsed = PrivateKey::from_wif(&wif).expect("wif");
        assert!(parsed.compressed, "restored WIF must be compressed");
        assert_eq!(
            derive_p2pkh_address(&wif, Network::Testnet).unwrap(),
            address,
            "restored address must equal the legacy address"
        );

        // And the recovered key bytes equal the original.
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&parsed.inner[..]);
        assert_eq!(buf, raw, "recovered key must equal the original");
    }

    /// S2 — a wrong legacy password surfaces the generic
    /// `SingleKeyPassphraseIncorrect`, identical to the variant a corrupt
    /// blob would produce: no oracle that distinguishes the two.
    #[test]
    fn wrong_password_is_generic_failure_no_oracle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("db");
        let raw = [0x11u8; 32];
        let address = seed_protected_row(&conn, &raw, "right-password", None, Network::Testnet);

        let blob = read_protected_blob(&conn, &address, Network::Testnet)
            .expect("read")
            .expect("present");

        let err = legacy_decrypt_to_wif(&blob, "wrong-password", Network::Testnet)
            .expect_err("wrong password must fail");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseIncorrect),
            "expected generic SingleKeyPassphraseIncorrect, got {err:?}"
        );

        // A blob with corrupted ciphertext yields the SAME variant — the
        // failure shape must not let an attacker tell "wrong password"
        // from "damaged data".
        let mut corrupt = blob;
        corrupt.encrypted_private_key[0] ^= 0xFF;
        let err2 = legacy_decrypt_to_wif(&corrupt, "right-password", Network::Testnet)
            .expect_err("corrupt blob must fail");
        assert!(
            matches!(err2, TaskError::SingleKeyPassphraseIncorrect),
            "corrupt blob must surface the same generic error, got {err2:?}"
        );
    }

    /// Listing pending restores returns only protected rows for the active
    /// network and never their secret fields.
    #[test]
    fn pending_list_returns_protected_rows_for_network_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("db");
        let raw = [0x22u8; 32];
        let address = seed_protected_row(&conn, &raw, "pw", Some("nick"), Network::Testnet);

        let pending = read_pending_protected_rows(&conn, Network::Testnet).expect("list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].address, address);
        assert_eq!(pending[0].alias.as_deref(), Some("nick"));
        assert_eq!(pending[0].network, Network::Testnet);

        // A different network sees nothing.
        let other = read_pending_protected_rows(&conn, Network::Mainnet).expect("list mainnet");
        assert!(other.is_empty(), "protected rows are per-network");
    }

    /// Missing table → empty pending list and `None` blob (fresh install).
    #[test]
    fn missing_table_yields_empty_pending_and_none_blob() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("empty.db")).expect("db");
        assert!(
            read_pending_protected_rows(&conn, Network::Testnet)
                .expect("list")
                .is_empty()
        );
        assert!(
            read_protected_blob(&conn, "yAny", Network::Testnet)
                .expect("blob")
                .is_none()
        );
    }

    /// The recovered WIF re-imports cleanly through the modern import path
    /// and the resulting display wallet keeps the same address — closing
    /// the legacy→modern bridge end to end at the crypto layer (S4/S5).
    /// Exercises the import path against a bare `SingleKeyView` (no full
    /// `AppContext`).
    #[test]
    fn recovered_wif_reimports_with_stable_address() {
        use crate::wallet_backend::single_key::{SingleKeyView, open_secret_store};

        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("data.db")).expect("db");
        let mut raw = [0u8; 32];
        raw[30] = 5;
        let address = seed_protected_row(&conn, &raw, "legacy-pw", Some("vault"), Network::Testnet);

        let blob = read_protected_blob(&conn, &address, Network::Testnet)
            .expect("read")
            .expect("present");
        let wif = legacy_decrypt_to_wif(&blob, "legacy-pw", Network::Testnet).expect("decrypt");

        let store =
            Arc::new(open_secret_store(&dir.path().join("secrets.pwsvault")).expect("vault"));
        let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
        let view = SingleKeyView::from_views(&store, &index, Network::Testnet, None);

        // Re-protect under a NEW passphrase — the round-trip must keep the
        // address identical (S5) and store an encrypted (not raw) entry (S4).
        let imported = view
            .import_wif_with_passphrase(
                &wif,
                blob.alias.clone(),
                ImportPassphrase {
                    passphrase: Some(Zeroizing::new("new-strong-passphrase".into())),
                    hint: None,
                },
            )
            .expect("re-import");
        assert_eq!(
            imported.address, address,
            "re-imported address must be stable"
        );
        assert!(
            imported.has_passphrase,
            "re-import must be passphrase-protected"
        );

        // And the rebuilt display wallet derives the same address.
        let wallet = SingleKeyWallet::from_wif(&wif, None, None).expect("rebuild");
        assert_eq!(wallet.address.to_string(), address);
    }
}
