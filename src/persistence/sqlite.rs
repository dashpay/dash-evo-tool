//! SQLite-backed implementation of [`WalletPersistence`].
//!
//! Persists wallet change deltas into the existing evo-tool database tables
//! (`wallet`, `wallet_transactions`, `utxos`) using a single `rusqlite::Transaction`
//! for atomicity.

use crate::database::Database;
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{BlockHash, OutPoint, Transaction, Txid};
use platform_wallet::persistence::Merge;
use platform_wallet::persistence::WalletPersistence;
use platform_wallet::persistence::changeset::{
    ChainChangeSet, PlatformWalletChangeSet, TransactionChangeSet, TransactionEntry, UtxoChangeSet,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Persists [`PlatformWalletChangeSet`] deltas into the evo-tool SQLite database.
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

impl SqliteWalletPersister {
    /// Persist platform-level transaction changeset entries into `wallet_transactions`.
    fn persist_transactions(
        tx: &rusqlite::Transaction,
        seed_hash: &[u8; 32],
        network: &str,
        txs: &TransactionChangeSet,
    ) -> Result<(), rusqlite::Error> {
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
                &seed_hash[..],
                txid.as_byte_array(),
                network,
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
        Ok(())
    }

    /// Persist platform-level UTXO changeset entries into `utxos`.
    fn persist_utxos(
        tx: &rusqlite::Transaction,
        network: &str,
        utxos: &UtxoChangeSet,
    ) -> Result<(), rusqlite::Error> {
        // Insert added UTXOs.
        // The platform-level UtxoChangeSet only carries outpoint -> value (no
        // address/script). We store a placeholder; full details come from SPV.
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
                network,
            ])?;
        }

        // Delete spent UTXOs.
        let mut delete_stmt =
            tx.prepare_cached("DELETE FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3")?;
        for outpoint in &utxos.spent {
            delete_stmt.execute(rusqlite::params![
                outpoint.txid.as_byte_array(),
                outpoint.vout as i64,
                network,
            ])?;
        }
        Ok(())
    }
}

impl WalletPersistence for SqliteWalletPersister {
    type Error = SqlitePersistError;

    fn initialize(&mut self) -> Result<PlatformWalletChangeSet, Self::Error> {
        let conn = self.db.shared_connection();
        let guard = conn.lock().unwrap();

        // -- Load chain height ---------------------------------------------------
        let chain = {
            let maybe_height: Option<i64> = guard
                .query_row(
                    "SELECT last_terminal_block FROM wallet
                     WHERE seed_hash = ?1 AND network = ?2",
                    rusqlite::params![&self.seed_hash[..], &self.network],
                    |row| row.get(0),
                )
                .ok();

            maybe_height.filter(|&h| h > 0).map(|h| ChainChangeSet {
                height: Some(h as u32),
                block_hash: None,
            })
        };

        // -- Load transactions ---------------------------------------------------
        let transactions = {
            let mut stmt = guard.prepare(
                "SELECT txid, raw_transaction, height, block_hash,
                        timestamp, net_amount, fee, label, status
                 FROM wallet_transactions
                 WHERE seed_hash = ?1 AND network = ?2",
            )?;

            let mut txs = BTreeMap::new();
            let mut rows = stmt.query(rusqlite::params![&self.seed_hash[..], &self.network])?;
            while let Some(row) = rows.next()? {
                let txid_bytes: Vec<u8> = row.get(0)?;
                let raw: Vec<u8> = row.get(1)?;
                let height: Option<i64> = row.get(2)?;
                let block_hash_bytes: Option<Vec<u8>> = row.get(3)?;
                let timestamp: i64 = row.get(4)?;
                let net_amount: i64 = row.get(5)?;
                let fee: Option<i64> = row.get(6)?;
                let label: Option<String> = row.get(7)?;
                let status: i32 = row.get(8)?;

                let Ok(txid) = Txid::from_slice(&txid_bytes) else {
                    continue;
                };
                let Ok(transaction) = deserialize::<Transaction>(&raw) else {
                    continue;
                };
                let block_hash = block_hash_bytes
                    .as_deref()
                    .and_then(|b| BlockHash::from_slice(b).ok());

                txs.insert(
                    txid,
                    TransactionEntry {
                        transaction,
                        block_height: height.map(|h| h as u32),
                        block_hash,
                        timestamp: timestamp as u64,
                        net_amount,
                        fee: fee.map(|f| f as u64),
                        label,
                        is_instant_locked: status == 1,
                        is_chain_locked: status == 2,
                    },
                );
            }

            if txs.is_empty() {
                None
            } else {
                Some(TransactionChangeSet { transactions: txs })
            }
        };

        // -- Load UTXOs ----------------------------------------------------------
        let utxos = {
            let mut stmt =
                guard.prepare("SELECT txid, vout, value FROM utxos WHERE network = ?1")?;

            let mut added = BTreeMap::new();
            let mut rows = stmt.query(rusqlite::params![&self.network])?;
            while let Some(row) = rows.next()? {
                let txid_bytes: Vec<u8> = row.get(0)?;
                let vout: i64 = row.get(1)?;
                let value: i64 = row.get(2)?;

                let Ok(txid) = Txid::from_slice(&txid_bytes) else {
                    continue;
                };
                let outpoint = OutPoint {
                    txid,
                    vout: vout as u32,
                };
                added.insert(outpoint, value as u64);
            }

            if added.is_empty() {
                None
            } else {
                Some(UtxoChangeSet {
                    added,
                    spent: BTreeSet::new(),
                })
            }
        };

        let cs = PlatformWalletChangeSet {
            chain,
            transactions,
            utxos,
            ..Default::default()
        };

        if cs.is_empty() {
            Ok(PlatformWalletChangeSet::default())
        } else {
            Ok(cs)
        }
    }

    fn persist(&mut self, changeset: &PlatformWalletChangeSet) -> Result<(), Self::Error> {
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
            Self::persist_transactions(&tx, &self.seed_hash, &self.network, txs)?;
        }

        // -- UTXOs -------------------------------------------------------------
        if let Some(ref utxos) = changeset.utxos {
            Self::persist_utxos(&tx, &self.network, utxos)?;
        }

        // -- Key-wallet sub-changesets -----------------------------------------
        if let Some(ref wallet_cs) = changeset.wallet {
            // wallet.chain → update wallet height
            if let Some(ref chain) = wallet_cs.chain
                && let Some(height) = chain.height
            {
                tx.execute(
                    "UPDATE wallet SET last_terminal_block = ?1
                     WHERE seed_hash = ?2 AND network = ?3",
                    rusqlite::params![height as i64, &self.seed_hash[..], &self.network],
                )?;
            }

            // wallet.transactions → INSERT OR REPLACE transaction records
            if let Some(ref kw_txs) = wallet_cs.transactions {
                let mut stmt = tx.prepare_cached(
                    "INSERT OR REPLACE INTO wallet_transactions (
                        seed_hash, txid, network, timestamp, height, block_hash,
                        net_amount, fee, label, is_ours, raw_transaction, status
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                )?;

                for (txid, entry) in &kw_txs.records {
                    let raw = serialize(&entry.transaction);
                    let block_hash_bytes: Option<Vec<u8>> =
                        entry.block_hash.map(|bh| bh.as_byte_array().to_vec());

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
                        1i32,
                        &raw,
                        status,
                    ])?;
                }
            }

            // wallet.utxos → INSERT added, DELETE spent
            if let Some(ref kw_utxos) = wallet_cs.utxos {
                let mut insert_stmt = tx.prepare_cached(
                    "INSERT OR IGNORE INTO utxos (txid, vout, address, value, script_pubkey, network)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                for (outpoint, entry) in &kw_utxos.added {
                    insert_stmt.execute(rusqlite::params![
                        outpoint.txid.as_byte_array(),
                        outpoint.vout as i64,
                        entry.address.to_string(),
                        entry.value as i64,
                        entry.script_pubkey.as_bytes(),
                        &self.network,
                    ])?;
                }

                let mut delete_stmt = tx.prepare_cached(
                    "DELETE FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3",
                )?;
                for outpoint in &kw_utxos.spent {
                    delete_stmt.execute(rusqlite::params![
                        outpoint.txid.as_byte_array(),
                        outpoint.vout as i64,
                        &self.network,
                    ])?;
                }
            }

            // wallet.accounts → TODO: address pool state not yet persisted

            // wallet.balance → UPDATE wallet balance fields
            if let Some(ref bal) = wallet_cs.balance {
                // Balance changeset carries deltas; apply them with SQL arithmetic.
                tx.execute(
                    "UPDATE wallet SET
                         confirmed_balance = MAX(0, confirmed_balance + ?1),
                         unconfirmed_balance = MAX(0, unconfirmed_balance + ?2),
                         total_balance = MAX(0, total_balance + ?1 + ?2)
                     WHERE seed_hash = ?3 AND network = ?4",
                    rusqlite::params![
                        bal.spendable_delta,
                        bal.unconfirmed_delta,
                        &self.seed_hash[..],
                        &self.network,
                    ],
                )?;
            }
        }

        // -- Accounts ----------------------------------------------------------
        // TODO: persist AccountChangeSet (last_revealed indices) once we add
        // a wallet_accounts table.

        // -- Contacts ----------------------------------------------------------
        if let Some(ref contacts) = changeset.contacts {
            // Sent contact requests
            for (from_id, to_id) in contacts.sent_requests.keys() {
                tx.execute(
                    "INSERT OR IGNORE INTO dashpay_contact_requests
                        (from_identity_id, to_identity_id, network, request_type, status)
                     VALUES (?1, ?2, ?3, 'sent', 'pending')",
                    rusqlite::params![from_id.as_bytes(), to_id.as_bytes(), &self.network,],
                )?;
            }

            // Incoming contact requests
            for (from_id, to_id) in contacts.incoming_requests.keys() {
                tx.execute(
                    "INSERT OR IGNORE INTO dashpay_contact_requests
                        (from_identity_id, to_identity_id, network, request_type, status)
                     VALUES (?1, ?2, ?3, 'received', 'pending')",
                    rusqlite::params![from_id.as_bytes(), to_id.as_bytes(), &self.network,],
                )?;
            }

            // Established contacts
            for (owner_id, contact_id) in &contacts.established {
                tx.execute(
                    "INSERT OR IGNORE INTO dashpay_contacts
                        (owner_identity_id, contact_identity_id, network, contact_status)
                     VALUES (?1, ?2, ?3, 'accepted')",
                    rusqlite::params![owner_id.as_bytes(), contact_id.as_bytes(), &self.network,],
                )?;
            }
        }

        // -- Identities --------------------------------------------------------
        // TODO: persist IdentityChangeSet once we wire identity persistence
        // through the changeset path.

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

        let cs = PlatformWalletChangeSet::default();
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

        let cs = PlatformWalletChangeSet {
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
        let cs = PlatformWalletChangeSet {
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
        let cs2 = PlatformWalletChangeSet {
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

    /// Persist a chain changeset then initialize() to verify round-trip.
    #[test]
    fn test_initialize_loads_persisted_state() {
        use dash_sdk::dpp::dashcore::{OutPoint, Txid};
        use std::collections::{BTreeMap, BTreeSet};

        let db = Arc::new(create_test_database().expect("create test db"));

        // Insert a wallet row.
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

        // Persist chain height and a UTXO.
        let txid = Txid::from_slice(&[2u8; 32]).unwrap();
        let outpoint = OutPoint { txid, vout: 1 };
        let mut added = BTreeMap::new();
        added.insert(outpoint, 75_000u64);

        let cs = PlatformWalletChangeSet {
            chain: Some(ChainChangeSet {
                height: Some(99999),
                block_hash: None,
            }),
            utxos: Some(UtxoChangeSet {
                added,
                spent: BTreeSet::new(),
            }),
            ..Default::default()
        };
        persister.persist(&cs).expect("persist changeset");

        // Now initialize and verify the state was loaded.
        let loaded = persister.initialize().expect("initialize");
        assert!(!loaded.is_empty());

        // Chain height should match.
        let chain = loaded.chain.expect("chain should be loaded");
        assert_eq!(chain.height, Some(99999));

        // UTXOs should contain the one we added.
        let utxos = loaded.utxos.expect("utxos should be loaded");
        assert_eq!(utxos.added.len(), 1);
        assert_eq!(utxos.added.get(&outpoint), Some(&75_000u64));
        assert!(utxos.spent.is_empty());
    }
}
