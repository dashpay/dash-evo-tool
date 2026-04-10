//! SQLite-backed implementation of [`PlatformWalletPersistence`].
//!
//! # Scope (Phase 9a-5d)
//!
//! Today the persister handles only the wallet state that has no
//! backend-task owner: chain sync height (`wallet.last_terminal_block`)
//! and SPV-driven UTXO updates (`utxos`). Everything else —
//! identities, contacts, asset locks, platform addresses, token
//! balances — is owned by backend tasks via direct
//! `Database::*` writers (`insert_local_qualified_identity`,
//! `save_dashpay_contact`, `store_asset_lock_transaction`,
//! `set_platform_address_info`, `insert_identity_token_balance`).
//!
//! Earlier revisions of this file (the Phase 9a-5a rewrite) tried to
//! be the sole writer for every sub-changeset, but the
//! `identity` table actually stores serialized `QualifiedIdentity`
//! blobs (an evo-tool wrapper around `dpp::Identity` that the
//! platform-wallet doesn't know about). The persister was writing
//! raw `Identity` bytes into the same column, latently corrupting
//! the blob on every flush. Same shape mismatch existed for the
//! other sub-changesets to varying degrees.
//!
//! The pragmatic resolution is for the persister to stop writing
//! state that backend tasks already own. Phase 9b will revisit and
//! either:
//!
//! - Move `QualifiedIdentity` (and similar evo-tool wrappers) into
//!   platform-wallet (or a shared crate) so the persister can be the
//!   sole writer end-to-end, OR
//! - Introduce a serializer abstraction so the persister can write
//!   evo-tool's wrapper format without depending on its types.
//!
//! Until then, platform-side sub-changesets that arrive in the
//! buffered `staged` map are silently dropped on flush — the
//! `tracing::debug!` log records what was discarded so cross-checks
//! against backend-task DB writes can verify nothing was lost.
//!
//! # Atomicity
//!
//! Every `flush` writes inside one `rusqlite::Transaction`. Partial
//! failures roll back. The staged accumulator is cleared on flush
//! regardless of outcome.

use crate::database::Database;
use platform_wallet::changeset::{Merge, PlatformWalletChangeSet, PlatformWalletPersistence};
use platform_wallet::wallet::WalletId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use dash_sdk::dpp::dashcore::hashes::Hash;

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
/// See the module docs for the current scope. The persister is
/// shared across all wallets managed by a `PlatformWalletManager`;
/// each wallet is identified by its `WalletId` (which equals the
/// evo-tool `seed_hash`).
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
        _wallet_id: WalletId,
    ) -> Result<PlatformWalletChangeSet, Box<dyn std::error::Error + Send + Sync>> {
        // Load is intentionally a no-op. Identities, contacts, asset
        // locks, platform addresses, and token balances are loaded by
        // their respective evo-tool domain helpers
        // (`get_local_qualified_identities`,
        // `get_asset_lock_transactions_for_wallet`,
        // `get_all_platform_address_info`, etc.) and mirrored into
        // the platform-wallet's in-memory state via `wallet_lifecycle`.
        // Chain height is read directly from `wallet.last_terminal_block`
        // by the SPV layer on startup.
        //
        // The persister returns an empty changeset so the
        // `platform_wallet.apply()` call in `manager.rs` is a no-op
        // until Phase 9b moves real load logic in here.
        Ok(PlatformWalletChangeSet::default())
    }
}

// ---------------------------------------------------------------------------
// Flush — atomic write of one full PlatformWalletChangeSet
// ---------------------------------------------------------------------------

impl SqliteWalletPersister {
    /// Internal flush — drains the changeset by value into one
    /// `rusqlite::Transaction`. Only the `core` sub-changeset is
    /// actually written; the platform-side sub-changesets are
    /// silently dropped (logged at debug) per the module-level scope
    /// note.
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

        // Log what we're dropping so cross-checks against backend-task
        // direct writes can confirm nothing was meant to land here.
        if identities
            .as_ref()
            .map(|c| !<_ as Merge>::is_empty(c))
            .unwrap_or(false)
        {
            tracing::debug!(
                "persister: dropping IdentityChangeSet (backend tasks own identity persistence)"
            );
        }
        if contacts
            .as_ref()
            .map(|c| !<_ as Merge>::is_empty(c))
            .unwrap_or(false)
        {
            tracing::debug!(
                "persister: dropping ContactChangeSet (backend tasks own contact persistence)"
            );
        }
        if platform_addresses
            .as_ref()
            .map(|c| !<_ as Merge>::is_empty(c))
            .unwrap_or(false)
        {
            tracing::debug!(
                "persister: dropping PlatformAddressChangeSet (backend tasks own platform address persistence)"
            );
        }
        if asset_locks
            .as_ref()
            .map(|c| !<_ as Merge>::is_empty(c))
            .unwrap_or(false)
        {
            tracing::debug!(
                "persister: dropping AssetLockChangeSet (backend tasks own asset lock persistence)"
            );
        }
        if token_balances
            .as_ref()
            .map(|c| !<_ as Merge>::is_empty(c))
            .unwrap_or(false)
        {
            tracing::debug!(
                "persister: dropping TokenBalanceChangeSet (backend tasks own token balance persistence)"
            );
        }

        let core = match core {
            Some(core) if !<_ as platform_wallet::changeset::Merge>::is_empty(&core) => core,
            _ => return Ok(()),
        };

        let conn = self.db.shared_connection();
        let mut guard = conn.lock().unwrap();
        let tx = guard.transaction()?;

        Self::write_core(&tx, &wallet_id, &self.network, core)?;

        tx.commit()?;
        Ok(())
    }

    /// Write the core wallet sub-changeset (`key_wallet::WalletChangeSet`).
    ///
    /// Today this covers chain height (`wallet.last_terminal_block`)
    /// and per-account UTXO inserts/deletes (`utxos`). The other
    /// per-account fields (`addresses_used`, `highest_used`,
    /// `utxos_instant_locked`, `transactions`) are deferred to
    /// Phase 9b — SPV is the authoritative writer for the
    /// `wallet_transactions` table on this branch, and address-pool
    /// state is rebuilt on load via gap-limit scanning.
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

        // Per-account UTXO writes. Drain the per_account map by value
        // so each `Utxo` and the `BTreeSet`s move directly into the
        // SQL params with no extra clones beyond what rusqlite needs.
        let mut insert_utxo = tx.prepare_cached(
            "INSERT OR IGNORE INTO utxos (txid, vout, address, value, script_pubkey, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut delete_utxo =
            tx.prepare_cached("DELETE FROM utxos WHERE txid = ?1 AND vout = ?2 AND network = ?3")?;

        for (_account_type, bucket) in core.per_account {
            for (outpoint, utxo) in bucket.utxos_added {
                insert_utxo.execute(rusqlite::params![
                    outpoint.txid.as_byte_array(),
                    outpoint.vout as i64,
                    utxo.address.to_string(),
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
            // and `transactions` are deferred — see module-level scope
            // note. Phase 9b will reconcile.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;

    const TEST_WALLET_ID: WalletId = [0u8; 32];

    fn make_persister(db: Arc<Database>) -> SqliteWalletPersister {
        SqliteWalletPersister::new(db, "testnet".to_string())
    }

    /// `load` is a no-op in the current scope: returns an empty
    /// changeset regardless of database state. The actual loading
    /// happens via the evo-tool domain helpers.
    #[test]
    fn test_load_returns_empty_changeset() {
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
