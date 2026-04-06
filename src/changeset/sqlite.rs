//! SQLite-backed implementation of [`PlatformWalletPersistence`].
//!
//! Persists wallet change deltas into the existing evo-tool database tables
//! (`wallet`, `wallet_transactions`, `utxos`) using a single `rusqlite::Transaction`
//! for atomicity.

use crate::database::Database;
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{BlockHash, OutPoint, Transaction, Txid};
use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;
use dash_sdk::dpp::key_wallet::dip9::DerivationPathReference;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::asset_lock_builder::AssetLockFundingType;
use dash_sdk::dpp::prelude::{AssetLockProof, Identifier};
use dash_sdk::dpp::serialization::{PlatformDeserializable, PlatformSerializable};
use platform_wallet::AssetLockStatus;
use platform_wallet::changeset::Merge;
use platform_wallet::changeset::PlatformWalletPersistence;
use platform_wallet::changeset::changeset::{
    AccountChangeSet, AssetLockChangeSet, AssetLockEntry, ChainChangeSet, IdentityChangeSet,
    IdentityEntry, PlatformAddressChangeSet, PlatformAddressEntry, PlatformWalletChangeSet,
    TransactionChangeSet, TransactionEntry, UtxoChangeSet,
};
use platform_wallet::wallet::WalletId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// Controls when queued changesets are written to storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushStrategy {
    /// Flush to storage after every [`queue`](PlatformWalletPersistence::queue) call.
    Immediate,
    /// Caller must explicitly call [`flush`](PlatformWalletPersistence::flush).
    Manual,
}

/// Persists [`PlatformWalletChangeSet`] deltas into the evo-tool SQLite database.
///
/// Changesets are buffered via [`queue`](PlatformWalletPersistence::queue) and
/// written atomically on [`flush`](PlatformWalletPersistence::flush).
///
/// When [`FlushStrategy::Immediate`] is set (the default), each `queue()` call
/// automatically triggers a `flush()`, so callers don't need to call
/// `persist_platform_wallet` / `flush_persist` separately.
///
/// A single persister instance is shared across all wallets managed by a
/// [`PlatformWalletManager`]. Each wallet is identified by its `WalletId`
/// (which equals the evo-tool `seed_hash`).
pub struct SqliteWalletPersister {
    db: Arc<Database>,
    network: String,
    /// Per-wallet accumulated changesets waiting to be flushed.
    staged: Mutex<BTreeMap<WalletId, PlatformWalletChangeSet>>,
    /// When to write queued changesets to storage.
    strategy: FlushStrategy,
}

/// Error type for [`SqliteWalletPersister`].
#[derive(Debug, thiserror::Error)]
pub enum SqlitePersistError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl SqliteWalletPersister {
    /// Create a new persister for the given `network`.
    ///
    /// Uses [`FlushStrategy::Immediate`] by default so that every `queue()` call
    /// is automatically persisted. The persister is wallet-id-aware: the
    /// `wallet_id` is passed per-call to [`queue`], [`flush`] and
    /// [`initialize`].
    pub fn new(db: Arc<Database>, network: String) -> Self {
        Self {
            db,
            network,
            staged: Mutex::new(BTreeMap::new()),
            strategy: FlushStrategy::Immediate,
        }
    }

    /// Set the flush strategy for this persister.
    #[allow(dead_code)]
    pub fn with_strategy(mut self, strategy: FlushStrategy) -> Self {
        self.strategy = strategy;
        self
    }
}

/// Convert a `u32` discriminant back to [`DerivationPathReference`].
///
/// The key-wallet crate does not expose a `TryFrom<u32>` impl for its enum,
/// so we maintain a local mapping that mirrors the discriminant values.
fn derivation_path_reference_from_u32(v: u32) -> Option<DerivationPathReference> {
    match v {
        0 => Some(DerivationPathReference::Unknown),
        1 => Some(DerivationPathReference::BIP32),
        2 => Some(DerivationPathReference::BIP44),
        3 => Some(DerivationPathReference::BlockchainIdentities),
        4 => Some(DerivationPathReference::ProviderFunds),
        5 => Some(DerivationPathReference::ProviderVotingKeys),
        6 => Some(DerivationPathReference::ProviderOperatorKeys),
        7 => Some(DerivationPathReference::ProviderOwnerKeys),
        8 => Some(DerivationPathReference::ContactBasedFunds),
        9 => Some(DerivationPathReference::ContactBasedFundsRoot),
        10 => Some(DerivationPathReference::ContactBasedFundsExternal),
        11 => Some(DerivationPathReference::BlockchainIdentityCreditRegistrationFunding),
        12 => Some(DerivationPathReference::BlockchainIdentityCreditTopupFunding),
        13 => Some(DerivationPathReference::BlockchainIdentityCreditInvitationFunding),
        14 => Some(DerivationPathReference::ProviderPlatformNodeKeys),
        15 => Some(DerivationPathReference::CoinJoin),
        16 => Some(DerivationPathReference::PlatformPayment),
        17 => Some(DerivationPathReference::BlockchainAssetLockAddressTopupFunding),
        18 => Some(DerivationPathReference::BlockchainAssetLockShieldedAddressTopupFunding),
        255 => Some(DerivationPathReference::Root),
        _ => None,
    }
}

/// Convert an [`AssetLockFundingType`] to an integer discriminant for SQLite storage.
fn funding_type_to_i64(ft: AssetLockFundingType) -> i64 {
    match ft {
        AssetLockFundingType::IdentityRegistration => 0,
        AssetLockFundingType::IdentityTopUp => 1,
        AssetLockFundingType::IdentityTopUpNotBound => 2,
        AssetLockFundingType::IdentityInvitation => 3,
        AssetLockFundingType::AssetLockAddressTopUp => 4,
        AssetLockFundingType::AssetLockShieldedAddressTopUp => 5,
    }
}

/// Convert an integer discriminant back to [`AssetLockFundingType`].
fn funding_type_from_i64(v: i64) -> Option<AssetLockFundingType> {
    match v {
        0 => Some(AssetLockFundingType::IdentityRegistration),
        1 => Some(AssetLockFundingType::IdentityTopUp),
        2 => Some(AssetLockFundingType::IdentityTopUpNotBound),
        3 => Some(AssetLockFundingType::IdentityInvitation),
        4 => Some(AssetLockFundingType::AssetLockAddressTopUp),
        5 => Some(AssetLockFundingType::AssetLockShieldedAddressTopUp),
        _ => None,
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

    /// Ensure the `wallet_account_state` table exists.
    fn ensure_account_state_table(tx: &rusqlite::Transaction) -> Result<(), rusqlite::Error> {
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS wallet_account_state (
                seed_hash BLOB NOT NULL,
                account_index INTEGER NOT NULL,
                path_reference INTEGER NOT NULL,
                last_revealed INTEGER NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY (seed_hash, account_index, path_reference, network)
            )",
        )?;
        Ok(())
    }

    /// Persist platform-wallet account changeset (last revealed indices) into `wallet_account_state`.
    fn persist_accounts(
        tx: &rusqlite::Transaction,
        seed_hash: &[u8; 32],
        network: &str,
        accounts: &AccountChangeSet,
    ) -> Result<(), rusqlite::Error> {
        Self::ensure_account_state_table(tx)?;

        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO wallet_account_state
                (seed_hash, account_index, path_reference, last_revealed, network)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        for (&(account_index, path_ref), &last_revealed) in &accounts.last_revealed {
            stmt.execute(rusqlite::params![
                &seed_hash[..],
                account_index as i64,
                path_ref as u32 as i64,
                last_revealed as i64,
                network,
            ])?;
        }
        Ok(())
    }

    /// Persist key-wallet account changeset (last revealed indices without path reference).
    ///
    /// The key-wallet [`key_wallet::changeset::AccountChangeSet`] maps
    /// `account_index -> last_revealed` without a `DerivationPathReference` dimension.
    /// We store these with `path_reference = 0` (Unknown) as a sentinel.
    fn persist_key_wallet_accounts(
        tx: &rusqlite::Transaction,
        seed_hash: &[u8; 32],
        network: &str,
        accounts: &dash_sdk::dpp::key_wallet::changeset::AccountChangeSet,
    ) -> Result<(), rusqlite::Error> {
        Self::ensure_account_state_table(tx)?;

        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO wallet_account_state
                (seed_hash, account_index, path_reference, last_revealed, network)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        for (&account_index, &last_revealed) in &accounts.last_revealed {
            stmt.execute(rusqlite::params![
                &seed_hash[..],
                account_index as i64,
                0i64, // Unknown path reference — key-wallet does not carry one
                last_revealed as i64,
                network,
            ])?;
        }
        Ok(())
    }

    /// Persist identity changeset into the existing `identity` and `top_up` tables,
    /// and a `wallet_identity_dpns_names` table for DPNS names.
    fn persist_identities(
        tx: &rusqlite::Transaction,
        seed_hash: &[u8; 32],
        network: &str,
        identities: &IdentityChangeSet,
    ) -> Result<(), rusqlite::Error> {
        // Ensure the DPNS names table exists.
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS wallet_identity_dpns_names (
                identity_id BLOB NOT NULL,
                name TEXT NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY (identity_id, name, network)
            )",
        )?;

        let mut identity_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO identity
                (id, data, is_local, alias, wallet, wallet_index, network, status)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, 0)",
        )?;

        let mut topup_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO top_up (identity_id, top_up_index, amount)
             VALUES (?1, ?2, ?3)",
        )?;

        let mut dpns_stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO wallet_identity_dpns_names
                (identity_id, name, network)
             VALUES (?1, ?2, ?3)",
        )?;

        for (id, entry) in &identities.identities {
            let identity_bytes = entry
                .identity
                .serialize_to_bytes()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            identity_stmt.execute(rusqlite::params![
                id.as_bytes(),
                &identity_bytes,
                &entry.label,
                &seed_hash[..],
                entry.identity_index as i64,
                network,
            ])?;

            // Persist top-ups.
            for (&top_up_index, &amount) in &entry.top_ups {
                topup_stmt.execute(rusqlite::params![
                    id.as_bytes(),
                    top_up_index as i64,
                    amount as i64,
                ])?;
            }

            // Persist DPNS names.
            for name in &entry.dpns_names {
                dpns_stmt.execute(rusqlite::params![id.as_bytes(), name, network,])?;
            }
        }
        Ok(())
    }

    /// Persist platform address balances into the existing `platform_address_balances` table.
    fn persist_platform_addresses(
        tx: &rusqlite::Transaction,
        seed_hash: &[u8; 32],
        network: &str,
        addrs: &PlatformAddressChangeSet,
    ) -> Result<(), rusqlite::Error> {
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO platform_address_balances
                (seed_hash, address, balance, nonce, network, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))",
        )?;

        for (addr, entry) in &addrs.addresses {
            stmt.execute(rusqlite::params![
                &seed_hash[..],
                addr.as_bytes(),
                entry.credit_balance as i64,
                entry.nonce.unwrap_or(0) as i64,
                network,
            ])?;
        }
        Ok(())
    }

    /// Persist asset lock changeset into the existing `asset_lock_transaction` table.
    fn persist_asset_locks(
        tx: &rusqlite::Transaction,
        seed_hash: &[u8; 32],
        network: &str,
        asset_locks: &AssetLockChangeSet,
    ) -> Result<(), rusqlite::Error> {
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO asset_lock_transaction
                (tx_id, output_index, transaction_data, amount, identity_id, wallet, network,
                 chain_locked_height, account_index, funding_type, identity_index, proof_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;

        for (out_point, entry) in &asset_locks.asset_locks {
            let raw = serialize(&entry.transaction);
            // Encode chain-lock status as a height sentinel.
            let chain_locked_height: Option<i64> = if entry.status == AssetLockStatus::ChainLocked {
                Some(0) // chain-locked but exact height unknown from changeset
            } else {
                None
            };

            // Serialize AssetLockProof using bincode.
            let proof_bytes: Option<Vec<u8>> = entry.proof.as_ref().map(|p| {
                bincode::encode_to_vec(p, bincode::config::standard())
                    .expect("AssetLockProof bincode encoding should not fail")
            });

            // Map AssetLockFundingType to integer discriminant.
            let funding_type_int: i64 = funding_type_to_i64(entry.funding_type);

            stmt.execute(rusqlite::params![
                out_point.txid.as_byte_array(),
                out_point.vout as i64,
                &raw,
                entry.amount_duffs as i64,
                None::<Vec<u8>>, // identity_id: not tracked in changeset
                &seed_hash[..],
                network,
                chain_locked_height,
                entry.account_index as i64,
                funding_type_int,
                entry.identity_index as i64,
                proof_bytes,
            ])?;
        }
        Ok(())
    }
}

impl PlatformWalletPersistence for SqliteWalletPersister {
    fn load(
        &self,
        wallet_id: WalletId,
    ) -> Result<PlatformWalletChangeSet, Box<dyn std::error::Error + Send + Sync>> {
        let seed_hash = wallet_id;
        let conn = self.db.shared_connection();
        let guard = conn.lock().unwrap();

        // -- Load chain height ---------------------------------------------------
        let chain = {
            let maybe_height: Option<i64> = guard
                .query_row(
                    "SELECT last_terminal_block FROM wallet
                     WHERE seed_hash = ?1 AND network = ?2",
                    rusqlite::params![&seed_hash[..], &self.network],
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
            let mut rows = stmt.query(rusqlite::params![&seed_hash[..], &self.network])?;
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

        // -- Load balance -------------------------------------------------------
        let balance = {
            let row: Option<(i64, i64)> = guard
                .query_row(
                    "SELECT confirmed_balance, unconfirmed_balance FROM wallet
                     WHERE seed_hash = ?1 AND network = ?2",
                    rusqlite::params![&seed_hash[..], &self.network],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            row.and_then(|(confirmed, unconfirmed)| {
                if confirmed == 0 && unconfirmed == 0 {
                    None
                } else {
                    Some(dash_sdk::dpp::key_wallet::changeset::BalanceChangeSet {
                        spendable_delta: confirmed,
                        unconfirmed_delta: unconfirmed,
                        immature_delta: 0,
                        locked_delta: 0,
                    })
                }
            })
        };

        // -- Load accounts (last_revealed indices) ----------------------------
        let accounts = {
            // Table may not exist yet if persist() has never been called.
            let table_exists: bool = guard
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name='wallet_account_state'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);

            if table_exists {
                let mut stmt = guard.prepare(
                    "SELECT account_index, path_reference, last_revealed
                     FROM wallet_account_state
                     WHERE seed_hash = ?1 AND network = ?2",
                )?;
                let mut last_revealed = BTreeMap::new();
                let mut rows = stmt.query(rusqlite::params![&seed_hash[..], &self.network])?;
                while let Some(row) = rows.next()? {
                    let account_index: i64 = row.get(0)?;
                    let path_ref_val: i64 = row.get(1)?;
                    let revealed: i64 = row.get(2)?;
                    if let Some(path_ref) = derivation_path_reference_from_u32(path_ref_val as u32)
                    {
                        last_revealed.insert((account_index as u32, path_ref), revealed as u32);
                    }
                }
                if last_revealed.is_empty() {
                    None
                } else {
                    Some(AccountChangeSet { last_revealed })
                }
            } else {
                None
            }
        };

        // -- Load identities ---------------------------------------------------
        let identities = {
            let mut stmt = guard.prepare(
                "SELECT i.id, i.data, i.wallet_index, i.alias, t.top_up_index, t.amount
                 FROM identity i
                 LEFT JOIN top_up t ON i.id = t.identity_id
                 WHERE i.wallet = ?1 AND i.is_local = 1 AND i.network = ?2
                   AND i.data IS NOT NULL
                 ORDER BY i.id",
            )?;

            let mut map: BTreeMap<Identifier, IdentityEntry> = BTreeMap::new();
            let mut rows = stmt.query(rusqlite::params![&seed_hash[..], &self.network])?;
            while let Some(row) = rows.next()? {
                let id_bytes: Vec<u8> = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let wallet_index: Option<i64> = row.get(2)?;
                let alias: Option<String> = row.get(3)?;
                let top_up_index: Option<i64> = row.get(4)?;
                let top_up_amount: Option<i64> = row.get(5)?;

                let Ok(id) = Identifier::from_bytes(&id_bytes) else {
                    continue;
                };
                let Ok(identity) =
                    dash_sdk::dpp::identity::Identity::deserialize_from_bytes_no_limit(&data)
                else {
                    continue;
                };

                let entry = map.entry(id).or_insert_with(|| IdentityEntry {
                    identity,
                    identity_index: wallet_index.unwrap_or(0) as u32,
                    label: alias,
                    last_updated_balance_block_time: None,
                    last_synced_keys_block_time: None,
                    dpns_names: Vec::new(),
                    top_ups: BTreeMap::new(),
                });

                // Accumulate top-ups from the JOIN rows.
                if let (Some(ti), Some(ta)) = (top_up_index, top_up_amount) {
                    entry.top_ups.insert(ti as u32, ta as u64);
                }
            }

            // Load DPNS names (table may not exist).
            let dpns_table_exists: bool = guard
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name='wallet_identity_dpns_names'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);

            if dpns_table_exists {
                let mut dpns_stmt = guard.prepare(
                    "SELECT identity_id, name FROM wallet_identity_dpns_names
                     WHERE network = ?1",
                )?;
                let mut rows = dpns_stmt.query(rusqlite::params![&self.network])?;
                while let Some(row) = rows.next()? {
                    let id_bytes: Vec<u8> = row.get(0)?;
                    let name: String = row.get(1)?;
                    if let Ok(id) = Identifier::from_bytes(&id_bytes)
                        && let Some(entry) = map.get_mut(&id)
                        && !entry.dpns_names.contains(&name)
                    {
                        entry.dpns_names.push(name);
                    }
                }
            }

            if map.is_empty() {
                None
            } else {
                Some(IdentityChangeSet { identities: map })
            }
        };

        // -- Load platform address balances ------------------------------------
        let platform_addresses = {
            let mut stmt = guard.prepare(
                "SELECT address, balance, nonce FROM platform_address_balances
                 WHERE seed_hash = ?1 AND network = ?2",
            )?;
            let mut addresses = BTreeMap::new();
            let mut rows = stmt.query(rusqlite::params![&seed_hash[..], &self.network])?;
            while let Some(row) = rows.next()? {
                let addr_bytes: Vec<u8> = row.get(0)?;
                let credit_balance: i64 = row.get(1)?;
                let nonce: i64 = row.get(2)?;

                let Ok(addr) = PlatformP2PKHAddress::from_slice(&addr_bytes) else {
                    continue;
                };
                addresses.insert(
                    addr,
                    PlatformAddressEntry {
                        credit_balance: credit_balance as u64,
                        nonce: if nonce > 0 { Some(nonce as u64) } else { None },
                    },
                );
            }
            if addresses.is_empty() {
                None
            } else {
                Some(PlatformAddressChangeSet { addresses })
            }
        };

        // -- Load asset locks ---------------------------------------------------
        let asset_locks = {
            let mut stmt = guard.prepare(
                "SELECT tx_id, output_index, transaction_data, amount, identity_id,
                        chain_locked_height, instant_lock_data,
                        account_index, funding_type, identity_index, proof_data
                 FROM asset_lock_transaction
                 WHERE wallet = ?1 AND network = ?2",
            )?;
            let mut locks = BTreeMap::new();
            let mut rows = stmt.query(rusqlite::params![&seed_hash[..], &self.network])?;
            while let Some(row) = rows.next()? {
                let txid_bytes: Vec<u8> = row.get(0)?;
                let output_index: Option<i64> = row.get(1)?;
                let raw: Vec<u8> = row.get(2)?;
                let amount: i64 = row.get(3)?;
                let identity_id_bytes: Option<Vec<u8>> = row.get(4)?;
                let chain_locked_height: Option<i64> = row.get(5)?;
                let islock_bytes: Option<Vec<u8>> = row.get(6)?;
                let account_index: Option<i64> = row.get(7)?;
                let funding_type_int: Option<i64> = row.get(8)?;
                let identity_index: Option<i64> = row.get(9)?;
                let proof_bytes: Option<Vec<u8>> = row.get(10)?;

                let Ok(txid) = Txid::from_slice(&txid_bytes) else {
                    continue;
                };
                let Ok(transaction) = deserialize::<Transaction>(&raw) else {
                    continue;
                };
                let _identity_id = identity_id_bytes
                    .as_deref()
                    .and_then(|b| Identifier::from_bytes(b).ok());

                let vout = output_index.unwrap_or(0) as u32;
                let out_point = OutPoint { txid, vout };

                let funding_type = funding_type_int
                    .and_then(funding_type_from_i64)
                    .unwrap_or(AssetLockFundingType::IdentityRegistration);

                // Deserialize proof from bincode bytes, if present.
                let proof: Option<AssetLockProof> = proof_bytes.and_then(|bytes| {
                    bincode::decode_from_slice::<AssetLockProof, _>(
                        &bytes,
                        bincode::config::standard(),
                    )
                    .ok()
                    .map(|(p, _)| p)
                });

                locks.insert(
                    out_point,
                    AssetLockEntry {
                        out_point,
                        transaction,
                        account_index: account_index.unwrap_or(0) as u32,
                        funding_type,
                        identity_index: identity_index.unwrap_or(0) as u32,
                        amount_duffs: amount as u64,
                        status: if chain_locked_height.is_some() {
                            AssetLockStatus::ChainLocked
                        } else if islock_bytes.is_some() {
                            AssetLockStatus::InstantSendLocked
                        } else {
                            AssetLockStatus::Broadcast
                        },
                        proof,
                    },
                );
            }
            if locks.is_empty() {
                None
            } else {
                Some(AssetLockChangeSet { asset_locks: locks })
            }
        };

        // Build a key-wallet changeset if we have balance data.
        let wallet = balance.map(
            |bal| dash_sdk::dpp::key_wallet::changeset::WalletChangeSet {
                balance: Some(bal),
                ..Default::default()
            },
        );

        let cs = PlatformWalletChangeSet {
            chain,
            transactions,
            utxos,
            accounts,
            identities,
            platform_addresses,
            asset_locks,
            wallet,
            ..Default::default()
        };

        if cs.is_empty() {
            Ok(PlatformWalletChangeSet::default())
        } else {
            Ok(cs)
        }
    }

    fn store(&self, wallet_id: WalletId, changeset: PlatformWalletChangeSet) {
        {
            let mut staged = self.staged.lock().unwrap();
            staged
                .entry(wallet_id)
                .or_insert_with(PlatformWalletChangeSet::default)
                .merge(changeset);
        }
        if matches!(self.strategy, FlushStrategy::Immediate) {
            if let Err(e) = self.flush(wallet_id) {
                tracing::warn!(error = %e, "Auto-flush after queue failed");
            }
        }
    }

    fn flush(&self, wallet_id: WalletId) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let changeset = {
            let mut staged = self.staged.lock().unwrap();
            staged.remove(&wallet_id).unwrap_or_default()
        };
        if changeset.is_empty() {
            return Ok(());
        }
        self.persist_inner(&wallet_id, &changeset)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

impl SqliteWalletPersister {
    /// Internal persist implementation used by [`flush`].
    fn persist_inner(
        &self,
        wallet_id: &WalletId,
        changeset: &PlatformWalletChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let seed_hash = wallet_id;
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
                rusqlite::params![height as i64, &seed_hash[..], &self.network],
            )?;
        }

        // -- Transactions ------------------------------------------------------
        if let Some(ref txs) = changeset.transactions {
            Self::persist_transactions(&tx, &seed_hash, &self.network, txs)?;
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
                    rusqlite::params![height as i64, &seed_hash[..], &self.network],
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
                        &seed_hash[..],
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

            // wallet.accounts → persist last_revealed indices (key-wallet type)
            if let Some(ref kw_accounts) = wallet_cs.accounts {
                Self::persist_key_wallet_accounts(
                    &tx,
                    &seed_hash,
                    &self.network,
                    kw_accounts,
                )?;
            }

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
                        &seed_hash[..],
                        &self.network,
                    ],
                )?;
            }
        }

        // -- Accounts ----------------------------------------------------------
        if let Some(ref accounts) = changeset.accounts {
            Self::persist_accounts(&tx, &seed_hash, &self.network, accounts)?;
        }

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
        if let Some(ref identities) = changeset.identities {
            Self::persist_identities(&tx, &seed_hash, &self.network, identities)?;
        }

        // -- Platform addresses ------------------------------------------------
        if let Some(ref platform_addrs) = changeset.platform_addresses {
            Self::persist_platform_addresses(&tx, &seed_hash, &self.network, platform_addrs)?;
        }

        // -- Asset locks -------------------------------------------------------
        if let Some(ref asset_locks) = changeset.asset_locks {
            Self::persist_asset_locks(&tx, &seed_hash, &self.network, asset_locks)?;
        }

        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;

    const TEST_WALLET_ID: WalletId = [0u8; 32];

    fn make_persister(db: Arc<Database>) -> SqliteWalletPersister {
        SqliteWalletPersister::new(db, "testnet".to_string())
    }

    /// A persister can be created and initialized without error.
    #[test]
    fn test_initialize_returns_empty_changeset() {
        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db);

        let cs = persister.initialize(TEST_WALLET_ID).expect("initialize");
        assert!(cs.is_empty());
    }

    /// Persisting an empty changeset succeeds (no-op transaction).
    #[test]
    fn test_persist_empty_changeset() {
        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db);

        let cs = PlatformWalletChangeSet::default();
        persister.store(TEST_WALLET_ID, cs);
        persister.flush(TEST_WALLET_ID).expect("flush empty changeset");
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

        let persister = make_persister(db.clone());

        let cs = PlatformWalletChangeSet {
            chain: Some(ChainChangeSet {
                height: Some(12345),
                block_hash: None,
            }),
            ..Default::default()
        };
        persister.store(TEST_WALLET_ID, cs);
        persister.flush(TEST_WALLET_ID).expect("flush chain changeset");

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
        let persister = make_persister(db.clone());

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
        persister.store(TEST_WALLET_ID, cs);
        persister.flush(TEST_WALLET_ID).expect("flush add utxo");

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
        persister.store(TEST_WALLET_ID, cs2);
        persister.flush(TEST_WALLET_ID).expect("flush spend utxo");

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

        let persister = make_persister(db.clone());

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
        persister.store(TEST_WALLET_ID, cs);
        persister.flush(TEST_WALLET_ID).expect("flush changeset");

        // Now initialize and verify the state was loaded.
        let loaded = persister.initialize(TEST_WALLET_ID).expect("initialize");
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
