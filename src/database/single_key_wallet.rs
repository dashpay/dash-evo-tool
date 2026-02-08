//! Database operations for single key wallets

use crate::database::Database;
use crate::lock_helper::MutexExt;
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
                total_balance INTEGER DEFAULT 0
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
        let conn = self.conn.lock_or_recover();
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
                total_balance
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            let conn = self.conn.lock_or_recover();
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
                    total_balance
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
        let conn = self.conn.lock_or_recover();
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
        let conn = self.conn.lock_or_recover();
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

    /// Update alias for a single key wallet
    pub fn update_single_key_wallet_alias(
        &self,
        key_hash: &SingleKeyHash,
        alias: Option<&str>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock_or_recover();
        conn.execute(
            "UPDATE single_key_wallet SET alias = ?1 WHERE key_hash = ?2",
            params![alias, key_hash.as_slice()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;
    use crate::model::wallet::single_key::SingleKeyWallet;
    use dash_sdk::dpp::dashcore::Network;

    fn create_test_wallet() -> SingleKeyWallet {
        // Generate a deterministic test key
        let private_key_bytes: [u8; 32] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];
        SingleKeyWallet::new(
            private_key_bytes,
            Network::Testnet,
            None,
            Some("Test Wallet".to_string()),
        )
        .expect("Failed to create test wallet")
    }

    fn create_test_wallet_with_password() -> SingleKeyWallet {
        let private_key_bytes: [u8; 32] = [
            0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e,
            0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c,
            0x3d, 0x3e, 0x3f, 0x40,
        ];
        SingleKeyWallet::new(
            private_key_bytes,
            Network::Testnet,
            Some("test_password"),
            Some("Password Wallet".to_string()),
        )
        .expect("Failed to create test wallet with password")
    }

    #[test]
    fn test_store_and_retrieve_wallet() {
        let db = create_test_database().unwrap();
        let wallet = create_test_wallet();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();

        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert_eq!(wallets.len(), 1);

        let retrieved = &wallets[0];
        assert_eq!(retrieved.key_hash, wallet.key_hash);
        assert_eq!(retrieved.public_key, wallet.public_key);
        assert_eq!(retrieved.address, wallet.address);
        assert_eq!(retrieved.alias, Some("Test Wallet".to_string()));
        assert!(!retrieved.uses_password);
    }

    #[test]
    fn test_store_wallet_with_password() {
        let db = create_test_database().unwrap();
        let wallet = create_test_wallet_with_password();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();

        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert_eq!(wallets.len(), 1);
        assert!(wallets[0].uses_password);
        assert_eq!(wallets[0].alias, Some("Password Wallet".to_string()));
    }

    #[test]
    fn test_network_isolation() {
        let db = create_test_database().unwrap();
        let wallet = create_test_wallet();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();

        let testnet = db.get_single_key_wallets(Network::Testnet).unwrap();
        let mainnet = db.get_single_key_wallets(Network::Dash).unwrap();
        assert_eq!(testnet.len(), 1);
        assert_eq!(mainnet.len(), 0);
    }

    #[test]
    fn test_update_balances() {
        let db = create_test_database().unwrap();
        let wallet = create_test_wallet();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();
        db.update_single_key_wallet_balances(&wallet.key_hash, 1_000_000, 500_000, 1_500_000)
            .unwrap();

        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert_eq!(wallets[0].confirmed_balance, 1_000_000);
        assert_eq!(wallets[0].unconfirmed_balance, 500_000);
        assert_eq!(wallets[0].total_balance, 1_500_000);
    }

    #[test]
    fn test_update_alias() {
        let db = create_test_database().unwrap();
        let wallet = create_test_wallet();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();
        db.update_single_key_wallet_alias(&wallet.key_hash, Some("Renamed Wallet"))
            .unwrap();

        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert_eq!(wallets[0].alias, Some("Renamed Wallet".to_string()));

        // Set alias to None
        db.update_single_key_wallet_alias(&wallet.key_hash, None)
            .unwrap();
        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert!(wallets[0].alias.is_none());
    }

    #[test]
    fn test_remove_wallet() {
        let db = create_test_database().unwrap();
        let wallet = create_test_wallet();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();
        assert_eq!(
            db.get_single_key_wallets(Network::Testnet).unwrap().len(),
            1
        );

        db.remove_single_key_wallet(&wallet.key_hash, Network::Testnet)
            .unwrap();
        assert_eq!(
            db.get_single_key_wallets(Network::Testnet).unwrap().len(),
            0
        );
    }

    #[test]
    fn test_upsert_replaces_existing() {
        let db = create_test_database().unwrap();
        let mut wallet = create_test_wallet();

        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();

        // Update alias and re-store (INSERT OR REPLACE)
        wallet.alias = Some("Updated Alias".to_string());
        db.store_single_key_wallet(&wallet, Network::Testnet)
            .unwrap();

        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].alias, Some("Updated Alias".to_string()));
    }

    #[test]
    fn test_multiple_wallets() {
        let db = create_test_database().unwrap();
        let w1 = create_test_wallet();
        let w2 = create_test_wallet_with_password();

        db.store_single_key_wallet(&w1, Network::Testnet).unwrap();
        db.store_single_key_wallet(&w2, Network::Testnet).unwrap();

        let wallets = db.get_single_key_wallets(Network::Testnet).unwrap();
        assert_eq!(wallets.len(), 2);
    }
}
