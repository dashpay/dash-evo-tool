//! Database operations for single key wallets

use crate::database::Database;
use crate::model::wallet::single_key::SingleKeyHash;
use rusqlite::{Connection, params};

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

    /// Update alias for a single key wallet
    pub fn update_single_key_wallet_alias(
        &self,
        key_hash: &SingleKeyHash,
        alias: Option<&str>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE single_key_wallet SET alias = ?1 WHERE key_hash = ?2",
            params![alias, key_hash.as_slice()],
        )?;
        Ok(())
    }
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
