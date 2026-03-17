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
