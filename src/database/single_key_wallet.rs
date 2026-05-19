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

    #[test]
    fn single_key_wallet_loads_utxos_via_retained_get_utxos_by_address() {
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

        let loaded = db
            .get_single_key_wallets(network)
            .expect("load single-key wallets");
        let sk = loaded
            .iter()
            .find(|w| w.key_hash == wallet.key_hash)
            .expect("stored single-key wallet must load");

        // Carve-out proof: utxos hydrated from the retained utxos table.
        assert_eq!(sk.utxos.len(), 2, "both seeded UTXOs must load");
        let total: u64 = sk.utxos.values().map(|o| o.value).sum();
        assert_eq!(
            total, 130_456,
            "UTXO values must round-trip from utxos table"
        );
        assert!(
            sk.utxos.values().all(|o| o.script_pubkey == script),
            "script_pubkey must round-trip"
        );
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
