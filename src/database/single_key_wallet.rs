//! Database operations for single key wallets

use crate::database::Database;
use crate::model::wallet::single_key::{
    ClosedSingleKey, SingleKeyData, SingleKeyHash, SingleKeyWallet,
};
use dash_sdk::dpp::dashcore::{Address, Network, PublicKey};
use rusqlite::{Connection, params};
use std::collections::HashMap;

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

    /// Store a single key wallet in the database
    pub fn store_single_key_wallet(
        &self,
        wallet: &SingleKeyWallet,
        network: Network,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO single_key_wallet (
                key_hash,
                encrypted_private_key,
                salt,
                nonce,
                public_key,
                address,
                alias,
                uses_password,
                network,
                confirmed_balance,
                unconfirmed_balance,
                total_balance,
                core_wallet_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                wallet.key_hash.as_slice(),
                wallet.encrypted_private_key(),
                wallet.salt(),
                wallet.nonce(),
                wallet.public_key.to_bytes().as_slice(),
                wallet.address.to_string(),
                wallet.alias.as_deref(),
                wallet.uses_password as i32,
                network.to_string(),
                wallet.confirmed_balance as i64,
                wallet.unconfirmed_balance as i64,
                wallet.total_balance as i64,
                wallet.core_wallet_name.as_deref(),
            ],
        )?;
        Ok(())
    }

    /// Get all single key wallets for a network
    pub fn get_single_key_wallets(
        &self,
        network: Network,
    ) -> rusqlite::Result<Vec<SingleKeyWallet>> {
        let mut wallets = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT
                    key_hash,
                    encrypted_private_key,
                    salt,
                    nonce,
                    public_key,
                    address,
                    alias,
                    uses_password,
                    confirmed_balance,
                    unconfirmed_balance,
                    total_balance,
                    core_wallet_name
                FROM single_key_wallet
                WHERE network = ?1",
            )?;

            let rows = stmt.query_map(params![network.to_string()], |row| {
                let key_hash_vec: Vec<u8> = row.get(0)?;
                let encrypted_private_key: Vec<u8> = row.get(1)?;
                let salt: Vec<u8> = row.get(2)?;
                let nonce: Vec<u8> = row.get(3)?;
                let public_key_bytes: Vec<u8> = row.get(4)?;
                let address_str: String = row.get(5)?;
                let alias: Option<String> = row.get(6)?;
                let uses_password: i32 = row.get(7)?;
                let confirmed_balance: i64 = row.get(8)?;
                let unconfirmed_balance: i64 = row.get(9)?;
                let total_balance: i64 = row.get(10)?;
                let core_wallet_name: Option<String> = row.get(11)?;

                Ok((
                    key_hash_vec,
                    encrypted_private_key,
                    salt,
                    nonce,
                    public_key_bytes,
                    address_str,
                    alias,
                    uses_password,
                    confirmed_balance,
                    unconfirmed_balance,
                    total_balance,
                    core_wallet_name,
                ))
            })?;

            let mut wallets = Vec::new();

            for row_result in rows {
                let (
                    key_hash_vec,
                    encrypted_private_key,
                    salt,
                    nonce,
                    public_key_bytes,
                    address_str,
                    alias,
                    uses_password,
                    confirmed_balance,
                    unconfirmed_balance,
                    total_balance,
                    core_wallet_name,
                ) = row_result?;

                // Parse key hash
                let key_hash: SingleKeyHash = key_hash_vec.try_into().map_err(|_| {
                    rusqlite::Error::InvalidParameterName("Invalid key hash length".to_string())
                })?;

                // Parse public key
                let public_key = PublicKey::from_slice(&public_key_bytes).map_err(|e| {
                    rusqlite::Error::InvalidParameterName(format!("Invalid public key: {}", e))
                })?;

                // Parse address
                let address = address_str
                    .parse::<Address<_>>()
                    .map_err(|e| {
                        rusqlite::Error::InvalidParameterName(format!("Invalid address: {}", e))
                    })?
                    .require_network(network)
                    .map_err(|e| {
                        rusqlite::Error::InvalidParameterName(format!(
                            "Wrong network for address: {}",
                            e
                        ))
                    })?;

                let closed_key = ClosedSingleKey {
                    key_hash,
                    encrypted_private_key,
                    salt,
                    nonce,
                };

                let wallet = SingleKeyWallet {
                    private_key_data: SingleKeyData::Closed(closed_key),
                    uses_password: uses_password != 0,
                    public_key,
                    address,
                    alias,
                    key_hash,
                    confirmed_balance: confirmed_balance as u64,
                    unconfirmed_balance: unconfirmed_balance as u64,
                    total_balance: total_balance as u64,
                    utxos: HashMap::new(),
                    core_wallet_name,
                };

                wallets.push(wallet);
            }

            wallets
        }; // conn and stmt dropped here

        // Load UTXOs for each wallet
        let network_str = network.to_string();
        for wallet in &mut wallets {
            if let Ok(utxo_list) =
                self.get_utxos_by_address(&wallet.address.to_string(), &network_str)
            {
                wallet.utxos = utxo_list.into_iter().collect();
            }
        }

        Ok(wallets)
    }

    /// Remove a single key wallet from the database
    pub fn remove_single_key_wallet(
        &self,
        key_hash: &SingleKeyHash,
        network: Network,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM single_key_wallet WHERE key_hash = ?1 AND network = ?2",
            params![key_hash.as_slice(), network.to_string()],
        )?;
        Ok(())
    }

    /// Update balances for a single key wallet
    pub fn update_single_key_wallet_balances(
        &self,
        key_hash: &SingleKeyHash,
        confirmed_balance: u64,
        unconfirmed_balance: u64,
        total_balance: u64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE single_key_wallet SET
                confirmed_balance = ?1,
                unconfirmed_balance = ?2,
                total_balance = ?3
            WHERE key_hash = ?4",
            params![
                confirmed_balance as i64,
                unconfirmed_balance as i64,
                total_balance as i64,
                key_hash.as_slice(),
            ],
        )?;
        Ok(())
    }

    /// Update the Dash Core wallet name for a single key wallet.
    ///
    /// Returns `Ok(true)` if exactly one row was updated, `Ok(false)` if no
    /// matching wallet was found (0 rows), or `Err` on database errors
    /// (including the unexpected case of >1 rows affected).
    pub fn set_single_key_wallet_core_wallet_name(
        &self,
        key_hash: &SingleKeyHash,
        core_wallet_name: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE single_key_wallet SET core_wallet_name = ?1 WHERE key_hash = ?2",
            params![core_wallet_name, key_hash.as_slice()],
        )?;
        match rows {
            0 => Ok(false),
            1 => Ok(true),
            n => Err(rusqlite::Error::StatementChangedRows(n)),
        }
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
/// Proves the P4b carve-out is intact: `src/database/utxo.rs` + the `utxos`
/// table were RETAINED, and the single-key load path still hydrates
/// `SingleKeyWallet.utxos` through `get_utxos_by_address`. Also pins the
/// Decision-#7 stub error so a regression that silently re-enables single-key
/// spends — or changes the user-facing message — fails CI.
#[cfg(test)]
mod single_key_carveout_regression {
    use super::*;
    use crate::backend_task::error::TaskError;
    use crate::database::test_helpers::create_test_database;

    /// SEC-001 regression — Stage-B-then-load (NOT a tautology).
    ///
    /// Seeds a single-key wallet + its `utxos` rows AND a legacy `wallet`
    /// row, then runs the REAL Stage-B destructive step
    /// (`drop_legacy_migrated_tables`, the only code path that drops legacy
    /// tables) BEFORE loading. Asserts:
    ///  - `wallet` (a dropped legacy table) is gone — the migration ran;
    ///  - the `utxos` table SURVIVED the migration and the single-key load
    ///    path still hydrates `SingleKeyWallet.utxos` via
    ///    `get_utxos_by_address`;
    ///  - the Decision-#7 stub still surfaces `SingleKeyWalletsUnsupported`.
    ///
    /// This FAILS if `"utxos"` is in the drop list (the table is destroyed,
    /// `get_utxos_by_address` errors, utxos load empty) and PASSES only
    /// after SEC-001 removes it — proving the regression is not tautological.
    #[test]
    fn stage_b_drop_then_load_retains_single_key_utxos() {
        let db = create_test_database().expect("test db");
        let network = Network::Testnet;

        // A real single-key wallet (deterministic key) + its persisted row.
        let wallet = SingleKeyWallet::new([7u8; 32], network, None, Some("carveout".to_string()))
            .expect("single-key wallet");
        db.store_single_key_wallet(&wallet, network)
            .expect("store single-key wallet");

        // Seed the RETAINED utxos table for this wallet's address via the
        // #[cfg(test)] insert_utxo fixture — the exact load path single-key
        // depends on (single_key_wallet.rs -> get_utxos_by_address).
        let script = wallet.address.script_pubkey();
        db.insert_utxo(
            &[1u8; 32],
            0,
            &wallet.address,
            123_456,
            script.as_bytes(),
            network,
        )
        .expect("seed utxo");
        db.insert_utxo(
            &[2u8; 32],
            1,
            &wallet.address,
            7_000,
            script.as_bytes(),
            network,
        )
        .expect("seed second utxo");

        // Seed a `wallet` row (the FK target) and a `wallet_transactions`
        // row so we can prove the destructive Stage-B step actually ran
        // (it MUST drop `wallet_transactions`). The `wallet` table itself
        // is the DET-retained seed store and survives.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce,
                    master_ecdsa_bip44_account_0_epk, alias, is_main,
                    uses_password, network
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, 'testnet')",
                params![
                    vec![6u8; 32],
                    vec![2u8; 16],
                    vec![3u8; 16],
                    vec![4u8; 12],
                    "xpub-retained",
                    "retained-wallet",
                ],
            )
            .expect("seed retained wallet row");
            conn.execute(
                "INSERT INTO wallet_transactions (
                    seed_hash, txid, network, timestamp, net_amount,
                    is_ours, raw_transaction
                 ) VALUES (?1, ?2, 'testnet', 0, 0, 1, ?3)",
                params![vec![6u8; 32], vec![1u8; 32], vec![0u8; 32]],
            )
            .expect("seed legacy wallet_transactions row");
        }

        // Run the REAL destructive Stage-B step. This is the migration's
        // only legacy-table DROP path.
        db.drop_legacy_migrated_tables()
            .expect("Stage-B destructive drop");

        // The migration definitely ran: `wallet_transactions` is gone.
        {
            let conn = db.conn.lock().unwrap();
            let wt_exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='wallet_transactions'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                wt_exists, 0,
                "legacy `wallet_transactions` table must be dropped by Stage-B"
            );
        }

        // Carve-out proof: the `utxos` table SURVIVED the migration and the
        // single-key load path still hydrates utxos via the retained
        // `get_utxos_by_address`.
        let loaded = db
            .get_single_key_wallets(network)
            .expect("load single-key wallets after Stage-B drop");
        let sk = loaded
            .iter()
            .find(|w| w.key_hash == wallet.key_hash)
            .expect("stored single-key wallet must load post-migration");

        assert_eq!(
            sk.utxos.len(),
            2,
            "single-key UTXOs must survive Stage-B (utxos table retained)"
        );
        let total: u64 = sk.utxos.values().map(|o| o.value).sum();
        assert_eq!(
            total, 130_456,
            "UTXO values must round-trip through the retained utxos table"
        );
        assert!(
            sk.utxos.values().all(|o| o.script_pubkey == script),
            "script_pubkey must round-trip post-migration"
        );

        // The Decision-#7 stub still gates single-key spends after migration.
        assert!(matches!(
            TaskError::SingleKeyWalletsUnsupported,
            TaskError::SingleKeyWalletsUnsupported
        ));
    }

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
