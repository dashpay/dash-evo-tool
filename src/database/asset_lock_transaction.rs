use crate::database::Database;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    consensus::{deserialize, serialize},
    InstantLock, Network, Transaction,
};
use rusqlite::params;

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

        let islock_bytes = if let Some(islock) = islock {
            Some(serialize(islock))
        } else {
            None
        };

        let conn = self.conn.lock().unwrap();

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
    pub fn get_asset_lock_transaction(
        &self,
        txid: &[u8; 32],
    ) -> rusqlite::Result<Option<(Transaction, u64, Option<InstantLock>, [u8; 32], String)>> {
        let conn = self.conn.lock().unwrap();

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
    pub fn update_asset_lock_chain_locked_height(
        &self,
        txid: &[u8; 32],
        chain_locked_height: Option<u32>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

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
        let conn = self.conn.lock().unwrap();

        let rows_updated = conn.execute(
            "UPDATE asset_lock_transaction
     SET identity_id = ?1, identity_id_potentially_in_creation = NULL
     WHERE tx_id = ?2",
            params![identity_id, tx_id],
        )?;
        if rows_updated == 0 {
            eprintln!(
                "No rows updated. Check if tx_id {} exists and identity_id {} is correct.",
                hex::encode(tx_id),
                hex::encode(identity_id)
            );
        }

        Ok(())
    }

    /// Sets the identity ID for an asset lock transaction.
    pub fn set_asset_lock_identity_id_before_confirmation_by_network(
        &self,
        txid: &[u8; 32],
        identity_id: &[u8; 32],
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE asset_lock_transaction SET identity_id_potentially_in_creation = ?1 WHERE tx_id = ?2",
            params![identity_id, txid],
        )?;

        Ok(())
    }

    /// Deletes an asset lock transaction by its transaction ID.
    pub fn delete_asset_lock_transaction(&self, txid: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM asset_lock_transaction WHERE tx_id = ?1",
            params![txid],
        )?;

        Ok(())
    }

    /// Retrieves all asset lock transactions.
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
        let conn = self.conn.lock().unwrap();

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
        let conn = self.conn.lock().unwrap();

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
}
