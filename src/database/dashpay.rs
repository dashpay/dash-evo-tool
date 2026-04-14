use dash_sdk::platform::Identifier;
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// DashPay profile data stored locally
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProfile {
    pub identity_id: Vec<u8>,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_hash: Option<Vec<u8>>,
    pub avatar_fingerprint: Option<Vec<u8>>,
    pub avatar_bytes: Option<Vec<u8>>,
    pub public_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// DashPay contact information stored locally
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredContact {
    pub owner_identity_id: Vec<u8>,
    pub contact_identity_id: Vec<u8>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub public_message: Option<String>,
    pub contact_status: String, // "pending", "accepted", "blocked"
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen: Option<i64>,
}

/// DIP-15 cryptographic material for a persisted contact request
/// (Item 7a/7b). Every field is `Option` so legacy call sites can
/// pass `Default::default()` and let the background contact sync
/// repopulate them later — the load path skips `EstablishedContact`
/// reconstruction for rows where any required field is `None`.
#[derive(Debug, Clone, Default)]
pub struct ContactRequestCryptoFields {
    pub sender_key_index: Option<u32>,
    pub recipient_key_index: Option<u32>,
    pub account_reference: Option<u32>,
    pub encrypted_public_key: Option<Vec<u8>>,
    pub encrypted_account_label_bytes: Option<Vec<u8>>,
    pub auto_accept_proof: Option<Vec<u8>>,
    pub core_height_created_at: Option<u32>,
}

/// DashPay contact request stored locally
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredContactRequest {
    pub id: i64,
    pub from_identity_id: Vec<u8>,
    pub to_identity_id: Vec<u8>,
    pub to_username: Option<String>,
    pub account_label: Option<String>,
    pub request_type: String, // "sent", "received"
    pub status: String,       // "pending", "accepted", "rejected", "expired"
    pub created_at: i64,
    pub responded_at: Option<i64>,
    pub expires_at: Option<i64>,
}

/// DashPay payment/transaction record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPayment {
    pub id: i64,
    pub tx_id: String,
    pub from_identity_id: Vec<u8>,
    pub to_identity_id: Vec<u8>,
    pub amount: i64, // in credits
    pub memo: Option<String>,
    pub payment_type: String, // "sent", "received"
    pub status: String,       // "pending", "confirmed", "failed"
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
}

impl crate::database::Database {
    /// Initialize all DashPay-related database tables using a transaction
    pub fn init_dashpay_tables_in_tx(&self, tx: &rusqlite::Connection) -> rusqlite::Result<()> {
        // Profiles table
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_profiles (
                identity_id BLOB NOT NULL,
                wallet_id BLOB,
                network TEXT NOT NULL,
                display_name TEXT,
                bio TEXT,
                avatar_url TEXT,
                avatar_hash BLOB,
                avatar_fingerprint BLOB,
                avatar_bytes BLOB,
                public_message TEXT,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch()),
                PRIMARY KEY (identity_id, network)
            )",
            [],
        )?;

        // Contacts table (extends the existing contact_private_info)
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contacts (
                owner_identity_id BLOB NOT NULL,
                contact_identity_id BLOB NOT NULL,
                wallet_id BLOB,
                network TEXT NOT NULL,
                username TEXT,
                display_name TEXT,
                avatar_url TEXT,
                public_message TEXT,
                contact_status TEXT DEFAULT 'pending',
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch()),
                last_seen INTEGER,
                PRIMARY KEY (owner_identity_id, contact_identity_id, network)
            )",
            [],
        )?;

        // Contact requests table. DIP-15 crypto columns
        // (sender_key_index, recipient_key_index, account_reference,
        // encrypted_public_key, encrypted_account_label_bytes,
        // auto_accept_proof, core_height_created_at) were added in v38
        // to support full `ContactRequest` reconstruction on wallet
        // open without re-fetching from platform (Item 7). They're
        // nullable so the migration runs without backfill — existing
        // rows stay NULL until the next background contact-request
        // sync repopulates them.
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contact_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
                wallet_id BLOB,
                network TEXT NOT NULL,
                to_username TEXT,
                account_label TEXT,
                request_type TEXT NOT NULL CHECK (request_type IN ('sent', 'received')),
                status TEXT DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'rejected', 'expired')),
                created_at INTEGER DEFAULT (unixepoch()),
                responded_at INTEGER,
                expires_at INTEGER,
                sender_key_index INTEGER,
                recipient_key_index INTEGER,
                account_reference INTEGER,
                encrypted_public_key BLOB,
                encrypted_account_label_bytes BLOB,
                auto_accept_proof BLOB,
                core_height_created_at INTEGER,
                platform_created_at_ms INTEGER
            )",
            [],
        )?;

        // Create index for faster queries
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_contact_requests_from
             ON dashpay_contact_requests(from_identity_id)",
            [],
        )?;

        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_contact_requests_to
             ON dashpay_contact_requests(to_identity_id)",
            [],
        )?;

        // Payments/transactions table
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_payments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_id TEXT UNIQUE NOT NULL,
                wallet_id BLOB,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
                amount INTEGER NOT NULL,
                memo TEXT,
                payment_type TEXT NOT NULL CHECK (payment_type IN ('sent', 'received')),
                status TEXT DEFAULT 'pending' CHECK (status IN ('pending', 'confirmed', 'failed')),
                created_at INTEGER DEFAULT (unixepoch()),
                confirmed_at INTEGER
            )",
            [],
        )?;

        // Create index for faster queries
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_payments_from
             ON dashpay_payments(from_identity_id)",
            [],
        )?;

        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_payments_to
             ON dashpay_payments(to_identity_id)",
            [],
        )?;

        // Contact address index tracking table.
        //
        // Phase 9b-3 rollback (current scope): this table now stores
        // only `next_send_index` — the send-side counter used by
        // `get_and_increment_send_index` when handing out unique
        // payment addresses to send to a contact. The receive-side
        // `highest_receive_index` and `bloom_registered_count`
        // columns are dead; they're still declared here so existing
        // databases keep working, but no code reads them anymore.
        // Phase 10 will migrate `next_send_index` to live on the
        // key-wallet's `DashpayExternalAccount` address pool
        // (`highest_generated + 1`) and drop this table entirely.
        //
        // Note: `dashpay_address_mappings` was dropped in v35.
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contact_address_indices (
                owner_identity_id BLOB NOT NULL,
                contact_identity_id BLOB NOT NULL,
                next_send_index INTEGER DEFAULT 0,
                highest_receive_index INTEGER DEFAULT 0,
                bloom_registered_count INTEGER DEFAULT 0,
                PRIMARY KEY (owner_identity_id, contact_identity_id)
            )",
            [],
        )?;

        Ok(())
    }

    // Profile operations

    pub fn save_dashpay_profile(
        &self,
        identity_id: &Identifier,
        network: &str,
        display_name: Option<&str>,
        bio: Option<&str>,
        avatar_url: Option<&str>,
        public_message: Option<&str>,
    ) -> rusqlite::Result<()> {
        // Use INSERT ... ON CONFLICT to preserve avatar_bytes when updating
        let sql = "
            INSERT INTO dashpay_profiles
            (identity_id, network, display_name, bio, avatar_url, public_message, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())
            ON CONFLICT(identity_id, network) DO UPDATE SET
                display_name = excluded.display_name,
                bio = excluded.bio,
                avatar_url = excluded.avatar_url,
                public_message = excluded.public_message,
                updated_at = unixepoch()
        ";

        let result = self.execute(
            sql,
            params![
                identity_id.to_buffer().to_vec(),
                network,
                display_name,
                bio,
                avatar_url,
                public_message,
            ],
        );

        result?;
        Ok(())
    }

    /// Save avatar bytes for a profile (called after fetching avatar from network)
    pub fn save_dashpay_profile_avatar_bytes(
        &self,
        identity_id: &Identifier,
        network: &str,
        avatar_bytes: Option<&[u8]>,
    ) -> rusqlite::Result<()> {
        let sql = "
            UPDATE dashpay_profiles
            SET avatar_bytes = ?1, updated_at = unixepoch()
            WHERE identity_id = ?2 AND network = ?3
        ";

        self.execute(
            sql,
            params![avatar_bytes, identity_id.to_buffer().to_vec(), network,],
        )?;
        Ok(())
    }

    pub fn load_dashpay_profile(
        &self,
        identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<Option<StoredProfile>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT identity_id, display_name, bio, avatar_url, avatar_hash,
                    avatar_fingerprint, avatar_bytes, public_message, created_at, updated_at
             FROM dashpay_profiles
             WHERE identity_id = ?1 AND network = ?2",
        )?;

        let result = stmt.query_row(params![identity_id.to_buffer().to_vec(), network], |row| {
            Ok(StoredProfile {
                identity_id: row.get(0)?,
                display_name: row.get(1)?,
                bio: row.get(2)?,
                avatar_url: row.get(3)?,
                avatar_hash: row.get(4)?,
                avatar_fingerprint: row.get(5)?,
                avatar_bytes: row.get(6)?,
                public_message: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        });

        match result {
            Ok(profile) => Ok(Some(profile)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    // Contact operations

    /// Superseded by changeset flush (Item 8.3). Kept for tests only.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn save_dashpay_contact(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        network: &str,
        username: Option<&str>,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
        public_message: Option<&str>,
        contact_status: &str,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT OR REPLACE INTO dashpay_contacts
            (owner_identity_id, contact_identity_id, network, username, display_name,
             avatar_url, public_message, contact_status, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, unixepoch())
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
                network,
                username,
                display_name,
                avatar_url,
                public_message,
                contact_status,
            ],
        )?;
        Ok(())
    }

    pub fn load_dashpay_contacts(
        &self,
        owner_identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<Vec<StoredContact>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT owner_identity_id, contact_identity_id, username, display_name,
                    avatar_url, public_message, contact_status, created_at, updated_at, last_seen
             FROM dashpay_contacts
             WHERE owner_identity_id = ?1 AND network = ?2
             ORDER BY updated_at DESC",
        )?;

        let contacts = stmt
            .query_map(
                params![owner_identity_id.to_buffer().to_vec(), network],
                |row| {
                    Ok(StoredContact {
                        owner_identity_id: row.get(0)?,
                        contact_identity_id: row.get(1)?,
                        username: row.get(2)?,
                        display_name: row.get(3)?,
                        avatar_url: row.get(4)?,
                        public_message: row.get(5)?,
                        contact_status: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                        last_seen: row.get(9)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(contacts)
    }

    pub fn update_contact_last_seen(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<()> {
        let sql = "
            UPDATE dashpay_contacts
            SET last_seen = unixepoch(), updated_at = unixepoch()
            WHERE owner_identity_id = ?1 AND contact_identity_id = ?2 AND network = ?3
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
                network,
            ],
        )?;
        Ok(())
    }

    /// Clear all contacts for a specific owner identity on a specific network
    pub fn clear_dashpay_contacts(
        &self,
        owner_identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<()> {
        let sql = "DELETE FROM dashpay_contacts WHERE owner_identity_id = ?1 AND network = ?2";

        self.execute(
            sql,
            params![owner_identity_id.to_buffer().to_vec(), network],
        )?;
        Ok(())
    }

    // Contact request operations

    /// Superseded by changeset flush (Item 8.2). Kept for tests only.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn save_contact_request(
        &self,
        from_identity_id: &Identifier,
        to_identity_id: &Identifier,
        network: &str,
        to_username: Option<&str>,
        account_label: Option<&str>,
        request_type: &str,
        crypto: ContactRequestCryptoFields,
        platform_created_at_ms: Option<i64>,
    ) -> rusqlite::Result<i64> {
        // Review S2: `dashpay_contact_requests` has an auto-increment
        // `id` PK and no UNIQUE constraint on `(from, to, network)`.
        // Every contact-request sync used to INSERT a fresh row, so
        // after N syncs the table held N duplicates per pair. Item 7c
        // then picked an arbitrary row via last-write-wins in its
        // HashMap insert. Fix: DELETE any existing row for the same
        // `(from, to, network)` before INSERT, wrapped in a
        // transaction so concurrent readers never see the gap.
        // This bounds the table size at one row per pair, matches
        // the intent of a "sync fresh state from platform" call,
        // and doesn't require a schema migration or dedupe of
        // existing rows (any pre-existing duplicates get collapsed
        // on the next save).
        let from_bytes = from_identity_id.to_buffer().to_vec();
        let to_bytes = to_identity_id.to_buffer().to_vec();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM dashpay_contact_requests
             WHERE from_identity_id = ?1 AND to_identity_id = ?2 AND network = ?3",
            params![&from_bytes, &to_bytes, network],
        )?;
        tx.execute(
            "INSERT INTO dashpay_contact_requests
                (from_identity_id, to_identity_id, network, to_username, account_label,
                 request_type, sender_key_index, recipient_key_index, account_reference,
                 encrypted_public_key, encrypted_account_label_bytes, auto_accept_proof,
                 core_height_created_at, platform_created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                &from_bytes,
                &to_bytes,
                network,
                to_username,
                account_label,
                request_type,
                crypto.sender_key_index,
                crypto.recipient_key_index,
                crypto.account_reference,
                crypto.encrypted_public_key,
                crypto.encrypted_account_label_bytes,
                crypto.auto_accept_proof,
                crypto.core_height_created_at,
                platform_created_at_ms,
            ],
        )?;
        let row_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(row_id)
    }

    pub fn update_contact_request_status(
        &self,
        request_id: i64,
        status: &str,
    ) -> rusqlite::Result<()> {
        let sql = "
            UPDATE dashpay_contact_requests
            SET status = ?1, responded_at = unixepoch()
            WHERE id = ?2
        ";

        self.execute(sql, params![status, request_id])?;
        Ok(())
    }

    /// Item 7c: Load full `ContactRequest` rows (metadata +
    /// DIP-15 crypto) for an identity, keyed by `(from, to)` pair.
    ///
    /// Returns rows where ALL required DIP-15 columns AND the
    /// `platform_created_at_ms` timestamp are non-NULL — rows with
    /// partial crypto (legacy rows or in-flight saves) are skipped.
    /// The caller joins outgoing+incoming pairs to reconstruct
    /// `EstablishedContact` objects.
    ///
    /// Covers both directions: requests where the identity is the
    /// sender (outgoing) and where it's the recipient (incoming).
    ///
    /// The returned `platform_created_at_ms` is in milliseconds
    /// (matching `ContactRequest.created_at: TimestampMillis`) —
    /// NOT the local-save `created_at` column, which is in seconds.
    /// See M2 in the Item 7 review for why these are separate.
    ///
    /// Superseded at runtime by the persister's `load()` flow which
    /// reconstructs `EstablishedContact` instances directly into the
    /// in-memory `ManagedIdentity.established_contacts` map. Kept
    /// only for tests that exercise the raw row-shape.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    pub fn load_contact_request_crypto_rows(
        &self,
        identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<
        Vec<(
            Identifier,      // from_identity_id
            Identifier,      // to_identity_id
            u32,             // sender_key_index
            u32,             // recipient_key_index
            u32,             // account_reference
            Vec<u8>,         // encrypted_public_key
            Option<Vec<u8>>, // encrypted_account_label_bytes
            Option<Vec<u8>>, // auto_accept_proof
            u32,             // core_height_created_at
            u64,             // platform_created_at_ms (TimestampMillis)
        )>,
    > {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT from_identity_id, to_identity_id,
                    sender_key_index, recipient_key_index, account_reference,
                    encrypted_public_key, encrypted_account_label_bytes,
                    auto_accept_proof, core_height_created_at, platform_created_at_ms
             FROM dashpay_contact_requests
             WHERE (from_identity_id = ?1 OR to_identity_id = ?1)
               AND network = ?2
               AND status = 'accepted'
               AND sender_key_index IS NOT NULL
               AND recipient_key_index IS NOT NULL
               AND account_reference IS NOT NULL
               AND encrypted_public_key IS NOT NULL
               AND core_height_created_at IS NOT NULL
               AND platform_created_at_ms IS NOT NULL",
        )?;

        let id_bytes = identity_id.to_buffer().to_vec();
        let rows = stmt
            .query_map(params![id_bytes, network], |row| {
                let from_bytes: Vec<u8> = row.get(0)?;
                let to_bytes: Vec<u8> = row.get(1)?;
                let sender_key_index: u32 = row.get(2)?;
                let recipient_key_index: u32 = row.get(3)?;
                let account_reference: u32 = row.get(4)?;
                let encrypted_public_key: Vec<u8> = row.get(5)?;
                let encrypted_account_label_bytes: Option<Vec<u8>> = row.get(6)?;
                let auto_accept_proof: Option<Vec<u8>> = row.get(7)?;
                let core_height_created_at: u32 = row.get(8)?;
                let platform_created_at_ms: i64 = row.get(9)?;

                let from_id = Identifier::from_bytes(&from_bytes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let to_id = Identifier::from_bytes(&to_bytes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok((
                    from_id,
                    to_id,
                    sender_key_index,
                    recipient_key_index,
                    account_reference,
                    encrypted_public_key,
                    encrypted_account_label_bytes,
                    auto_accept_proof,
                    core_height_created_at,
                    platform_created_at_ms.max(0) as u64,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn load_pending_contact_requests(
        &self,
        identity_id: &Identifier,
        network: &str,
        request_type: &str,
    ) -> rusqlite::Result<Vec<StoredContactRequest>> {
        let conn = self.conn.lock().unwrap();
        let sql = if request_type == "sent" {
            "SELECT id, from_identity_id, to_identity_id, to_username, account_label,
                    request_type, status, created_at, responded_at, expires_at
             FROM dashpay_contact_requests
             WHERE from_identity_id = ?1 AND network = ?2 AND request_type = 'sent' AND status = 'pending'
             ORDER BY created_at DESC"
        } else {
            "SELECT id, from_identity_id, to_identity_id, to_username, account_label,
                    request_type, status, created_at, responded_at, expires_at
             FROM dashpay_contact_requests
             WHERE to_identity_id = ?1 AND network = ?2 AND request_type = 'received' AND status = 'pending'
             ORDER BY created_at DESC"
        };

        let mut stmt = conn.prepare(sql)?;
        let requests = stmt
            .query_map(params![identity_id.to_buffer().to_vec(), network], |row| {
                Ok(StoredContactRequest {
                    id: row.get(0)?,
                    from_identity_id: row.get(1)?,
                    to_identity_id: row.get(2)?,
                    to_username: row.get(3)?,
                    account_label: row.get(4)?,
                    request_type: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                    responded_at: row.get(8)?,
                    expires_at: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(requests)
    }

    // Payment operations

    pub fn save_payment(
        &self,
        tx_id: &str,
        from_identity_id: &Identifier,
        to_identity_id: &Identifier,
        amount: i64,
        memo: Option<&str>,
        payment_type: &str,
    ) -> rusqlite::Result<i64> {
        let sql = "
            INSERT INTO dashpay_payments
            (tx_id, from_identity_id, to_identity_id, amount, memo, payment_type)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ";

        let conn = self.conn.lock().unwrap();
        conn.execute(
            sql,
            params![
                tx_id,
                from_identity_id.to_buffer().to_vec(),
                to_identity_id.to_buffer().to_vec(),
                amount,
                memo,
                payment_type,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    pub fn update_payment_status(&self, payment_id: i64, status: &str) -> rusqlite::Result<()> {
        let sql = if status == "confirmed" {
            "UPDATE dashpay_payments
             SET status = ?1, confirmed_at = unixepoch()
             WHERE id = ?2"
        } else {
            "UPDATE dashpay_payments
             SET status = ?1
             WHERE id = ?2"
        };

        self.execute(sql, params![status, payment_id])?;
        Ok(())
    }

    pub fn load_payment_history(
        &self,
        identity_id: &Identifier,
        limit: u32,
    ) -> rusqlite::Result<Vec<StoredPayment>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, tx_id, from_identity_id, to_identity_id, amount, memo,
                    payment_type, status, created_at, confirmed_at
             FROM dashpay_payments
             WHERE from_identity_id = ?1 OR to_identity_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;

        let identity_bytes = identity_id.to_buffer().to_vec();
        let payments = stmt
            .query_map(params![identity_bytes, limit], |row| {
                Ok(StoredPayment {
                    id: row.get(0)?,
                    tx_id: row.get(1)?,
                    from_identity_id: row.get(2)?,
                    to_identity_id: row.get(3)?,
                    amount: row.get(4)?,
                    memo: row.get(5)?,
                    payment_type: row.get(6)?,
                    status: row.get(7)?,
                    created_at: row.get(8)?,
                    confirmed_at: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(payments)
    }

    /// Delete all DashPay data for a specific identity
    pub fn delete_dashpay_data_for_identity(
        &self,
        identity_id: &Identifier,
    ) -> rusqlite::Result<()> {
        let identity_bytes = identity_id.to_buffer().to_vec();

        // Delete profile
        self.execute(
            "DELETE FROM dashpay_profiles WHERE identity_id = ?1",
            params![&identity_bytes],
        )?;

        // Delete contacts
        self.execute(
            "DELETE FROM dashpay_contacts WHERE owner_identity_id = ?1",
            params![&identity_bytes],
        )?;

        // Delete contact requests
        self.execute(
            "DELETE FROM dashpay_contact_requests
             WHERE from_identity_id = ?1 OR to_identity_id = ?1",
            params![&identity_bytes],
        )?;

        // Delete payments
        self.execute(
            "DELETE FROM dashpay_payments
             WHERE from_identity_id = ?1 OR to_identity_id = ?1",
            params![&identity_bytes],
        )?;

        // Delete contact address indices
        self.execute(
            "DELETE FROM dashpay_contact_address_indices WHERE owner_identity_id = ?1",
            params![&identity_bytes],
        )?;

        Ok(())
    }

    // Contact address index operations (DIP-0015)

    /// Get the next send address index for a contact and increment it atomically.
    /// This is used when sending a payment to ensure unique addresses.
    /// Uses an atomic INSERT/UPDATE with RETURNING to prevent race conditions.
    pub fn get_and_increment_send_index(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
    ) -> rusqlite::Result<u32> {
        let conn = self.conn.lock().unwrap();

        // First, ensure the row exists with default values if it doesn't
        let init_sql = "
            INSERT OR IGNORE INTO dashpay_contact_address_indices
            (owner_identity_id, contact_identity_id, next_send_index, highest_receive_index)
            VALUES (?1, ?2, 0, 0)
        ";
        conn.execute(
            init_sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
            ],
        )?;

        // Now atomically increment and return the old value
        // We update next_send_index = next_send_index + 1 and return the old value
        let update_sql = "
            UPDATE dashpay_contact_address_indices
            SET next_send_index = next_send_index + 1
            WHERE owner_identity_id = ?1 AND contact_identity_id = ?2
            RETURNING next_send_index - 1
        ";

        conn.query_row(
            update_sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
            ],
            |row| row.get(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;

    /// Item 7c: full DIP-15 crypto round-trip for contact requests.
    /// Saves one outgoing + one incoming request with full crypto,
    /// then loads both back and verifies all 7 cryptographic fields
    /// match byte-for-byte.
    #[test]
    fn test_contact_request_dip15_crypto_round_trip() {
        let db = create_test_database().expect("create test db");

        let owner = Identifier::from([0x11u8; 32]);
        let contact = Identifier::from([0x22u8; 32]);
        let network = "testnet";

        // Update status to 'accepted' after insert so the load path
        // filter picks them up.
        let accept = |req_id: i64| {
            db.execute(
                "UPDATE dashpay_contact_requests SET status = 'accepted' WHERE id = ?1",
                params![req_id],
            )
            .unwrap();
        };

        // Outgoing: owner → contact
        let outgoing_crypto = ContactRequestCryptoFields {
            sender_key_index: Some(1),
            recipient_key_index: Some(2),
            account_reference: Some(42),
            encrypted_public_key: Some(vec![0xa0u8; 96]),
            encrypted_account_label_bytes: Some(vec![0xb0u8, 0xb1, 0xb2]),
            auto_accept_proof: None,
            core_height_created_at: Some(100_000),
        };
        let outgoing_id = db
            .save_contact_request(
                &owner,
                &contact,
                network,
                None,
                None,
                "sent",
                outgoing_crypto.clone(),
                Some(1_700_000_000_000),
            )
            .expect("save outgoing");
        accept(outgoing_id);

        // Incoming: contact → owner
        let incoming_crypto = ContactRequestCryptoFields {
            sender_key_index: Some(3),
            recipient_key_index: Some(4),
            account_reference: Some(7),
            encrypted_public_key: Some(vec![0xc0u8; 96]),
            encrypted_account_label_bytes: None,
            auto_accept_proof: Some(vec![0xd0u8; 32]),
            core_height_created_at: Some(100_005),
        };
        let incoming_id = db
            .save_contact_request(
                &contact,
                &owner,
                network,
                None,
                None,
                "received",
                incoming_crypto.clone(),
                Some(1_700_000_001_000),
            )
            .expect("save incoming");
        accept(incoming_id);

        // Load — should return both rows (outgoing + incoming) for
        // the owner, since the query's WHERE clause picks up either
        // direction keyed on the identity.
        let rows = db
            .load_contact_request_crypto_rows(&owner, network)
            .expect("load rows");
        assert_eq!(rows.len(), 2, "both directions must load");

        // Split by direction from the owner's perspective.
        let (loaded_outgoing, loaded_incoming) = if rows[0].0 == owner {
            (&rows[0], &rows[1])
        } else {
            (&rows[1], &rows[0])
        };

        // Outgoing assertions (owner → contact)
        assert_eq!(loaded_outgoing.0, owner);
        assert_eq!(loaded_outgoing.1, contact);
        assert_eq!(loaded_outgoing.2, 1); // sender_key_index
        assert_eq!(loaded_outgoing.3, 2); // recipient_key_index
        assert_eq!(loaded_outgoing.4, 42); // account_reference
        assert_eq!(loaded_outgoing.5, vec![0xa0u8; 96]); // encrypted_public_key
        assert_eq!(loaded_outgoing.6, Some(vec![0xb0u8, 0xb1, 0xb2]));
        assert_eq!(loaded_outgoing.7, None); // auto_accept_proof
        assert_eq!(loaded_outgoing.8, 100_000); // core_height_created_at
        assert_eq!(loaded_outgoing.9, 1_700_000_000_000); // platform_created_at_ms

        // Incoming assertions (contact → owner)
        assert_eq!(loaded_incoming.0, contact);
        assert_eq!(loaded_incoming.1, owner);
        assert_eq!(loaded_incoming.2, 3);
        assert_eq!(loaded_incoming.3, 4);
        assert_eq!(loaded_incoming.4, 7);
        assert_eq!(loaded_incoming.5, vec![0xc0u8; 96]);
        assert_eq!(loaded_incoming.6, None);
        assert_eq!(loaded_incoming.7, Some(vec![0xd0u8; 32]));
        assert_eq!(loaded_incoming.8, 100_005);
        assert_eq!(loaded_incoming.9, 1_700_000_001_000);
    }

    /// Item 7c: legacy rows (all-None crypto) must be filtered out
    /// by `load_contact_request_crypto_rows`. The load path in
    /// `sync_identity_to_platform_wallet` relies on this filter so
    /// incomplete rows don't cause `ContactRequest::new` panics.
    #[test]
    fn test_contact_request_legacy_rows_filtered() {
        let db = create_test_database().expect("create test db");

        let owner = Identifier::from([0x33u8; 32]);
        let contact = Identifier::from([0x44u8; 32]);
        let network = "testnet";

        // Save a legacy-style row with no crypto fields.
        let id = db
            .save_contact_request(
                &owner,
                &contact,
                network,
                None,
                None,
                "sent",
                ContactRequestCryptoFields::default(),
                None,
            )
            .expect("save legacy");
        db.execute(
            "UPDATE dashpay_contact_requests SET status = 'accepted' WHERE id = ?1",
            params![id],
        )
        .unwrap();

        let rows = db
            .load_contact_request_crypto_rows(&owner, network)
            .expect("load rows");
        assert!(
            rows.is_empty(),
            "legacy rows with NULL crypto must be filtered out"
        );
    }
}
