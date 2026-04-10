//! SQLite-backed implementation of [`PlatformWalletPersistence`].
//!
//! Persists [`PlatformWalletChangeSet`] deltas into evo-tool's existing
//! database tables (`wallet`, `wallet_account_state`, `utxos`,
//! `wallet_transactions`, `identity`, `top_up`, `dashpay_contacts`,
//! `dashpay_contact_requests`, `platform_address_balances`,
//! `asset_lock_transaction`, `identity_token_balance`).
//!
//! # Shape
//!
//! Phase 9a-5: this is a thin adapter against the new
//! [`PlatformWalletChangeSet`] shape (post-Phase 9a-3). Each sub-changeset
//! drains into a small group of writer calls or inline `INSERT` /
//! `DELETE` statements:
//!
//! - `cs.core` ([`key_wallet::changeset::WalletChangeSet`]) — chain
//!   height, per-account UTXOs / transactions / address pool
//!   watermarks. Only `chain.synced_height` and the per-account
//!   `utxos_added` / `utxos_spent` lists are persisted today.
//! - `cs.identities` — `INSERT OR REPLACE` into the `identity` table
//!   with the serialized dpp::Identity blob plus a per-identity DPNS
//!   names side table; `top_ups` go to the `top_up` table.
//! - `cs.contacts` — sent / incoming requests via `save_contact_request`,
//!   established contacts via `save_dashpay_contact`.
//! - `cs.platform_addresses` — `set_platform_address_info` /
//!   `delete_platform_address_info`.
//! - `cs.asset_locks` — inline `INSERT OR REPLACE` into
//!   `asset_lock_transaction` (the existing helper doesn't capture all
//!   the new fields).
//! - `cs.token_balances` — inline `INSERT OR REPLACE` into
//!   `identity_token_balance` (mirrors `insert_identity_token_balance`'s
//!   shape but uses the network owned by the persister).
//!
//! # Atomicity
//!
//! All writes for a single `flush` happen inside one
//! `rusqlite::Transaction`, so a partial failure rolls back the
//! entire batch. The staged in-memory accumulator is cleared on
//! flush regardless of outcome — callers re-emit changesets if a
//! flush fails because the wallet's in-memory state is the
//! authoritative source until the next persister round-trip.

use crate::database::Database;
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{OutPoint, Transaction, Txid};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::asset_lock_builder::AssetLockFundingType;
use dash_sdk::dpp::prelude::{AssetLockProof, Identifier};
use dash_sdk::dpp::serialization::{PlatformDeserializable, PlatformSerializable};
use platform_wallet::changeset::{
    AssetLockChangeSet, AssetLockEntry, ContactChangeSet, IdentityChangeSet, IdentityEntry, Merge,
    PlatformAddressChangeSet, PlatformWalletChangeSet, PlatformWalletPersistence,
    TokenBalanceChangeSet,
};
use platform_wallet::wallet::identity::managed_identity::DpnsNameInfo;
use platform_wallet::wallet::WalletId;
use platform_wallet::AssetLockStatus;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// Controls when queued changesets are written to storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushStrategy {
    /// Flush to storage after every [`store`](PlatformWalletPersistence::store) call.
    Immediate,
    /// Caller must explicitly call [`flush`](PlatformWalletPersistence::flush).
    Manual,
}

/// Persists [`PlatformWalletChangeSet`] deltas into the evo-tool SQLite database.
///
/// Changesets are buffered via [`store`](PlatformWalletPersistence::store) and
/// written atomically on [`flush`](PlatformWalletPersistence::flush).
///
/// When [`FlushStrategy::Immediate`] is set (the default), each `store()` call
/// automatically triggers a `flush()`, so callers don't need to call
/// flush separately.
///
/// A single persister instance is shared across all wallets managed by a
/// `PlatformWalletManager`. Each wallet is identified by its `WalletId`
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
    #[error("identity serialization failed: {0}")]
    IdentitySerialize(String),
}

impl SqliteWalletPersister {
    /// Create a new persister for the given `network`.
    ///
    /// Uses [`FlushStrategy::Immediate`] by default so that every
    /// `store()` call is automatically persisted.
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

// ---------------------------------------------------------------------------
// Persister trait impl — store / flush / load
// ---------------------------------------------------------------------------

impl PlatformWalletPersistence for SqliteWalletPersister {
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
                tracing::warn!(error = %e, "Auto-flush after store failed");
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
        self.flush_inner(wallet_id, changeset)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn load(
        &self,
        wallet_id: WalletId,
    ) -> Result<PlatformWalletChangeSet, Box<dyn std::error::Error + Send + Sync>> {
        self.load_inner(wallet_id)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

// ---------------------------------------------------------------------------
// Flush — atomic write of one full PlatformWalletChangeSet
// ---------------------------------------------------------------------------

impl SqliteWalletPersister {
    /// Internal flush — drains the changeset by value into one
    /// `rusqlite::Transaction`. Each sub-changeset writer is a private
    /// helper below.
    fn flush_inner(
        &self,
        wallet_id: WalletId,
        cs: PlatformWalletChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let PlatformWalletChangeSet {
            core,
            identities,
            contacts,
            platform_addresses,
            asset_locks,
            token_balances,
        } = cs;

        let conn = self.db.shared_connection();
        let mut guard = conn.lock().unwrap();
        let tx = guard.transaction()?;

        if let Some(core) = core {
            Self::write_core(&tx, &wallet_id, &self.network, core)?;
        }
        if let Some(id_cs) = identities {
            Self::write_identities(&tx, &wallet_id, &self.network, id_cs)?;
        }
        if let Some(contact_cs) = contacts {
            Self::write_contacts(&tx, &self.network, contact_cs)?;
        }
        if let Some(addr_cs) = platform_addresses {
            Self::write_platform_addresses(&tx, &wallet_id, &self.network, addr_cs)?;
        }
        if let Some(al_cs) = asset_locks {
            Self::write_asset_locks(&tx, &wallet_id, &self.network, al_cs)?;
        }
        if let Some(tok_cs) = token_balances {
            Self::write_token_balances(&tx, &self.network, tok_cs)?;
        }

        tx.commit()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Per-sub-changeset writers
// ---------------------------------------------------------------------------

impl SqliteWalletPersister {
    /// Write the core wallet sub-changeset (`key_wallet::WalletChangeSet`).
    ///
    /// Today the persister covers chain height and per-account UTXO
    /// adds/spends. Per-account `transactions` and `addresses_used` /
    /// `highest_used` are deferred — SPV is the authoritative writer
    /// for the wallet_transactions table, and address-pool watermarks
    /// are recomputed on load via gap-limit scanning.
    fn write_core(
        tx: &rusqlite::Transaction,
        wallet_id: &WalletId,
        network: &str,
        core: dash_sdk::dpp::key_wallet::changeset::WalletChangeSet,
    ) -> Result<(), SqlitePersistError> {
        // Chain height — UPDATE wallet.last_terminal_block.
        if let Some(chain) = core.chain {
            if let Some(height) = chain.synced_height {
                tx.execute(
                    "UPDATE wallet SET last_terminal_block = ?1
                     WHERE seed_hash = ?2 AND network = ?3",
                    rusqlite::params![height as i64, &wallet_id[..], network],
                )?;
            }
        }

        // Per-account UTXO writes. We drain the per_account map by value
        // so each `Utxo` and the BTreeSets move directly into the SQL
        // params with no extra clones beyond what rusqlite needs.
        let mut insert_utxo = tx.prepare_cached(
            "INSERT OR IGNORE INTO utxos (txid, vout, address, value, script_pubkey, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut delete_utxo =
            tx.prepare_cached("DELETE FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3")?;

        for (_account_type, bucket) in core.per_account {
            for (outpoint, utxo) in bucket.utxos_added {
                let address_str = utxo.address.to_string();
                insert_utxo.execute(rusqlite::params![
                    outpoint.txid.as_byte_array(),
                    outpoint.vout as i64,
                    address_str,
                    utxo.txout.value as i64,
                    utxo.txout.script_pubkey.as_bytes(),
                    network,
                ])?;
            }
            for outpoint in bucket.utxos_spent {
                delete_utxo.execute(rusqlite::params![
                    outpoint.txid.as_byte_array(),
                    outpoint.vout as i64,
                    network,
                ])?;
            }
            // `addresses_used`, `highest_used`, `utxos_instant_locked`,
            // and `transactions` are deferred — SPV is the authoritative
            // writer for the wallet_transactions table on this branch,
            // and address-pool state is rebuilt on load via gap-limit
            // scanning. Phase 9b will reconcile.
            let _ = bucket.addresses_used;
            let _ = bucket.highest_used;
            let _ = bucket.utxos_instant_locked;
            let _ = bucket.transactions;
        }

        // `account_keys` and `balance` are intentionally not persisted:
        // account_keys is re-derived from the seed on load, and balance
        // is recomputed from the restored UTXO set via update_balance().
        let _ = core.account_keys;
        let _ = core.balance;

        Ok(())
    }

    /// Write the identity sub-changeset.
    ///
    /// Inserts the dpp::Identity blob into the `identity` table, plus
    /// per-identity top_ups into `top_up` and DPNS name metadata into
    /// the `wallet_identity_dpns_names` side table (created on demand
    /// if absent — it lives in this persister, not the main schema).
    ///
    /// Tombstones drop the identity row + its dependent rows.
    fn write_identities(
        tx: &rusqlite::Transaction,
        wallet_id: &WalletId,
        network: &str,
        id_cs: IdentityChangeSet,
    ) -> Result<(), SqlitePersistError> {
        // Ensure DPNS names side table exists (it's not in the base
        // schema; the persister owns it).
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS wallet_identity_dpns_names (
                identity_id BLOB NOT NULL,
                label TEXT NOT NULL,
                acquired_at INTEGER,
                network TEXT NOT NULL,
                PRIMARY KEY (identity_id, label, network)
            )",
        )?;

        let IdentityChangeSet {
            identities,
            removed,
            primary_identity,
            last_scanned_index,
        } = id_cs;

        // Removals first so that an INSERT-then-REMOVE in the same
        // changeset (latent merge ordering hazard documented on
        // IdentityChangeSet) ends up "removed" — same policy as the
        // apply path.
        let mut delete_identity = tx
            .prepare_cached("DELETE FROM identity WHERE id = ?1 AND network = ?2")?;
        let mut delete_topups =
            tx.prepare_cached("DELETE FROM top_up WHERE identity_id = ?1")?;
        let mut delete_dpns = tx.prepare_cached(
            "DELETE FROM wallet_identity_dpns_names WHERE identity_id = ?1 AND network = ?2",
        )?;
        for id in &removed {
            delete_identity.execute(rusqlite::params![id.as_bytes(), network])?;
            delete_topups.execute(rusqlite::params![id.as_bytes()])?;
            delete_dpns.execute(rusqlite::params![id.as_bytes(), network])?;
        }

        // Inserts / updates.
        let mut insert_identity = tx.prepare_cached(
            "INSERT OR REPLACE INTO identity
                (id, data, is_local, alias, network, wallet, wallet_index, status)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut insert_topup = tx.prepare_cached(
            "INSERT OR REPLACE INTO top_up (identity_id, top_up_index, amount)
             VALUES (?1, ?2, ?3)",
        )?;
        let mut insert_dpns = tx.prepare_cached(
            "INSERT OR REPLACE INTO wallet_identity_dpns_names
                (identity_id, label, acquired_at, network)
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        for (id, entry) in identities {
            let identity_bytes = entry
                .identity
                .serialize_to_bytes()
                .map_err(|e| SqlitePersistError::IdentitySerialize(e.to_string()))?;
            insert_identity.execute(rusqlite::params![
                id.as_bytes(),
                identity_bytes,
                entry.label,
                network,
                &wallet_id[..],
                entry.identity_index as i64,
                // status: 0 = unknown, persistance-side concept; the
                // identity blob carries the authoritative state.
                0i32,
            ])?;
            for (top_up_index, amount) in entry.top_ups {
                insert_topup.execute(rusqlite::params![
                    id.as_bytes(),
                    top_up_index as i64,
                    amount as i64,
                ])?;
            }
            for name in entry.dpns_names {
                insert_dpns.execute(rusqlite::params![
                    id.as_bytes(),
                    name.label,
                    name.acquired_at.map(|v| v as i64),
                    network,
                ])?;
            }
        }

        // Wallet-level identity metadata: primary identity selection
        // and gap-limit scan watermark are wallet-scoped, not
        // per-identity. They live on the wallet row itself if a column
        // exists; for now they're not persisted (loader picks the
        // first identity by iteration order, scan resumes from 0).
        // Phase 9b will add columns when there's a concrete need.
        let _ = primary_identity;
        let _ = last_scanned_index;

        Ok(())
    }

    /// Write the contact sub-changeset.
    ///
    /// Sent / incoming requests delegate to `save_contact_request`
    /// (which writes `dashpay_contact_requests`); established contacts
    /// delegate to `save_dashpay_contact` (`dashpay_contacts`).
    fn write_contacts(
        tx: &rusqlite::Transaction,
        network: &str,
        contact_cs: ContactChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let ContactChangeSet {
            sent_requests,
            removed_sent,
            incoming_requests,
            removed_incoming,
            established,
        } = contact_cs;

        // Tombstones first.
        let mut delete_request = tx.prepare_cached(
            "DELETE FROM dashpay_contact_requests
             WHERE from_identity_id = ?1 AND to_identity_id = ?2 AND network = ?3",
        )?;
        for (owner, contact) in removed_sent {
            // sent: from = owner, to = contact
            delete_request.execute(rusqlite::params![
                owner.as_bytes(),
                contact.as_bytes(),
                network,
            ])?;
        }
        for (owner, contact) in removed_incoming {
            // incoming: from = contact, to = owner
            delete_request.execute(rusqlite::params![
                contact.as_bytes(),
                owner.as_bytes(),
                network,
            ])?;
        }

        // Inserts.
        let mut insert_request = tx.prepare_cached(
            "INSERT OR IGNORE INTO dashpay_contact_requests
                (from_identity_id, to_identity_id, network, request_type, status)
             VALUES (?1, ?2, ?3, ?4, 'pending')",
        )?;
        for ((owner, _other), entry) in sent_requests {
            // owner sent to recipient
            insert_request.execute(rusqlite::params![
                owner.as_bytes(),
                entry.request.recipient_id.as_bytes(),
                network,
                "sent",
            ])?;
        }
        for ((owner, _other), entry) in incoming_requests {
            // sender sent to owner
            insert_request.execute(rusqlite::params![
                entry.request.sender_id.as_bytes(),
                owner.as_bytes(),
                network,
                "received",
            ])?;
        }

        // Established contacts.
        let mut insert_contact = tx.prepare_cached(
            "INSERT OR REPLACE INTO dashpay_contacts
                (owner_identity_id, contact_identity_id, network, contact_status, updated_at)
             VALUES (?1, ?2, ?3, 'accepted', unixepoch())",
        )?;
        for ((owner, _other), contact) in established {
            insert_contact.execute(rusqlite::params![
                owner.as_bytes(),
                contact.contact_identity_id.as_bytes(),
                network,
            ])?;
        }

        Ok(())
    }

    /// Write the platform address balances sub-changeset.
    fn write_platform_addresses(
        tx: &rusqlite::Transaction,
        wallet_id: &WalletId,
        network: &str,
        addr_cs: PlatformAddressChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let PlatformAddressChangeSet { addresses, removed } = addr_cs;

        let mut delete_stmt = tx.prepare_cached(
            "DELETE FROM platform_address_balances
             WHERE seed_hash = ?1 AND address = ?2 AND network = ?3",
        )?;
        let mut insert_stmt = tx.prepare_cached(
            "INSERT INTO platform_address_balances
                (seed_hash, address, balance, nonce, network, updated_at)
             VALUES (?1, ?2, ?3, 0, ?4, unixepoch())
             ON CONFLICT(seed_hash, address, network) DO UPDATE SET
                balance = excluded.balance,
                updated_at = excluded.updated_at",
        )?;

        for addr in removed {
            delete_stmt.execute(rusqlite::params![
                &wallet_id[..],
                hex::encode(addr.to_bytes()),
                network,
            ])?;
        }
        for (addr, balance) in addresses {
            insert_stmt.execute(rusqlite::params![
                &wallet_id[..],
                hex::encode(addr.to_bytes()),
                balance as i64,
                network,
            ])?;
        }
        Ok(())
    }

    /// Write the asset lock sub-changeset.
    ///
    /// `asset_lock_transaction` carries 12 columns the existing
    /// `store_asset_lock_transaction` helper doesn't all expose, so
    /// we use raw SQL with all fields.
    fn write_asset_locks(
        tx: &rusqlite::Transaction,
        wallet_id: &WalletId,
        network: &str,
        al_cs: AssetLockChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let AssetLockChangeSet {
            asset_locks,
            removed,
        } = al_cs;

        let mut delete_stmt = tx.prepare_cached(
            "DELETE FROM asset_lock_transaction WHERE tx_id = ?1 AND output_index = ?2",
        )?;
        for out_point in removed {
            delete_stmt.execute(rusqlite::params![
                out_point.txid.as_byte_array(),
                out_point.vout as i64,
            ])?;
        }

        let mut insert_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO asset_lock_transaction
                (tx_id, output_index, transaction_data, amount, identity_id, wallet, network,
                 chain_locked_height, account_index, funding_type, identity_index, proof_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for (out_point, entry) in asset_locks {
            let raw = serialize(&entry.transaction);
            // Encode chain-lock status as a height sentinel; exact
            // height is not in the changeset.
            let chain_locked_height: Option<i64> =
                if entry.status == AssetLockStatus::ChainLocked {
                    Some(0)
                } else {
                    None
                };
            let proof_bytes: Option<Vec<u8>> = entry.proof.as_ref().map(|p| {
                bincode::encode_to_vec(p, bincode::config::standard())
                    .expect("AssetLockProof bincode encoding should not fail")
            });
            insert_stmt.execute(rusqlite::params![
                out_point.txid.as_byte_array(),
                out_point.vout as i64,
                raw,
                entry.amount_duffs as i64,
                None::<Vec<u8>>, // identity_id: not tracked in the changeset
                &wallet_id[..],
                network,
                chain_locked_height,
                entry.account_index as i64,
                funding_type_to_i64(entry.funding_type),
                entry.identity_index as i64,
                proof_bytes,
            ])?;
        }
        Ok(())
    }

    /// Write the token balances sub-changeset.
    ///
    /// Writes into the existing `identity_token_balances` table, whose
    /// schema is `(token_id, identity_id, balance, network)` with a
    /// composite primary key. Tombstones drop matching rows; balance
    /// upserts use `INSERT OR REPLACE`.
    fn write_token_balances(
        tx: &rusqlite::Transaction,
        network: &str,
        tok_cs: TokenBalanceChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let TokenBalanceChangeSet {
            balances,
            removed_balances,
            watched: _,
            unwatched: _,
        } = tok_cs;

        let mut delete_stmt = tx.prepare_cached(
            "DELETE FROM identity_token_balances
             WHERE identity_id = ?1 AND token_id = ?2 AND network = ?3",
        )?;
        for (identity_id, token_id) in removed_balances {
            delete_stmt.execute(rusqlite::params![
                identity_id.as_bytes(),
                token_id.as_bytes(),
                network,
            ])?;
        }

        let mut insert_stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO identity_token_balances
                (token_id, identity_id, balance, network)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for ((identity_id, token_id), balance) in balances {
            insert_stmt.execute(rusqlite::params![
                token_id.as_bytes(),
                identity_id.as_bytes(),
                balance as i64,
                network,
            ])?;
        }

        // `watched` / `unwatched` registry deltas are not persisted:
        // the watch list is rebuilt at runtime from the set of
        // identities the wallet owns × token contracts the user has
        // added. Phase 9b will revisit if a dedicated watch table is
        // introduced.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Load — read all tables and build a `PlatformWalletChangeSet`
// ---------------------------------------------------------------------------

impl SqliteWalletPersister {
    fn load_inner(
        &self,
        wallet_id: WalletId,
    ) -> Result<PlatformWalletChangeSet, SqlitePersistError> {
        let conn = self.db.shared_connection();
        let guard = conn.lock().unwrap();

        let identities = Self::load_identities(&guard, &wallet_id, &self.network)?;
        let asset_locks = Self::load_asset_locks(&guard, &wallet_id, &self.network)?;
        let platform_addresses =
            Self::load_platform_addresses(&guard, &wallet_id, &self.network)?;
        let token_balances = Self::load_token_balances(&guard, &self.network)?;
        // Contacts and `cs.core` (chain height + per-account UTXOs)
        // are not loaded by the persister today: contacts are queried
        // directly from the dashpay tables by the UI on demand, and
        // SPV re-fetches the chain state on startup.
        // Phase 9b will reconcile.

        let cs = PlatformWalletChangeSet {
            core: None,
            identities,
            contacts: None,
            platform_addresses,
            asset_locks,
            token_balances,
        };

        Ok(cs)
    }

    fn load_identities(
        conn: &rusqlite::Connection,
        wallet_id: &WalletId,
        network: &str,
    ) -> Result<Option<IdentityChangeSet>, SqlitePersistError> {
        let mut stmt = conn.prepare(
            "SELECT i.id, i.data, i.wallet_index, i.alias, t.top_up_index, t.amount
             FROM identity i
             LEFT JOIN top_up t ON i.id = t.identity_id
             WHERE i.wallet = ?1 AND i.is_local = 1 AND i.network = ?2
               AND i.data IS NOT NULL
             ORDER BY i.id",
        )?;
        let mut map: BTreeMap<Identifier, IdentityEntry> = BTreeMap::new();
        let mut rows = stmt.query(rusqlite::params![&wallet_id[..], network])?;
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
            let Ok(identity) = Identity::deserialize_from_bytes_no_limit(&data) else {
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
                status: Default::default(),
                key_storage: BTreeMap::new(),
                wallet_seed_hash: Some(*wallet_id),
            });
            if let (Some(ti), Some(ta)) = (top_up_index, top_up_amount) {
                entry.top_ups.insert(ti as u32, ta as u64);
            }
        }

        // DPNS names side table — created on demand by the persister.
        let dpns_table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='wallet_identity_dpns_names'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if dpns_table_exists {
            let mut dpns_stmt = conn.prepare(
                "SELECT identity_id, label, acquired_at FROM wallet_identity_dpns_names
                 WHERE network = ?1",
            )?;
            let mut rows = dpns_stmt.query(rusqlite::params![network])?;
            while let Some(row) = rows.next()? {
                let id_bytes: Vec<u8> = row.get(0)?;
                let label: String = row.get(1)?;
                let acquired_at: Option<i64> = row.get(2)?;
                if let Ok(id) = Identifier::from_bytes(&id_bytes)
                    && let Some(entry) = map.get_mut(&id)
                    && !entry.dpns_names.iter().any(|n| n.label == label)
                {
                    entry.dpns_names.push(DpnsNameInfo {
                        label,
                        acquired_at: acquired_at.map(|v| v as u64),
                    });
                }
            }
        }

        if map.is_empty() {
            Ok(None)
        } else {
            Ok(Some(IdentityChangeSet {
                identities: map,
                removed: BTreeSet::new(),
                primary_identity: None,
                last_scanned_index: None,
            }))
        }
    }

    fn load_asset_locks(
        conn: &rusqlite::Connection,
        wallet_id: &WalletId,
        network: &str,
    ) -> Result<Option<AssetLockChangeSet>, SqlitePersistError> {
        let mut stmt = conn.prepare(
            "SELECT tx_id, output_index, transaction_data, amount, chain_locked_height,
                    instant_lock_data, account_index, funding_type, identity_index, proof_data
             FROM asset_lock_transaction
             WHERE wallet = ?1 AND network = ?2",
        )?;
        let mut locks = BTreeMap::new();
        let mut rows = stmt.query(rusqlite::params![&wallet_id[..], network])?;
        while let Some(row) = rows.next()? {
            let txid_bytes: Vec<u8> = row.get(0)?;
            let output_index: Option<i64> = row.get(1)?;
            let raw: Vec<u8> = row.get(2)?;
            let amount: i64 = row.get(3)?;
            let chain_locked_height: Option<i64> = row.get(4)?;
            let islock_bytes: Option<Vec<u8>> = row.get(5)?;
            let account_index: Option<i64> = row.get(6)?;
            let funding_type_int: Option<i64> = row.get(7)?;
            let identity_index: Option<i64> = row.get(8)?;
            let proof_bytes: Option<Vec<u8>> = row.get(9)?;

            let Ok(txid) = Txid::from_slice(&txid_bytes) else {
                continue;
            };
            let Ok(transaction) = deserialize::<Transaction>(&raw) else {
                continue;
            };
            let vout = output_index.unwrap_or(0) as u32;
            let out_point = OutPoint { txid, vout };

            let funding_type = funding_type_int
                .and_then(funding_type_from_i64)
                .unwrap_or(AssetLockFundingType::IdentityRegistration);
            let proof: Option<AssetLockProof> = proof_bytes.and_then(|bytes| {
                bincode::decode_from_slice::<AssetLockProof, _>(
                    &bytes,
                    bincode::config::standard(),
                )
                .ok()
                .map(|(p, _)| p)
            });
            let status = if chain_locked_height.is_some() {
                AssetLockStatus::ChainLocked
            } else if islock_bytes.is_some() {
                AssetLockStatus::InstantSendLocked
            } else {
                AssetLockStatus::Broadcast
            };

            locks.insert(
                out_point,
                AssetLockEntry {
                    out_point,
                    transaction,
                    account_index: account_index.unwrap_or(0) as u32,
                    funding_type,
                    identity_index: identity_index.unwrap_or(0) as u32,
                    amount_duffs: amount as u64,
                    status,
                    proof,
                },
            );
        }

        if locks.is_empty() {
            Ok(None)
        } else {
            Ok(Some(AssetLockChangeSet {
                asset_locks: locks,
                removed: BTreeSet::new(),
            }))
        }
    }

    fn load_platform_addresses(
        conn: &rusqlite::Connection,
        wallet_id: &WalletId,
        network: &str,
    ) -> Result<Option<PlatformAddressChangeSet>, SqlitePersistError> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        let mut stmt = conn.prepare(
            "SELECT address, balance FROM platform_address_balances
             WHERE seed_hash = ?1 AND network = ?2",
        )?;
        let mut addresses = BTreeMap::new();
        let mut rows = stmt.query(rusqlite::params![&wallet_id[..], network])?;
        while let Some(row) = rows.next()? {
            let addr_str: String = row.get(0)?;
            let balance: i64 = row.get(1)?;
            // Persisted as hex of PlatformAddress::to_bytes() — decode
            // and reparse. Skip rows that don't round-trip.
            let Ok(bytes) = hex::decode(&addr_str) else {
                continue;
            };
            let Ok(addr) = PlatformAddress::from_bytes(&bytes) else {
                continue;
            };
            addresses.insert(addr, balance as u64);
        }
        if addresses.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PlatformAddressChangeSet {
                addresses,
                removed: BTreeSet::new(),
            }))
        }
    }

    fn load_token_balances(
        conn: &rusqlite::Connection,
        network: &str,
    ) -> Result<Option<TokenBalanceChangeSet>, SqlitePersistError> {
        let mut stmt = conn.prepare(
            "SELECT identity_id, token_id, balance FROM identity_token_balances
             WHERE network = ?1",
        )?;
        let mut balances = BTreeMap::new();
        let mut rows = stmt.query(rusqlite::params![network])?;
        while let Some(row) = rows.next()? {
            let identity_id_bytes: Vec<u8> = row.get(0)?;
            let token_id_bytes: Vec<u8> = row.get(1)?;
            let balance: i64 = row.get(2)?;
            let Ok(identity_id) = Identifier::from_bytes(&identity_id_bytes) else {
                continue;
            };
            let Ok(token_id) = Identifier::from_bytes(&token_id_bytes) else {
                continue;
            };
            balances.insert((identity_id, token_id), balance as u64);
        }
        if balances.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenBalanceChangeSet {
                balances,
                removed_balances: BTreeSet::new(),
                watched: BTreeMap::new(),
                unwatched: BTreeMap::new(),
            }))
        }
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

    /// A persister can load an empty changeset from a fresh database.
    #[test]
    fn test_load_returns_empty_changeset_on_fresh_db() {
        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db);

        let cs = persister.load(TEST_WALLET_ID).expect("load");
        assert!(cs.is_empty());
    }

    /// Persisting an empty changeset is a no-op.
    #[test]
    fn test_persist_empty_changeset() {
        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db);

        let cs = PlatformWalletChangeSet::default();
        persister.store(TEST_WALLET_ID, cs);
        persister
            .flush(TEST_WALLET_ID)
            .expect("flush empty changeset");
    }
}
