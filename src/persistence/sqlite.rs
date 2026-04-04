//! SQLite-backed implementation of [`WalletPersistence`].
//!
//! Persists wallet change deltas into the existing evo-tool database tables
//! (`wallet`, `wallet_transactions`, `utxos`) using a single `rusqlite::Transaction`
//! for atomicity.

use crate::database::Database;
use dash_sdk::dpp::dashcore::consensus::serialize;
use dash_sdk::dpp::dashcore::hashes::Hash;
use platform_wallet::persistence::WalletPersistence;
use platform_wallet::persistence::changeset::{ChainChangeSet, WalletChangeSet};
use std::sync::Arc;

/// Persists [`WalletChangeSet`] deltas into the evo-tool SQLite database.
///
/// Each call to [`persist`](WalletPersistence::persist) acquires the database
/// connection lock, opens a transaction, writes all sub-changesets, and commits
/// atomically.
pub struct SqliteWalletPersister {
    db: Arc<Database>,
    seed_hash: [u8; 32],
    network: String,
}

/// Error type for [`SqliteWalletPersister`].
#[derive(Debug, thiserror::Error)]
pub enum SqlitePersistError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl SqliteWalletPersister {
    /// Create a new persister for the wallet identified by `seed_hash` on `network`.
    pub fn new(db: Arc<Database>, seed_hash: [u8; 32], network: String) -> Self {
        Self {
            db,
            seed_hash,
            network,
        }
    }
}

impl WalletPersistence for SqliteWalletPersister {
    type Error = SqlitePersistError;

    fn initialize(&mut self) -> Result<WalletChangeSet, Self::Error> {
        // TODO: Load persisted state from the database and reconstruct
        // a full WalletChangeSet. For now we return an empty changeset;
        // the wallet starts fresh each launch.
        Ok(WalletChangeSet::default())
    }

    fn persist(&mut self, changeset: &WalletChangeSet) -> Result<(), Self::Error> {
        let conn = self.db.shared_connection();
        let mut guard = conn.lock().unwrap();
        let tx = guard.transaction()?;

        // -- Chain sync state --------------------------------------------------
        // NOTE: block_hash is not persisted yet — the wallet table does not
        // have a dedicated block_hash column. Will be added with a schema migration.
        if let Some(ChainChangeSet {
            height: Some(height),
            ..
        }) = changeset.chain
        {
            tx.execute(
                "UPDATE wallet SET last_terminal_block = ?1
                 WHERE seed_hash = ?2 AND network = ?3",
                rusqlite::params![height as i64, &self.seed_hash[..], &self.network],
            )?;
        }

        // -- Transactions ------------------------------------------------------
        if let Some(ref txs) = changeset.transactions {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO wallet_transactions (
                    seed_hash, txid, network, timestamp, height, block_hash,
                    net_amount, fee, label, is_ours, raw_transaction, status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;

            for (txid, entry) in &txs.transactions {
                let raw = serialize(&entry.transaction);
                let block_hash_bytes: Option<Vec<u8>> =
                    entry.block_hash.map(|bh| bh.as_byte_array().to_vec());

                // Derive a simple status integer compatible with the existing schema:
                //   0 = unconfirmed, 1 = instant-locked, 2 = confirmed/chain-locked.
                let status: i32 = if entry.is_chain_locked {
                    2
                } else if entry.is_instant_locked {
                    1
                } else {
                    0
                };

                stmt.execute(rusqlite::params![
                    &self.seed_hash[..],
                    txid.as_byte_array(),
                    &self.network,
                    entry.timestamp as i64,
                    entry.block_height.map(|h| h as i64),
                    block_hash_bytes,
                    entry.net_amount,
                    entry.fee.map(|f| f as i64),
                    &entry.label,
                    1i32, // is_ours: changeset transactions are always ours
                    &raw,
                    status,
                ])?;
            }
        }

        // -- UTXOs -------------------------------------------------------------
        if let Some(ref utxos) = changeset.utxos {
            // Insert added UTXOs.
            // The utxos table requires an address and script_pubkey which the
            // UtxoChangeSet doesn't carry (it only has outpoint -> value).
            // We store a placeholder address/script for now; the full UTXO
            // details will be populated by SPV sync independently.
            let mut insert_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO utxos (txid, vout, address, value, script_pubkey, network)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (outpoint, value) in &utxos.added {
                insert_stmt.execute(rusqlite::params![
                    outpoint.txid.as_byte_array(),
                    outpoint.vout as i64,
                    "", // address placeholder
                    *value as i64,
                    &[] as &[u8], // script_pubkey placeholder
                    &self.network,
                ])?;
            }

            // Delete spent UTXOs.
            let mut delete_stmt = tx.prepare_cached(
                "DELETE FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3",
            )?;
            for outpoint in &utxos.spent {
                delete_stmt.execute(rusqlite::params![
                    outpoint.txid.as_byte_array(),
                    outpoint.vout as i64,
                    &self.network,
                ])?;
            }
        }

        // -- Accounts ----------------------------------------------------------
        // TODO: persist AccountChangeSet (last_revealed indices) once we add
        // a wallet_accounts table.

        // -- Identities --------------------------------------------------------
        // TODO: persist IdentityChangeSet once we wire identity persistence
        // through the changeset path.

        // -- Contacts ----------------------------------------------------------
        // TODO: persist ContactChangeSet once DashPay contacts use changesets.

        // -- Platform addresses ------------------------------------------------
        // TODO: persist PlatformAddressChangeSet once platform_address_balances
        // table is wired through changesets.

        // -- Asset locks -------------------------------------------------------
        // TODO: persist AssetLockChangeSet once asset_lock_transaction table
        // is wired through changesets.

        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;
    use platform_wallet::persistence::Merge;
    use platform_wallet::persistence::changeset::*;

    fn make_persister(db: Arc<Database>) -> SqliteWalletPersister {
        SqliteWalletPersister::new(db, [0u8; 32], "testnet".to_string())
    }

    /// A persister can be created and initialized without error.
    #[test]
    fn test_initialize_returns_empty_changeset() {
        let db = Arc::new(create_test_database().expect("create test db"));
        let mut persister = make_persister(db);

        let cs = persister.initialize().expect("initialize");
        assert!(cs.is_empty());
    }

    /// Persisting an empty changeset succeeds (no-op transaction).
    #[test]
    fn test_persist_empty_changeset() {
        let db = Arc::new(create_test_database().expect("create test db"));
        let mut persister = make_persister(db);

        let cs = WalletChangeSet::default();
        persister.persist(&cs).expect("persist empty changeset");
    }

    /// Persisting a chain changeset updates the wallet row.
    #[test]
    fn test_persist_chain_changeset() {
        let db = Arc::new(create_test_database().expect("create test db"));

        // Insert a wallet row so the UPDATE has something to hit.
        db.execute(
            "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce,
             master_ecdsa_bip44_account_0_epk, uses_password, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                &[0u8; 32][..],
                &[0u8; 1][..],
                &[0u8; 1][..],
                &[0u8; 1][..],
                &[0u8; 1][..],
                0i32,
                "testnet",
            ],
        )
        .expect("insert wallet row");

        let mut persister = make_persister(db.clone());

        let cs = WalletChangeSet {
            chain: Some(ChainChangeSet {
                height: Some(12345),
                block_hash: None,
            }),
            ..Default::default()
        };
        persister.persist(&cs).expect("persist chain changeset");

        // Verify the height was written.
        let conn = db.shared_connection();
        let guard = conn.lock().unwrap();
        let stored_height: i64 = guard
            .query_row(
                "SELECT last_terminal_block FROM wallet
                 WHERE seed_hash = ?1 AND network = ?2",
                rusqlite::params![&[0u8; 32][..], "testnet"],
                |row| row.get(0),
            )
            .expect("query height");
        assert_eq!(stored_height, 12345);
    }

    /// Persisting a UTXO changeset adds and removes rows.
    #[test]
    fn test_persist_utxo_changeset() {
        use dash_sdk::dpp::dashcore::{OutPoint, Txid};
        use std::collections::{BTreeMap, BTreeSet};

        let db = Arc::new(create_test_database().expect("create test db"));
        let mut persister = make_persister(db.clone());

        let txid = Txid::from_slice(&[1u8; 32]).unwrap();
        let outpoint = OutPoint { txid, vout: 0 };

        // Add a UTXO.
        let mut added = BTreeMap::new();
        added.insert(outpoint, 50_000u64);
        let cs = WalletChangeSet {
            utxos: Some(UtxoChangeSet {
                added,
                spent: BTreeSet::new(),
            }),
            ..Default::default()
        };
        persister.persist(&cs).expect("persist add utxo");

        // Verify it was inserted.
        let conn = db.shared_connection();
        {
            let guard = conn.lock().unwrap();
            let count: i64 = guard
                .query_row(
                    "SELECT COUNT(*) FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3",
                    rusqlite::params![txid.as_byte_array(), 0i64, "testnet"],
                    |row| row.get(0),
                )
                .expect("count utxos");
            assert_eq!(count, 1);
        }

        // Now spend it.
        let mut spent = BTreeSet::new();
        spent.insert(outpoint);
        let cs2 = WalletChangeSet {
            utxos: Some(UtxoChangeSet {
                added: BTreeMap::new(),
                spent,
            }),
            ..Default::default()
        };
        persister.persist(&cs2).expect("persist spend utxo");

        // Verify it was removed.
        {
            let guard = conn.lock().unwrap();
            let count: i64 = guard
                .query_row(
                    "SELECT COUNT(*) FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3",
                    rusqlite::params![txid.as_byte_array(), 0i64, "testnet"],
                    |row| row.get(0),
                )
                .expect("count utxos after spend");
            assert_eq!(count, 0);
        }
    }
}
