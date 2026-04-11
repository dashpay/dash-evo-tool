//! SQLite-backed implementation of [`PlatformWalletPersistence`].
//!
//! # Scope (Phase 9b)
//!
//! The persister handles wallet state with no backend-task owner,
//! plus the DashPay subsets that Phase 9b migrated to the changeset
//! flow:
//!
//! - `core.chain` → `wallet.last_terminal_block` and SPV UTXO state
//!   (Phase 9a-5d).
//! - `identities.dashpay_profile` → `dashpay_profiles` table (9b-1).
//! - `identities.dashpay_payments` → `dashpay_payments` table (9b-2).
//! - `contacts.established[*].{highest_receive_index,
//!   bloom_registered_count, next_send_index}` →
//!   `dashpay_contact_address_indices` table (9b-3).
//!
//! Everything else — platform addresses, asset locks, token balances,
//! the QualifiedIdentity blob on `identity`, label, top_ups,
//! dpns_names, status, and the per-contact pending-request maps — is
//! still owned by backend tasks via direct `Database::*` writers.
//! Sub-changesets that arrive in the buffered `staged` map for those
//! tables are silently dropped on flush with a `tracing::debug!`
//! recording what was discarded.
//!
//! Earlier revisions of this file (the Phase 9a-5a rewrite) tried to
//! be the sole writer for every sub-changeset, but the `identity`
//! table stores serialized `QualifiedIdentity` blobs (an evo-tool
//! wrapper around `dpp::Identity` that the platform-wallet doesn't
//! know about) and the persister was latently corrupting them on
//! every flush. The pragmatic resolution is for the persister to
//! stop writing state that backend tasks already own. Phase 9c will
//! revisit and either move `QualifiedIdentity` (and similar evo-tool
//! wrappers) into a shared crate, or introduce a serializer
//! abstraction so the persister can round-trip evo-tool's wrapper
//! format without depending on its types.
//!
//! # Atomicity
//!
//! Every `flush` writes inside one `rusqlite::Transaction`. Partial
//! failures roll back. The staged accumulator is cleared on flush
//! regardless of outcome.

use crate::database::Database;
use platform_wallet::changeset::{
    IdentityChangeSet, Merge, PlatformWalletChangeSet, PlatformWalletPersistence,
};
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

        // Sub-changesets owned by backend tasks — drop with a
        // `tracing::debug!` for cross-checks.
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

        // Decide whether there's anything to write at all. We need a
        // transaction iff `core` carries something, OR the identities
        // sub-changeset has at least one entry with a DashPay field
        // (profile or payments), OR the contacts sub-changeset has
        // an `established` entry with non-default derivation state.
        //
        // For `contacts`, we skip rows where all three derivation
        // fields are zero because Phase 9a auto-establishment paths
        // emit pristine-default `EstablishedContact` snapshots as a
        // side effect of contact establishment. The derivation
        // mutations (`bump_*` / `set_*`) never emit all-zero by
        // construction and by contract — see
        // `bump_contact_highest_receive_index` and
        // `set_contact_bloom_registered_count` docs in
        // rs-platform-wallet for the persister-contract rationale.
        let has_core_work = core
            .as_ref()
            .map(|c| !<_ as platform_wallet::changeset::Merge>::is_empty(c))
            .unwrap_or(false);
        let has_dashpay_identity_work = identities
            .as_ref()
            .map(|id_cs| {
                id_cs.identities.values().any(|e| {
                    e.dashpay_profile.is_some() || !e.dashpay_payments.is_empty()
                })
            })
            .unwrap_or(false);
        let has_contact_derivation_work = contacts
            .as_ref()
            .map(|contact_cs| {
                contact_cs.established.values().any(|c| {
                    c.highest_receive_index != 0
                        || c.bloom_registered_count != 0
                        || c.next_send_index != 0
                })
            })
            .unwrap_or(false);
        if !has_core_work && !has_dashpay_identity_work && !has_contact_derivation_work {
            return Ok(());
        }

        let conn = self.db.shared_connection();
        let mut guard = conn.lock().unwrap();
        let tx = guard.transaction()?;

        if let Some(core) = core
            && has_core_work
        {
            Self::write_core(&tx, &wallet_id, &self.network, core)?;
        }
        if let Some(id_cs) = identities {
            Self::write_identity_dashpay_subset(&tx, &self.network, id_cs)?;
        }
        if let Some(contact_cs) = contacts {
            Self::write_contact_derivation_subset(&tx, contact_cs)?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Write the contact-derivation-state subset of a
    /// `ContactChangeSet` to the `dashpay_contact_address_indices`
    /// table.
    ///
    /// This is a SUBSET write — only the per-contact
    /// `highest_receive_index`, `bloom_registered_count`, and
    /// `next_send_index` fields of each `EstablishedContact` in
    /// `contacts.established` are consumed. The pending contact
    /// request maps (sent / incoming / removed) are dropped —
    /// backend tasks own those tables via
    /// `save_contact_request` / `save_dashpay_contact`.
    ///
    /// `highest_receive_index` is written with a `MAX(old, new)`
    /// merge so stale replay never regresses the value. The other
    /// two columns use last-write-wins.
    fn write_contact_derivation_subset(
        tx: &rusqlite::Transaction,
        contact_cs: platform_wallet::changeset::ContactChangeSet,
    ) -> Result<(), SqlitePersistError> {
        let mut upsert = tx.prepare_cached(
            "INSERT INTO dashpay_contact_address_indices
                (owner_identity_id, contact_identity_id, next_send_index,
                 highest_receive_index, bloom_registered_count)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_identity_id, contact_identity_id) DO UPDATE SET
                next_send_index = excluded.next_send_index,
                highest_receive_index = MAX(highest_receive_index, excluded.highest_receive_index),
                bloom_registered_count = excluded.bloom_registered_count",
        )?;

        for ((owner, contact_id), established) in contact_cs.established {
            // Skip no-op rows (all-zero defaults). The work-gate in
            // `flush_inner` already filters these out at the
            // changeset level; this is belt-and-suspenders in case a
            // caller hands us a changeset directly.
            if established.highest_receive_index == 0
                && established.bloom_registered_count == 0
                && established.next_send_index == 0
            {
                continue;
            }
            upsert.execute(rusqlite::params![
                owner.to_buffer().to_vec(),
                contact_id.to_buffer().to_vec(),
                established.next_send_index as i64,
                established.highest_receive_index as i64,
                established.bloom_registered_count as i64,
            ])?;
        }

        // Pending contact-request maps are backend-task-owned — log
        // the drop for cross-checks against direct
        // `save_contact_request` / `save_dashpay_contact` writes.
        if !contact_cs.sent_requests.is_empty()
            || !contact_cs.removed_sent.is_empty()
            || !contact_cs.incoming_requests.is_empty()
            || !contact_cs.removed_incoming.is_empty()
        {
            tracing::debug!(
                sent = contact_cs.sent_requests.len(),
                incoming = contact_cs.incoming_requests.len(),
                removed_sent = contact_cs.removed_sent.len(),
                removed_incoming = contact_cs.removed_incoming.len(),
                "persister: dropping ContactChangeSet pending/removed subsets (backend tasks own contact request persistence)"
            );
        }
        Ok(())
    }

    /// Write the DashPay-related subset of an `IdentityChangeSet`
    /// to the `dashpay_profiles` and `dashpay_payments` tables.
    ///
    /// This is a SUBSET write — only the `dashpay_profile` and
    /// `dashpay_payments` fields of each entry are consumed; the
    /// rest of the entry (the QualifiedIdentity blob, label,
    /// top_ups, dpns_names, status) is dropped because backend
    /// tasks own those tables until later 9b sub-phases. Entries
    /// whose profile is `None` and whose payments are empty are
    /// silently skipped.
    ///
    /// Phase 9b-1 added the profile write path.
    /// Phase 9b-2 added the payment write path.
    fn write_identity_dashpay_subset(
        tx: &rusqlite::Transaction,
        network: &str,
        id_cs: IdentityChangeSet,
    ) -> Result<(), SqlitePersistError> {
        use platform_wallet::wallet::dashpay::{PaymentDirection, PaymentStatus};

        let mut upsert_profile = tx.prepare_cached(
            "INSERT INTO dashpay_profiles
                (identity_id, network, display_name, bio, avatar_url, public_message, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())
             ON CONFLICT(identity_id, network) DO UPDATE SET
                display_name = excluded.display_name,
                bio = excluded.bio,
                avatar_url = excluded.avatar_url,
                public_message = excluded.public_message,
                updated_at = unixepoch()",
        )?;
        let mut update_avatar = tx.prepare_cached(
            "UPDATE dashpay_profiles
             SET avatar_bytes = ?1, updated_at = unixepoch()
             WHERE identity_id = ?2 AND network = ?3",
        )?;
        // `dashpay_payments` has `tx_id TEXT UNIQUE NOT NULL`. Upsert
        // on `tx_id` so status updates (Pending → Confirmed) land on
        // the same row without creating duplicates. `confirmed_at`
        // is stamped only when the status transition actually
        // reaches `confirmed`.
        let mut upsert_payment = tx.prepare_cached(
            "INSERT INTO dashpay_payments
                (tx_id, from_identity_id, to_identity_id, amount, memo, payment_type, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(tx_id) DO UPDATE SET
                amount = excluded.amount,
                memo = excluded.memo,
                payment_type = excluded.payment_type,
                status = excluded.status,
                confirmed_at = CASE WHEN excluded.status = 'confirmed'
                                    THEN unixepoch()
                                    ELSE confirmed_at END",
        )?;

        for (id, entry) in id_cs.identities {
            // Profile upsert.
            if let Some(profile) = entry.dashpay_profile {
                upsert_profile.execute(rusqlite::params![
                    id.to_buffer().to_vec(),
                    network,
                    profile.display_name,
                    profile.bio,
                    profile.avatar_url,
                    profile.public_message,
                ])?;
                if let Some(avatar_bytes) = profile.avatar_bytes {
                    update_avatar.execute(rusqlite::params![
                        avatar_bytes,
                        id.to_buffer().to_vec(),
                        network,
                    ])?;
                }
            }

            // Payment upserts. `direction` / `status` enums translate
            // to the string values the SQL schema expects.
            for (tx_id, payment) in entry.dashpay_payments {
                let (from_id, to_id) = match payment.direction {
                    PaymentDirection::Sent => (id, payment.counterparty_id),
                    PaymentDirection::Received => (payment.counterparty_id, id),
                };
                let payment_type = match payment.direction {
                    PaymentDirection::Sent => "sent",
                    PaymentDirection::Received => "received",
                };
                let status_str = match payment.status {
                    PaymentStatus::Pending => "pending",
                    PaymentStatus::Confirmed => "confirmed",
                    PaymentStatus::Failed => "failed",
                };
                upsert_payment.execute(rusqlite::params![
                    tx_id,
                    from_id.to_buffer().to_vec(),
                    to_id.to_buffer().to_vec(),
                    payment.amount_duffs as i64,
                    payment.memo,
                    payment_type,
                    status_str,
                ])?;
            }
        }
        // Identity removal and the wallet-level metadata
        // (`primary_identity`, `last_scanned_index`) are
        // backend-task-owned for now — log the drop for cross-checks.
        if !id_cs.removed.is_empty()
            || id_cs.primary_identity.is_some()
            || id_cs.last_scanned_index.is_some()
        {
            tracing::debug!(
                removed = id_cs.removed.len(),
                has_primary = id_cs.primary_identity.is_some(),
                has_last_scanned_index = id_cs.last_scanned_index.is_some(),
                "persister: dropping IdentityChangeSet wallet-level subsets (backend tasks own identity removal / primary tracking)"
            );
        }
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

    /// An IdentityChangeSet with a `dashpay_profile` lands in the
    /// `dashpay_profiles` table; the rest of the IdentityEntry is
    /// silently ignored (Phase 9b-1 scope).
    #[test]
    fn test_dashpay_profile_round_trip_via_changeset() {
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::identity::v0::IdentityV0;
        use dash_sdk::platform::Identifier;
        use platform_wallet::changeset::IdentityEntry;
        use platform_wallet::wallet::dashpay::DashPayProfile;
        use std::collections::BTreeMap;

        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db.clone());

        let identity_id = Identifier::from([7u8; 32]);
        let identity = Identity::V0(IdentityV0 {
            id: identity_id,
            public_keys: BTreeMap::new(),
            balance: 0,
            revision: 0,
        });
        let profile = DashPayProfile {
            display_name: Some("alice".into()),
            bio: Some("test bio".into()),
            avatar_url: Some("https://example.com/avatar.png".into()),
            avatar_bytes: Some(vec![1, 2, 3, 4]),
            public_message: Some("hello world".into()),
        };
        let mut id_cs = platform_wallet::changeset::IdentityChangeSet::default();
        id_cs.identities.insert(
            identity_id,
            IdentityEntry {
                identity,
                identity_index: 0,
                label: None,
                last_updated_balance_block_time: None,
                last_synced_keys_block_time: None,
                dpns_names: Vec::new(),
                top_ups: BTreeMap::new(),
                status: Default::default(),
                key_storage: BTreeMap::new(),
                wallet_seed_hash: Some(TEST_WALLET_ID),
                dashpay_profile: Some(profile.clone()),
                dashpay_payments: BTreeMap::new(),
            },
        );
        let cs = PlatformWalletChangeSet {
            identities: Some(id_cs),
            ..Default::default()
        };
        persister.store(TEST_WALLET_ID, cs);
        persister.flush(TEST_WALLET_ID).expect("flush");

        // Read back via the existing evo-tool helper to confirm the
        // persister wrote the right shape.
        let stored = db
            .load_dashpay_profile(&identity_id, "testnet")
            .expect("load dashpay profile")
            .expect("profile present");
        assert_eq!(stored.display_name.as_deref(), Some("alice"));
        assert_eq!(stored.bio.as_deref(), Some("test bio"));
        assert_eq!(
            stored.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
        assert_eq!(stored.public_message.as_deref(), Some("hello world"));
        assert_eq!(stored.avatar_bytes, Some(vec![1, 2, 3, 4]));
    }

    /// An IdentityChangeSet carrying a `dashpay_payments` entry lands
    /// in the `dashpay_payments` table; status updates on the same
    /// tx_id upsert in place (confirmed_at is stamped).
    #[test]
    fn test_dashpay_payment_round_trip_via_changeset() {
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::identity::v0::IdentityV0;
        use dash_sdk::platform::Identifier;
        use platform_wallet::changeset::IdentityEntry;
        use platform_wallet::wallet::dashpay::{PaymentEntry, PaymentStatus};
        use std::collections::BTreeMap;

        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db.clone());

        let owner_id = Identifier::from([7u8; 32]);
        let contact_id = Identifier::from([8u8; 32]);
        let owner_identity = Identity::V0(IdentityV0 {
            id: owner_id,
            public_keys: BTreeMap::new(),
            balance: 0,
            revision: 0,
        });
        let tx_id = "a".repeat(64);

        // Build an IdentityEntry with one pending payment.
        let build_entry = |status: PaymentStatus| {
            let mut pay = PaymentEntry::new_sent(contact_id, 12_000, Some("lunch".into()));
            pay.status = status;
            let mut payments = BTreeMap::new();
            payments.insert(tx_id.clone(), pay);
            IdentityEntry {
                identity: owner_identity.clone(),
                identity_index: 0,
                label: None,
                last_updated_balance_block_time: None,
                last_synced_keys_block_time: None,
                dpns_names: Vec::new(),
                top_ups: BTreeMap::new(),
                status: Default::default(),
                key_storage: BTreeMap::new(),
                wallet_seed_hash: Some(TEST_WALLET_ID),
                dashpay_profile: None,
                dashpay_payments: payments,
            }
        };

        // First flush: pending payment.
        let mut id_cs = platform_wallet::changeset::IdentityChangeSet::default();
        id_cs.identities.insert(owner_id, build_entry(PaymentStatus::Pending));
        persister.store(
            TEST_WALLET_ID,
            PlatformWalletChangeSet {
                identities: Some(id_cs),
                ..Default::default()
            },
        );
        persister.flush(TEST_WALLET_ID).expect("flush pending");

        // Read back via the existing evo-tool helper.
        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments");
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].tx_id, tx_id);
        assert_eq!(payments[0].amount, 12_000);
        assert_eq!(payments[0].memo.as_deref(), Some("lunch"));
        assert_eq!(payments[0].payment_type, "sent");
        assert_eq!(payments[0].status, "pending");
        assert!(payments[0].confirmed_at.is_none());

        // Second flush: same tx_id, status confirmed. Upsert should
        // land on the same row and stamp confirmed_at.
        let mut id_cs = platform_wallet::changeset::IdentityChangeSet::default();
        id_cs.identities.insert(owner_id, build_entry(PaymentStatus::Confirmed));
        persister.store(
            TEST_WALLET_ID,
            PlatformWalletChangeSet {
                identities: Some(id_cs),
                ..Default::default()
            },
        );
        persister.flush(TEST_WALLET_ID).expect("flush confirmed");

        let payments = db
            .load_payment_history(&owner_id, 10)
            .expect("load payments after confirm");
        assert_eq!(payments.len(), 1, "status upsert must not duplicate row");
        assert_eq!(payments[0].status, "confirmed");
        assert!(payments[0].confirmed_at.is_some());
    }

    /// A ContactChangeSet carrying an `established` entry with non-zero
    /// derivation state lands in the `dashpay_contact_address_indices`
    /// table. A subsequent lower bump must not regress
    /// `highest_receive_index` (MAX merge in the upsert).
    #[test]
    fn test_contact_derivation_round_trip_via_changeset() {
        use dash_sdk::platform::Identifier;
        use platform_wallet::changeset::ContactChangeSet;
        use platform_wallet::wallet::dashpay::{ContactRequest, EstablishedContact};
        use std::collections::BTreeMap;

        let db = Arc::new(create_test_database().expect("create test db"));
        let persister = make_persister(db.clone());

        let owner_id = Identifier::from([1u8; 32]);
        let contact_id = Identifier::from([2u8; 32]);
        let contact_request = || {
            ContactRequest::new(
                owner_id,
                contact_id,
                0,
                0,
                0,
                vec![0u8; 96],
                100_000,
                1_700_000_000,
            )
        };
        let build = |highest: u32, bloom: u32, next_send: u32| {
            let mut contact = EstablishedContact::new(
                contact_id,
                contact_request(),
                contact_request(),
            );
            contact.highest_receive_index = highest;
            contact.bloom_registered_count = bloom;
            contact.next_send_index = next_send;
            let mut map = BTreeMap::new();
            map.insert((owner_id, contact_id), contact);
            ContactChangeSet {
                established: map,
                ..Default::default()
            }
        };

        // First flush: highest=5, bloom=10, next_send=3.
        persister.store(
            TEST_WALLET_ID,
            PlatformWalletChangeSet {
                contacts: Some(build(5, 10, 3)),
                ..Default::default()
            },
        );
        persister.flush(TEST_WALLET_ID).expect("flush first");

        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices");
        assert_eq!(indices.highest_receive_index, 5);
        assert_eq!(indices.bloom_registered_count, 10);
        assert_eq!(indices.next_send_index, 3);

        // Second flush: lower highest (2) must NOT regress — MAX merge.
        // bloom_registered_count DOES get updated (last-write-wins).
        persister.store(
            TEST_WALLET_ID,
            PlatformWalletChangeSet {
                contacts: Some(build(2, 20, 5)),
                ..Default::default()
            },
        );
        persister.flush(TEST_WALLET_ID).expect("flush second");

        let indices = db
            .get_contact_address_indices(&owner_id, &contact_id)
            .expect("load indices after regression");
        assert_eq!(
            indices.highest_receive_index, 5,
            "highest_receive_index must not regress"
        );
        assert_eq!(indices.bloom_registered_count, 20);
        assert_eq!(indices.next_send_index, 5);
    }
}
