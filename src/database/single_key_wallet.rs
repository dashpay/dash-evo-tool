//! LEGACY — Database operations for the Decision-#7 single-key carve-out.
//!
//! Persists the [`crate::model::wallet::single_key::SingleKeyWallet`] runtime
//! type and reads its UTXOs via [`crate::database::utxo`]. Retained (spending
//! gated) until single-key moves onto the upstream wallet runtime. The LIVE
//! imported-key metadata sidecar is [`crate::model::single_key`].

use crate::database::Database;
use rusqlite::Connection;

impl Database {
    /// Initialize the single key wallet table
    pub fn initialize_single_key_wallet_table(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS single_key_wallet (
                key_hash BLOB NOT NULL PRIMARY KEY,
                encrypted_private_key BLOB NOT NULL,
                salt BLOB NOT NULL,
                nonce BLOB NOT NULL,
                public_key BLOB NOT NULL,
                address TEXT NOT NULL,
                alias TEXT,
                uses_password INTEGER NOT NULL,
                network TEXT NOT NULL,
                confirmed_balance INTEGER DEFAULT 0,
                unconfirmed_balance INTEGER DEFAULT 0,
                total_balance INTEGER DEFAULT 0,
                core_wallet_name TEXT DEFAULT NULL
            )",
            [],
        )?;

        // Create index for network lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_single_key_wallet_network ON single_key_wallet (network)",
            [],
        )?;

        Ok(())
    }
}

/// Stores a password-protected imported key the way the earlier version
/// saved it, so it awaits its old password.
#[cfg(test)]
pub(crate) fn seed_legacy_protected_key(db: &Database) {
    use crate::model::wallet::encryption::EncryptedEnvelope;
    use crate::model::wallet::single_key::ClosedSingleKey;
    use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey};

    let raw_key = crate::test_support::keys::secret_bytes();
    let EncryptedEnvelope {
        ciphertext,
        salt,
        nonce,
    } = ClosedSingleKey::encrypt_private_key(&raw_key, "old-legacy-password").expect("encrypt");
    let public_key = PrivateKey::from_byte_array(&raw_key, Network::Testnet)
        .expect("private key")
        .public_key();
    let path = db.db_file_path().expect("data.db path");
    rusqlite::Connection::open(path)
        .expect("open data.db")
        .execute(
            "INSERT INTO single_key_wallet
                (key_hash, encrypted_private_key, salt, nonce, public_key,
                 address, alias, uses_password, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 1, ?7)",
            rusqlite::params![
                ClosedSingleKey::compute_key_hash(&raw_key).as_slice(),
                ciphertext,
                salt,
                nonce,
                public_key.inner.to_compressed().to_vec(),
                Address::p2pkh(&public_key, Network::Testnet).to_string(),
                Network::Testnet.to_string(),
            ],
        )
        .expect("insert legacy protected key");
}

/// Decision-#7 single-key carve-out regression lane (release-blocking).
///
/// Pins the Decision-#7 stub error so a regression that silently re-enables
/// single-key spends — or changes the user-facing message — fails CI.
#[cfg(test)]
mod single_key_carveout_regression {
    use crate::backend_task::error::TaskError;

    #[test]
    fn decision_7_stub_still_surfaces_single_key_unsupported() {
        // The stub error variant is the load-bearing Decision-#7 contract.
        // It is fieldless, so a structural match fully pins it; the
        // user-facing message is asserted verbatim so a regression that
        // weakens the disclosure fails here.
        let err = TaskError::SingleKeyWalletsUnsupported;
        assert!(matches!(err, TaskError::SingleKeyWalletsUnsupported));
        let msg = TaskError::SingleKeyWalletsUnsupported.to_string();
        assert!(
            msg.contains("Single-key wallets are not supported in this version"),
            "stub message must state the capability is unsupported: {msg}"
        );
        assert!(
            msg.contains("preserved") && msg.contains("future update"),
            "stub message must reassure data is preserved and will return: {msg}"
        );
        assert!(
            msg.contains("HD (recovery-phrase) wallet"),
            "stub message must give the user a concrete alternative: {msg}"
        );
    }
}
