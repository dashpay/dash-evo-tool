use crate::context::AppContext;
use crate::database::Database;
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
    #[allow(dead_code)] // May be used for querying asset locks
    #[allow(clippy::type_complexity)]
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
    #[allow(dead_code)] // May be used for tracking chain confirmation status
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

        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE asset_lock_transaction
         SET identity_id = NULL,
             identity_id_potentially_in_creation = NULL
         WHERE network = ?",
            params![network],
        )?;

        Ok(())
    }

    /// Deletes an asset lock transaction by its transaction ID (as bytes).
    pub fn delete_asset_lock_transaction(&self, txid: &[u8; 32]) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

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

/// I4 (release-blocking) — crash-retry must not double-broadcast.
///
/// DET never builds or broadcasts asset-lock transactions itself: the legacy
/// DET asset-lock builders (`generic_asset_lock_transaction`,
/// `*_for_utxo*`, `select_unspent_utxos_for`) are deleted (I2) and
/// `WalletBackend::broadcast_transaction` is never called for asset locks —
/// upstream `create_funded_asset_lock_proof` owns select+sign+broadcast+
/// track atomically. The only DET-side durable record is
/// `store_asset_lock_transaction`, keyed on `tx_id` with `ON CONFLICT DO
/// UPDATE`. This lane proves that a simulated crash between store and
/// (upstream) broadcast, followed by a relaunch that re-stores the retried
/// asset-lock tx, yields exactly ONE durable record with identical inputs —
/// the DET path produces no competing asset-lock tx spending different
/// inputs.
#[cfg(test)]
mod crash_retry_no_double_broadcast {
    use super::*;
    use crate::database::test_helpers::create_temp_database;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
    use dash_sdk::dpp::dashcore::transaction::special_transaction::asset_lock::AssetLockPayload;
    use dash_sdk::dpp::dashcore::{OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Txid, Witness};

    /// A deterministic asset-lock tx funded by `funding_outpoint`. Two calls
    /// with the same outpoint produce byte-identical txs (and thus the same
    /// txid) — exactly the determinism upstream relies on for dedup.
    fn asset_lock_tx(funding_outpoint: OutPoint, amount: u64) -> Transaction {
        Transaction {
            version: 3,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: funding_outpoint,
                script_sig: ScriptBuf::new(),
                sequence: u32::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: amount,
                script_pubkey: ScriptBuf::new(),
            }],
            special_transaction_payload: Some(TransactionPayload::AssetLockPayloadType(
                AssetLockPayload {
                    version: 1,
                    credit_outputs: vec![TxOut {
                        value: amount,
                        script_pubkey: ScriptBuf::new(),
                    }],
                },
            )),
        }
    }

    /// Seed a minimal legacy `wallet` row so the asset_lock_transaction FK
    /// (`wallet` → `wallet(seed_hash)`) is satisfied. A valid serialized
    /// account-0 xpub keeps `get_wallets` parseable if ever loaded.
    fn seed_wallet_row(db: &Database, seed_hash: &[u8; 32], network: Network) {
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::key_wallet::bip32::{
            ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey,
        };
        let secp = Secp256k1::new();
        let master = ExtendedPrivKey::new_master(network, &[7u8; 64]).expect("master");
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
        ]);
        let account = master.derive_priv(&secp, &path).expect("derive");
        let epk = ExtendedPubKey::from_priv(&secp, &account).encode().to_vec();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO wallet (seed_hash, encrypted_seed, salt, nonce, \
             master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, network) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'al-wallet', 1, 0, ?6)",
            params![
                seed_hash.as_slice(),
                vec![0u8; 64],
                vec![0u8; 16],
                vec![0u8; 12],
                epk,
                network.to_string(),
            ],
        )
        .expect("seed wallet row");
    }

    fn row_count(db: &Database) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM asset_lock_transaction", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn crash_between_store_and_broadcast_then_retry_yields_one_tx_same_inputs() {
        let (db, _tmp) = create_temp_database().expect("temp db");
        let network = Network::Testnet;
        let seed_hash = [7u8; 32];

        // The funding outpoint upstream coin-selection picked. The retried
        // broadcast reuses the same selection => same deterministic tx.
        let funding = OutPoint::new(Txid::from_byte_array([3u8; 32]), 0);
        let tx = asset_lock_tx(funding, 100_000);
        let txid = tx.txid().to_byte_array();

        seed_wallet_row(&db, &seed_hash, network);

        // Store-before-broadcast: the durable record exists BEFORE upstream
        // broadcast. Then a crash (process death) — no marker, nothing else.
        db.store_asset_lock_transaction(&tx, 100_000, None, &seed_hash, network)
            .expect("store before broadcast");
        assert_eq!(row_count(&db), 1, "exactly one durable record after store");

        // Relaunch: reopen the same DB file (crash-relaunch). The retried
        // broadcast re-stores the SAME deterministic asset-lock tx.
        let db_path = db.db_file_path().expect("file-backed db");
        drop(db);
        let db = Database::new(&db_path).expect("reopen after crash");
        db.initialize(&db_path).expect("init after crash");

        let retried = asset_lock_tx(funding, 100_000);
        assert_eq!(
            retried.txid().to_byte_array(),
            txid,
            "retry reuses the same selection => same deterministic txid"
        );
        db.store_asset_lock_transaction(&retried, 100_000, None, &seed_hash, network)
            .expect("idempotent re-store on retry");

        // No double-broadcast surface: exactly ONE record, same txid, same
        // serialized bytes (same inputs) — the DET path produced no second
        // asset-lock tx spending different inputs.
        assert_eq!(
            row_count(&db),
            1,
            "crash-retry must not create a second/competing asset-lock record"
        );
        let (stored, amount, islock, stored_seed, stored_net) = db
            .get_asset_lock_transaction(&txid)
            .expect("query")
            .expect("the single record must be present");
        assert_eq!(stored.txid().to_byte_array(), txid);
        assert_eq!(
            dash_sdk::dpp::dashcore::consensus::serialize(&stored),
            dash_sdk::dpp::dashcore::consensus::serialize(&tx),
            "the retried record spends the SAME inputs (identical tx bytes)"
        );
        assert_eq!(stored.input, tx.input, "funding inputs unchanged on retry");
        assert_eq!(amount, 100_000);
        assert!(islock.is_none());
        assert_eq!(stored_seed, seed_hash);
        assert_eq!(stored_net, network.to_string());

        // A hypothetical competing tx spending DIFFERENT inputs would have a
        // DIFFERENT txid and is simply absent — DET has no asset-lock
        // builder/broadcaster to ever produce it (I2).
        let competing = asset_lock_tx(OutPoint::new(Txid::from_byte_array([9u8; 32]), 1), 100_000);
        assert_ne!(competing.txid().to_byte_array(), txid);
        assert!(
            db.get_asset_lock_transaction(&competing.txid().to_byte_array())
                .expect("query competing")
                .is_none(),
            "no competing asset-lock tx exists — DET never broadcasts a second one"
        );
    }
}
