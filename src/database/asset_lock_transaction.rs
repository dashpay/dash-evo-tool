use crate::database::Database;
use dash_sdk::dpp::dashcore::{
    consensus::{deserialize, serialize},
    InstantLock, Transaction,
};
use rusqlite::params;

impl Database {
    /// Stores an asset lock transaction and optional InstantLock into the database.
    pub fn store_asset_lock_transaction(
        &self,
        tx: &Transaction,
        amount: u64, // Include amount as a parameter
        islock: Option<&InstantLock>,
        wallet_seed: &[u8; 64], // Include wallet_seed as a parameter
    ) -> rusqlite::Result<()> {
        let tx_bytes = serialize(tx);
        let txid = tx.txid().to_string();

        let islock_bytes = if let Some(islock) = islock {
            Some(serialize(islock))
        } else {
            None
        };

        let conn = self.conn.lock().unwrap();

        let sql = "
        INSERT INTO asset_lock_transaction (tx_id, transaction_data, amount, instant_lock_data, wallet)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(tx_id) DO UPDATE SET
            transaction_data = excluded.transaction_data,
            amount = excluded.amount,
            instant_lock_data = COALESCE(excluded.instant_lock_data, asset_lock_transaction.instant_lock_data);
        ";

        conn.execute(
            sql,
            params![&txid, &tx_bytes, amount, &islock_bytes, wallet_seed],
        )?;

        Ok(())
    }

    /// Retrieves an asset lock transaction by its transaction ID.
    pub fn get_asset_lock_transaction(
        &self,
        txid: &str,
    ) -> rusqlite::Result<Option<(Transaction, u64, Option<InstantLock>, [u8; 64])>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT transaction_data, amount, instant_lock_data, wallet FROM asset_lock_transaction WHERE tx_id = ?1",
        )?;

        let mut rows = stmt.query(params![txid])?;

        if let Some(row) = rows.next()? {
            let tx_data: Vec<u8> = row.get(0)?;
            let amount: u64 = row.get(1)?;
            let islock_data: Option<Vec<u8>> = row.get(2)?;
            let wallet_seed: Vec<u8> = row.get(3)?;

            let tx: Transaction =
                deserialize(&tx_data).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let islock = if let Some(islock_bytes) = islock_data {
                Some(deserialize(&islock_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?)
            } else {
                None
            };

            let wallet_seed_array: [u8; 64] = wallet_seed
                .try_into()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;

            Ok(Some((tx, amount, islock, wallet_seed_array)))
        } else {
            Ok(None)
        }
    }

    /// Updates the chain locked height for an asset lock transaction.
    pub fn update_asset_lock_chain_locked_height(
        &self,
        txid: &str,
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
        txid: &str,
        identity_id: Option<&[u8]>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE asset_lock_transaction SET identity_id = ?1 WHERE tx_id = ?2",
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
    ) -> rusqlite::Result<
        Vec<(
            Transaction,
            u64,
            Option<InstantLock>,
            Option<u32>,
            Option<Vec<u8>>,
            [u8; 64],
        )>,
    > {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT transaction_data, amount, instant_lock_data, chain_locked_height, identity_id, wallet FROM asset_lock_transaction",
        )?;

        let mut rows = stmt.query(params![])?;

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

            let wallet_seed_array: [u8; 64] = wallet_seed
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
        identity_id: &[u8],
    ) -> rusqlite::Result<Vec<(Transaction, u64, Option<InstantLock>, Option<u32>, [u8; 64])>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT transaction_data, amount, instant_lock_data, chain_locked_height, wallet FROM asset_lock_transaction WHERE identity_id = ?1",
        )?;

        let mut rows = stmt.query(params![identity_id])?;

        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            let tx_data: Vec<u8> = row.get(0)?;
            let amount: u64 = row.get(1)?;
            let islock_data: Option<Vec<u8>> = row.get(2)?;
            let chain_locked_height: Option<u32> = row.get(3)?;
            let wallet_seed: Vec<u8> = row.get(4)?;

            let tx: Transaction =
                deserialize(&tx_data).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let islock = if let Some(islock_bytes) = islock_data {
                Some(deserialize(&islock_bytes).map_err(|_| rusqlite::Error::InvalidQuery)?)
            } else {
                None
            };

            let wallet_seed_array: [u8; 64] = wallet_seed
                .try_into()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;

            results.push((tx, amount, islock, chain_locked_height, wallet_seed_array));
        }

        Ok(results)
    }
}
