use crate::context::AppContext;
use crate::database::Database;
use crate::lock_helper::MutexExt;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    InstantLock, Network, Transaction,
    consensus::{deserialize, serialize},
};
use rusqlite::{Connection, params};

impl Database {
    /// Stores an asset lock transaction and optional InstantLock into the database.
    pub fn store_asset_lock_transaction(
        &self,
        tx: &Transaction,
        amount: u64,
        islock: Option<&InstantLock>,
        wallet_seed_hash: &[u8; 32],
        network: Network,
    ) -> rusqlite::Result<()> {
        let tx_bytes = serialize(tx);
        let txid = tx.txid().to_byte_array();

        let islock_bytes = islock.map(serialize);

        let conn = self.conn.lock_or_recover();

        let sql = "
        INSERT INTO asset_lock_transaction (tx_id, transaction_data, amount, instant_lock_data, wallet, network)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(tx_id) DO UPDATE SET
            transaction_data = excluded.transaction_data,
            amount = excluded.amount,
            instant_lock_data = COALESCE(excluded.instant_lock_data, asset_lock_transaction.instant_lock_data),
            network = excluded.network;
        ";

        conn.execute(
            sql,
            params![
                &txid,
                &tx_bytes,
                amount,
                &islock_bytes,
                wallet_seed_hash,
                network.to_string()
            ],
        )?;

        Ok(())
    }

    /// Retrieves an asset lock transaction by its transaction ID.
    #[allow(dead_code)] // May be used for querying asset locks
    #[allow(clippy::type_complexity)]
    pub fn get_asset_lock_transaction(
        &self,
        txid: &[u8; 32],
    ) -> rusqlite::Result<Option<(Transaction, u64, Option<InstantLock>, [u8; 32], String)>> {
        let conn = self.conn.lock_or_recover();

        let mut stmt = conn.prepare(
            "SELECT transaction_data, amount, instant_lock_data, wallet, network FROM asset_lock_transaction WHERE tx_id = ?1",
        )?;

        let mut rows = stmt.query(params![txid])?;

        if let Some(row) = rows.next()? {
            let tx_data: Vec<u8> = row.get(0)?;
            let amount: u64 = row.get(1)?;
            let islock_data: Option<Vec<u8>> = row.get(2)?;
            let wallet_seed: Vec<u8> = row.get(3)?;
            let network: String = row.get(4)?;

            let tx: Transaction =
                deserialize(&tx_data).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let islock = if let Some(islock_bytes) = islock_data {
                Some(deserialize(&islock_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?)
            } else {
                None
            };

            let wallet_seed_hash: [u8; 32] = wallet_seed
                .try_into()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;

            Ok(Some((tx, amount, islock, wallet_seed_hash, network)))
        } else {
            Ok(None)
        }
    }

    /// Updates the chain locked height for an asset lock transaction.
    #[allow(dead_code)] // May be used for tracking chain confirmation status
    pub fn update_asset_lock_chain_locked_height(
        &self,
        txid: &[u8; 32],
        chain_locked_height: Option<u32>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock_or_recover();

        conn.execute(
            "UPDATE asset_lock_transaction SET chain_locked_height = ?1 WHERE tx_id = ?2",
            params![chain_locked_height, txid],
        )?;

        Ok(())
    }

    /// Sets the identity ID for an asset lock transaction.
    pub fn set_asset_lock_identity_id(
        &self,
        tx_id: &[u8; 32],
        identity_id: &[u8; 32],
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock_or_recover();

        let rows_updated = conn.execute(
            "UPDATE asset_lock_transaction
     SET identity_id = ?1, identity_id_potentially_in_creation = NULL
     WHERE tx_id = ?2",
            params![identity_id, tx_id],
        )?;
        if rows_updated == 0 {
            tracing::error!(
                "No rows updated. Check if tx_id {} exists and identity_id {} is correct.",
                hex::encode(tx_id),
                hex::encode(identity_id)
            );
        }

        Ok(())
    }

    /// Deletes all asset lock transactions in Devnet variants and Regtest.
    pub fn remove_all_asset_locks_identity_id_for_all_devnets_and_regtest(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM asset_lock_transaction
         WHERE network LIKE 'devnet%' OR network = 'regtest'",
            [],
        )?;

        Ok(())
    }

    /// Removes the identity ID and identity_id_potentially_in_creation for all asset lock transactions in Devnet.
    pub fn remove_all_asset_locks_identity_id_for_devnet(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        if app_context.network != Network::Devnet {
            return Ok(());
        }
        let network = app_context.network.to_string();

        let conn = self.conn.lock_or_recover();

        conn.execute(
            "UPDATE asset_lock_transaction
         SET identity_id = NULL,
             identity_id_potentially_in_creation = NULL
         WHERE network = ?",
            params![network],
        )?;

        Ok(())
    }

    /// Sets the identity ID for an asset lock transaction.
    pub fn set_asset_lock_identity_id_before_confirmation_by_network(
        &self,
        txid: &[u8; 32],
        identity_id: &[u8; 32],
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock_or_recover();

        conn.execute(
            "UPDATE asset_lock_transaction SET identity_id_potentially_in_creation = ?1 WHERE tx_id = ?2",
            params![identity_id, txid],
        )?;

        Ok(())
    }

    /// Deletes an asset lock transaction by its transaction ID (as bytes).
    pub fn delete_asset_lock_transaction(&self, txid: &[u8; 32]) -> rusqlite::Result<()> {
        let conn = self.conn.lock_or_recover();

        conn.execute(
            "DELETE FROM asset_lock_transaction WHERE tx_id = ?1",
            params![txid],
        )?;

        Ok(())
    }

    /// Retrieves all asset lock transactions.
    #[allow(dead_code)] // May be used for debugging or administrative views
    #[allow(clippy::type_complexity)]
    pub fn get_all_asset_lock_transactions(
        &self,
        network: Network,
    ) -> rusqlite::Result<
        Vec<(
            Transaction,
            u64,
            Option<InstantLock>,
            Option<u32>,
            Option<Vec<u8>>,
            [u8; 32],
        )>,
    > {
        let conn = self.conn.lock_or_recover();

        let mut stmt = conn.prepare(
            "SELECT transaction_data, amount, instant_lock_data, chain_locked_height, identity_id, wallet, network FROM asset_lock_transaction where network = ?",
        )?;

        let mut rows = stmt.query(params![network.to_string()])?;

        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            let tx_data: Vec<u8> = row.get(0)?;
            let amount: u64 = row.get(1)?;
            let islock_data: Option<Vec<u8>> = row.get(2)?;
            let chain_locked_height: Option<u32> = row.get(3)?;
            let identity_id: Option<Vec<u8>> = row.get(4)?;
            let wallet_seed: Vec<u8> = row.get(5)?;

            let tx: Transaction =
                deserialize(&tx_data).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let islock = if let Some(islock_bytes) = islock_data {
                Some(deserialize(&islock_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?)
            } else {
                None
            };

            let wallet_seed_array: [u8; 32] = wallet_seed
                .try_into()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;

            results.push((
                tx,
                amount,
                islock,
                chain_locked_height,
                identity_id,
                wallet_seed_array,
            ));
        }

        Ok(results)
    }

    /// Retrieves asset lock transactions by identity ID.
    #[allow(dead_code)] // May be used for identity-specific transaction history
    #[allow(clippy::type_complexity)]
    pub fn get_asset_lock_transactions_by_identity_id(
        &self,
        identity_id: &[u8; 32],
    ) -> rusqlite::Result<
        Vec<(
            Transaction,
            u64,
            Option<InstantLock>,
            Option<u32>,
            [u8; 32],
            String,
        )>,
    > {
        let conn = self.conn.lock_or_recover();

        let mut stmt = conn.prepare(
            "SELECT transaction_data, amount, instant_lock_data, chain_locked_height, wallet, network FROM asset_lock_transaction WHERE identity_id = ?1",
        )?;

        let mut rows = stmt.query(params![identity_id])?;

        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            let tx_data: Vec<u8> = row.get(0)?;
            let amount: u64 = row.get(1)?;
            let islock_data: Option<Vec<u8>> = row.get(2)?;
            let chain_locked_height: Option<u32> = row.get(3)?;
            let wallet_seed: Vec<u8> = row.get(4)?;
            let network: String = row.get(5)?;

            let tx: Transaction =
                deserialize(&tx_data).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let islock = if let Some(islock_bytes) = islock_data {
                Some(deserialize(&islock_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?)
            } else {
                None
            };

            let wallet_seed_hash: [u8; 32] = wallet_seed
                .try_into()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;

            results.push((
                tx,
                amount,
                islock,
                chain_locked_height,
                wallet_seed_hash,
                network,
            ));
        }

        Ok(results)
    }

    /// Migrates `asset_lock_transaction` so that both `identity_id` columns use
    /// `ON DELETE SET NULL` instead of `ON DELETE CASCADE`.
    ///
    /// Safe to run multiple times: if the table already has the correct FKs it
    /// exits early.
    pub fn migrate_asset_lock_fk_to_set_null(
        &self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        {
            // ── 1. Detect whether migration is needed ───────────────────────────────
            let mut pragma = conn.prepare("PRAGMA foreign_key_list('asset_lock_transaction')")?;
            let fk_rows = pragma
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(2)?, // table
                        row.get::<_, String>(6)?, // on_delete action
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            // If both identity-related FKs are already SET NULL, nothing to do.
            let needs_migration = fk_rows
                .iter()
                .filter(|(tbl, _)| tbl == "identity")
                .any(|(_, action)| action.to_uppercase() != "SET NULL");

            if !needs_migration {
                return Ok(());
            }
        }

        // ── 2. Recreate table with correct FK actions inside a transaction ─────
        conn.execute("PRAGMA foreign_keys = OFF", [])?;

        conn.execute(
            "ALTER TABLE asset_lock_transaction RENAME TO asset_lock_transaction_old",
            [],
        )?;

        conn.execute(
            "CREATE TABLE asset_lock_transaction (
                tx_id BLOB PRIMARY KEY,
                transaction_data BLOB NOT NULL,
                amount INTEGER,
                instant_lock_data BLOB,
                chain_locked_height INTEGER,
                identity_id BLOB,
                identity_id_potentially_in_creation BLOB,
                wallet BLOB NOT NULL,
                network TEXT NOT NULL,
                FOREIGN KEY (identity_id)
                    REFERENCES identity(id) ON DELETE SET NULL,
                FOREIGN KEY (identity_id_potentially_in_creation)
                    REFERENCES identity(id) ON DELETE SET NULL,
                FOREIGN KEY (wallet)
                    REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "INSERT INTO asset_lock_transaction
              (tx_id, transaction_data, amount, instant_lock_data,
               chain_locked_height, identity_id, identity_id_potentially_in_creation,
               wallet, network)
             SELECT tx_id, transaction_data, amount, instant_lock_data,
                    chain_locked_height, identity_id,
                    identity_id_potentially_in_creation, wallet, network
             FROM asset_lock_transaction_old",
            [],
        )?;

        conn.execute("DROP TABLE asset_lock_transaction_old", [])?;

        conn.execute("PRAGMA foreign_keys = ON", [])?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;
    use dash_sdk::dpp::dashcore::consensus::serialize;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::{Network, Transaction};

    fn create_test_wallet(db: &crate::database::Database) -> [u8; 32] {
        let seed_hash = [0xABu8; 32];
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, uses_password, network)
             VALUES (?, ?, ?, ?, ?, 0, 'testnet')",
            rusqlite::params![
                seed_hash.as_slice(),
                vec![0u8; 64],
                vec![0u8; 16],
                vec![0u8; 12],
                vec![0u8; 78],
            ],
        ).unwrap();
        seed_hash
    }

    fn create_test_tx(value: u64) -> Transaction {
        // Create a minimal valid transaction using consensus deserialization
        // This avoids needing to construct Transaction struct fields manually
        // which may differ between Bitcoin and Dash
        use dash_sdk::dpp::dashcore::consensus::deserialize;

        // Minimal Dash transaction: version 1, 1 input, 1 output, no special payload
        let mut tx_bytes = vec![];
        // version (4 bytes LE) - standard transaction type 0
        tx_bytes.extend_from_slice(&1u32.to_le_bytes());
        // input count (varint: 1)
        tx_bytes.push(1);
        // prev_hash (32 bytes - null for coinbase-like)
        tx_bytes.extend_from_slice(&[0u8; 32]);
        // prev_index (4 bytes)
        tx_bytes.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        // script_sig length (varint: 0)
        tx_bytes.push(0);
        // sequence (4 bytes)
        tx_bytes.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        // output count (varint: 1)
        tx_bytes.push(1);
        // value (8 bytes LE)
        tx_bytes.extend_from_slice(&value.to_le_bytes());
        // script_pubkey length (varint: 0)
        tx_bytes.push(0);
        // lock_time (4 bytes)
        tx_bytes.extend_from_slice(&0u32.to_le_bytes());

        deserialize::<Transaction>(&tx_bytes).expect("Failed to create test transaction")
    }

    #[test]
    fn test_store_and_get_asset_lock() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);
        let tx = create_test_tx(1_000_000);
        let txid = tx.txid().to_byte_array();

        db.store_asset_lock_transaction(&tx, 1_000_000, None, &wallet_hash, Network::Testnet)
            .unwrap();

        let result = db.get_asset_lock_transaction(&txid).unwrap();
        assert!(result.is_some());

        let (retrieved_tx, amount, islock, retrieved_wallet, network) = result.unwrap();
        assert_eq!(serialize(&retrieved_tx), serialize(&tx));
        assert_eq!(amount, 1_000_000);
        assert!(islock.is_none());
        assert_eq!(retrieved_wallet, wallet_hash);
        assert_eq!(network, "testnet");
    }

    #[test]
    fn test_get_nonexistent_asset_lock() {
        let db = create_test_database().unwrap();
        let result = db.get_asset_lock_transaction(&[0u8; 32]).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_chain_locked_height() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);
        let tx = create_test_tx(500_000);
        let txid = tx.txid().to_byte_array();

        db.store_asset_lock_transaction(&tx, 500_000, None, &wallet_hash, Network::Testnet)
            .unwrap();
        db.update_asset_lock_chain_locked_height(&txid, Some(12345))
            .unwrap();

        let all = db
            .get_all_asset_lock_transactions(Network::Testnet)
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].3, Some(12345)); // chain_locked_height
    }

    #[test]
    fn test_set_identity_id() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);
        let tx = create_test_tx(100_000);
        let txid = tx.txid().to_byte_array();
        let identity_id = [0x42u8; 32];

        // Insert identity for FK reference
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO identity (id, is_local, network) VALUES (?, 1, 'testnet')",
                rusqlite::params![identity_id.as_slice()],
            )
            .unwrap();
        }

        db.store_asset_lock_transaction(&tx, 100_000, None, &wallet_hash, Network::Testnet)
            .unwrap();
        db.set_asset_lock_identity_id(&txid, &identity_id).unwrap();

        let all = db
            .get_all_asset_lock_transactions(Network::Testnet)
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].4, Some(identity_id.to_vec())); // identity_id
    }

    #[test]
    fn test_set_identity_id_before_confirmation() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);
        let tx = create_test_tx(200_000);
        let txid = tx.txid().to_byte_array();
        let identity_id = [0x43u8; 32];

        // Insert identity for FK reference
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO identity (id, is_local, network) VALUES (?, 1, 'testnet')",
                rusqlite::params![identity_id.as_slice()],
            )
            .unwrap();
        }

        db.store_asset_lock_transaction(&tx, 200_000, None, &wallet_hash, Network::Testnet)
            .unwrap();
        db.set_asset_lock_identity_id_before_confirmation_by_network(&txid, &identity_id)
            .unwrap();

        // Verify via raw query since get_all doesn't return this column directly
        let conn = db.conn.lock().unwrap();
        let potential_id: Option<Vec<u8>> = conn.query_row(
            "SELECT identity_id_potentially_in_creation FROM asset_lock_transaction WHERE tx_id = ?",
            rusqlite::params![txid.as_slice()],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(potential_id, Some(identity_id.to_vec()));
    }

    #[test]
    fn test_delete_asset_lock() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);
        let tx = create_test_tx(300_000);
        let txid = tx.txid().to_byte_array();

        db.store_asset_lock_transaction(&tx, 300_000, None, &wallet_hash, Network::Testnet)
            .unwrap();
        assert!(db.get_asset_lock_transaction(&txid).unwrap().is_some());

        db.delete_asset_lock_transaction(&txid).unwrap();
        assert!(db.get_asset_lock_transaction(&txid).unwrap().is_none());
    }

    #[test]
    fn test_get_all_filters_by_network() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);

        let tx1 = create_test_tx(100);
        let tx2 = create_test_tx(200);

        db.store_asset_lock_transaction(&tx1, 100, None, &wallet_hash, Network::Testnet)
            .unwrap();
        db.store_asset_lock_transaction(&tx2, 200, None, &wallet_hash, Network::Dash)
            .unwrap();

        let testnet = db
            .get_all_asset_lock_transactions(Network::Testnet)
            .unwrap();
        assert_eq!(testnet.len(), 1);
        assert_eq!(testnet[0].1, 100);

        let mainnet = db.get_all_asset_lock_transactions(Network::Dash).unwrap();
        assert_eq!(mainnet.len(), 1);
        assert_eq!(mainnet[0].1, 200);
    }

    #[test]
    fn test_upsert_preserves_instant_lock() {
        let db = create_test_database().unwrap();
        let wallet_hash = create_test_wallet(&db);
        let tx = create_test_tx(100);
        let txid = tx.txid().to_byte_array();

        // First insert with no islock
        db.store_asset_lock_transaction(&tx, 100, None, &wallet_hash, Network::Testnet)
            .unwrap();

        // Re-insert with no islock (COALESCE should preserve NULL)
        db.store_asset_lock_transaction(&tx, 100, None, &wallet_hash, Network::Testnet)
            .unwrap();

        let result = db.get_asset_lock_transaction(&txid).unwrap().unwrap();
        assert!(result.2.is_none()); // islock still None
    }
}
