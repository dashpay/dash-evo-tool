use crate::model::wallet::WalletSeedHash;
use dash_sdk::platform::Identifier;
use rusqlite::{OptionalExtension, params};
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
    pub output_index: i64,
    pub from_identity_id: Vec<u8>,
    pub to_identity_id: Vec<u8>,
    pub amount: i64, // in credits
    pub memo: Option<String>,
    pub payment_type: String, // "sent", "received"
    pub status: String,       // "pending", "confirmed", "failed"
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
}

/// DashPay contact address index tracking per DIP-0015
/// Tracks address indices used for sending/receiving payments per contact relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactAddressIndex {
    pub owner_identity_id: Vec<u8>,
    pub contact_identity_id: Vec<u8>,
    /// Next address index to use when sending TO this contact
    pub next_send_index: u32,
    /// Highest address index seen when receiving FROM this contact (for bloom filter)
    pub highest_receive_index: u32,
    /// Number of addresses registered in bloom filter for this contact
    pub bloom_registered_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentSaveResult {
    pub row_id: Option<i64>,
    pub inserted: bool,
    pub updated_existing: bool,
}

impl PaymentSaveResult {
    pub fn changed(&self) -> bool {
        self.inserted || self.updated_existing
    }
}

impl crate::database::Database {
    /// Initialize all DashPay-related database tables using a transaction
    pub fn init_dashpay_tables_in_tx(&self, tx: &rusqlite::Connection) -> rusqlite::Result<()> {
        // Profiles table
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_profiles (
                identity_id BLOB NOT NULL,
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

        // Contact requests table
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contact_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
                network TEXT NOT NULL,
                to_username TEXT,
                account_label TEXT,
                request_type TEXT NOT NULL CHECK (request_type IN ('sent', 'received')),
                status TEXT DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'rejected', 'expired')),
                created_at INTEGER DEFAULT (unixepoch()),
                responded_at INTEGER,
                expires_at INTEGER
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
                tx_id TEXT NOT NULL,
                output_index INTEGER NOT NULL DEFAULT -1,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
                amount INTEGER NOT NULL,
                memo TEXT,
                payment_type TEXT NOT NULL CHECK (payment_type IN ('sent', 'received')),
                status TEXT DEFAULT 'pending' CHECK (status IN ('pending', 'confirmed', 'failed')),
                created_at INTEGER DEFAULT (unixepoch()),
                confirmed_at INTEGER,
                UNIQUE (tx_id, from_identity_id, to_identity_id, output_index)
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

        // Contact address index tracking table (DIP-0015)
        // Tracks address indices per contact for payment derivation
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

        // DashPay address mappings for incoming payment detection
        // Maps addresses to contact relationships for transaction matching
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_address_mappings (
                address TEXT PRIMARY KEY,
                seed_hash BLOB,
                owner_identity_id BLOB NOT NULL,
                contact_identity_id BLOB NOT NULL,
                address_index INTEGER NOT NULL,
                created_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_dashpay_address_mappings_owner
             ON dashpay_address_mappings(owner_identity_id)",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_dashpay_address_mappings_contact
             ON dashpay_address_mappings(owner_identity_id, contact_identity_id)",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_dashpay_address_mappings_owner_seed_contact
             ON dashpay_address_mappings(owner_identity_id, seed_hash, contact_identity_id, address_index)",
            [],
        )?;

        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_wallet_tx_scan_markers (
                seed_hash BLOB NOT NULL,
                identity_id BLOB NOT NULL,
                network TEXT NOT NULL,
                completed_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (seed_hash, identity_id, network)
            )",
            [],
        )?;
        tx.execute(
            "CREATE INDEX IF NOT EXISTS idx_dashpay_wallet_tx_scan_markers_identity
             ON dashpay_wallet_tx_scan_markers(identity_id, network)",
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
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let previous_status: Option<String> = tx
            .query_row(
                "SELECT contact_status
             FROM dashpay_contacts
             WHERE owner_identity_id = ?1 AND contact_identity_id = ?2 AND network = ?3",
                params![
                    owner_identity_id.to_buffer().to_vec(),
                    contact_identity_id.to_buffer().to_vec(),
                    network,
                ],
                |row| row.get(0),
            )
            .optional()?;

        tx.execute(
            "INSERT OR REPLACE INTO dashpay_contacts
             (owner_identity_id, contact_identity_id, network, username, display_name,
              avatar_url, public_message, contact_status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, unixepoch())",
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

        let should_clear_scan_markers =
            contact_status == "accepted" && previous_status.as_deref() != Some("accepted");
        if should_clear_scan_markers {
            tx.execute(
                "DELETE FROM dashpay_wallet_tx_scan_markers
                 WHERE identity_id = ?1 AND network = ?2",
                params![owner_identity_id.to_buffer().to_vec(), network],
            )?;
        }

        tx.commit()?;
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

    pub fn save_contact_request(
        &self,
        from_identity_id: &Identifier,
        to_identity_id: &Identifier,
        network: &str,
        to_username: Option<&str>,
        account_label: Option<&str>,
        request_type: &str,
    ) -> rusqlite::Result<i64> {
        let sql = "
            INSERT INTO dashpay_contact_requests
            (from_identity_id, to_identity_id, network, to_username, account_label, request_type)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ";

        let conn = self.conn.lock().unwrap();
        conn.execute(
            sql,
            params![
                from_identity_id.to_buffer().to_vec(),
                to_identity_id.to_buffer().to_vec(),
                network,
                to_username,
                account_label,
                request_type,
            ],
        )?;

        Ok(conn.last_insert_rowid())
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

    /// Save a payment record.
    ///
    /// `created_at` is an optional Unix timestamp. Pass `None` for real-time
    /// events (the current time is used via `unixepoch()`). Pass `Some(ts)`
    /// when backfilling from an existing wallet transaction, so the payment
    /// row reflects the block time rather than the moment it was scanned.
    #[allow(clippy::too_many_arguments)]
    pub fn save_payment(
        &self,
        tx_id: &str,
        from_identity_id: &Identifier,
        to_identity_id: &Identifier,
        amount: i64,
        memo: Option<&str>,
        payment_type: &str,
        created_at: Option<i64>,
    ) -> rusqlite::Result<i64> {
        let result = self.save_payment_with_output_index(
            tx_id,
            None,
            from_identity_id,
            to_identity_id,
            amount,
            memo,
            payment_type,
            created_at,
        )?;

        Ok(if result.inserted {
            result.row_id.unwrap_or(0)
        } else {
            0
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_payment_with_output_index(
        &self,
        tx_id: &str,
        output_index: Option<u32>,
        from_identity_id: &Identifier,
        to_identity_id: &Identifier,
        amount: i64,
        memo: Option<&str>,
        payment_type: &str,
        created_at: Option<i64>,
    ) -> rusqlite::Result<PaymentSaveResult> {
        let conn = self.conn.lock().unwrap();
        let from_identity_bytes = from_identity_id.to_buffer().to_vec();
        let to_identity_bytes = to_identity_id.to_buffer().to_vec();

        let updated_existing = if let Some(output_index) = output_index {
            conn.execute(
                "UPDATE OR IGNORE dashpay_payments
                 SET output_index = ?1
                 WHERE tx_id = ?2
                   AND from_identity_id = ?3
                   AND to_identity_id = ?4
                   AND output_index = -1",
                params![
                    i64::from(output_index),
                    tx_id,
                    &from_identity_bytes,
                    &to_identity_bytes,
                ],
            )? > 0
        } else {
            false
        };

        if updated_existing {
            return Ok(PaymentSaveResult {
                row_id: None,
                inserted: false,
                updated_existing: true,
            });
        }

        let inserted = conn.execute(
            "
            INSERT OR IGNORE INTO dashpay_payments
            (tx_id, output_index, from_identity_id, to_identity_id, amount, memo, payment_type, created_at)
            VALUES (?1, COALESCE(?2, -1), ?3, ?4, ?5, ?6, ?7, COALESCE(?8, unixepoch()))
            ",
            params![
                tx_id,
                output_index.map(i64::from),
                &from_identity_bytes,
                &to_identity_bytes,
                amount,
                memo,
                payment_type,
                created_at,
            ],
        )? > 0;

        Ok(PaymentSaveResult {
            row_id: inserted.then(|| conn.last_insert_rowid()),
            inserted,
            updated_existing: false,
        })
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
            "SELECT id, tx_id, output_index, from_identity_id, to_identity_id, amount, memo,
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
                    output_index: row.get(2)?,
                    from_identity_id: row.get(3)?,
                    to_identity_id: row.get(4)?,
                    amount: row.get(5)?,
                    memo: row.get(6)?,
                    payment_type: row.get(7)?,
                    status: row.get(8)?,
                    created_at: row.get(9)?,
                    confirmed_at: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(payments)
    }

    /// Load payment history filtered to a specific contact relationship.
    /// Returns payments where both identity_id and contact_id are involved
    /// (either as sender or receiver), ordered by most recent first.
    pub fn load_payment_history_for_contact(
        &self,
        identity_id: &Identifier,
        contact_id: &Identifier,
        limit: u32,
    ) -> rusqlite::Result<Vec<StoredPayment>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, tx_id, output_index, from_identity_id, to_identity_id, amount, memo,
                    payment_type, status, created_at, confirmed_at
             FROM dashpay_payments
             WHERE (from_identity_id = ?1 OR to_identity_id = ?1)
               AND (from_identity_id = ?2 OR to_identity_id = ?2)
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;

        let identity_bytes = identity_id.to_buffer().to_vec();
        let contact_bytes = contact_id.to_buffer().to_vec();
        let payments = stmt
            .query_map(params![identity_bytes, contact_bytes, limit], |row| {
                Ok(StoredPayment {
                    id: row.get(0)?,
                    tx_id: row.get(1)?,
                    output_index: row.get(2)?,
                    from_identity_id: row.get(3)?,
                    to_identity_id: row.get(4)?,
                    amount: row.get(5)?,
                    memo: row.get(6)?,
                    payment_type: row.get(7)?,
                    status: row.get(8)?,
                    created_at: row.get(9)?,
                    confirmed_at: row.get(10)?,
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

    /// Get or create contact address index entry
    /// Returns (next_send_index, highest_receive_index, bloom_registered_count)
    pub fn get_contact_address_indices(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
    ) -> rusqlite::Result<ContactAddressIndex> {
        let conn = self.conn.lock().unwrap();

        // Try to get existing entry
        let mut stmt = conn.prepare(
            "SELECT owner_identity_id, contact_identity_id, next_send_index,
                    highest_receive_index, bloom_registered_count
             FROM dashpay_contact_address_indices
             WHERE owner_identity_id = ?1 AND contact_identity_id = ?2",
        )?;

        let result = stmt.query_row(
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec()
            ],
            |row| {
                Ok(ContactAddressIndex {
                    owner_identity_id: row.get(0)?,
                    contact_identity_id: row.get(1)?,
                    next_send_index: row.get(2)?,
                    highest_receive_index: row.get(3)?,
                    bloom_registered_count: row.get(4)?,
                })
            },
        );

        match result {
            Ok(indices) => Ok(indices),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Create new entry with defaults
                Ok(ContactAddressIndex {
                    owner_identity_id: owner_identity_id.to_buffer().to_vec(),
                    contact_identity_id: contact_identity_id.to_buffer().to_vec(),
                    next_send_index: 0,
                    highest_receive_index: 0,
                    bloom_registered_count: 0,
                })
            }
            Err(e) => Err(e),
        }
    }

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

    /// Update the highest receive index seen for a contact
    /// Called when we detect an incoming payment at a higher index
    pub fn update_highest_receive_index(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        index: u32,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT INTO dashpay_contact_address_indices
            (owner_identity_id, contact_identity_id, highest_receive_index)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(owner_identity_id, contact_identity_id)
            DO UPDATE SET highest_receive_index = MAX(highest_receive_index, ?3)
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
                index,
            ],
        )?;

        Ok(())
    }

    /// Update the bloom registered count for a contact
    /// Called after registering addresses in bloom filter
    pub fn update_bloom_registered_count(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        count: u32,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT INTO dashpay_contact_address_indices
            (owner_identity_id, contact_identity_id, bloom_registered_count)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(owner_identity_id, contact_identity_id)
            DO UPDATE SET bloom_registered_count = ?3
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
                count,
            ],
        )?;

        Ok(())
    }

    /// Get all contact address indices for an identity
    /// Useful for registering bloom filters on startup
    pub fn get_all_contact_address_indices(
        &self,
        owner_identity_id: &Identifier,
    ) -> rusqlite::Result<Vec<ContactAddressIndex>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT owner_identity_id, contact_identity_id, next_send_index,
                    highest_receive_index, bloom_registered_count
             FROM dashpay_contact_address_indices
             WHERE owner_identity_id = ?1",
        )?;

        let indices = stmt
            .query_map(params![owner_identity_id.to_buffer().to_vec()], |row| {
                Ok(ContactAddressIndex {
                    owner_identity_id: row.get(0)?,
                    contact_identity_id: row.get(1)?,
                    next_send_index: row.get(2)?,
                    highest_receive_index: row.get(3)?,
                    bloom_registered_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(indices)
    }

    // DashPay address mapping operations

    /// Save a DashPay address mapping for incoming payment detection
    pub fn save_dashpay_address_mapping(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        seed_hash: &WalletSeedHash,
        address: &dash_sdk::dpp::dashcore::Address,
        address_index: u32,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT OR REPLACE INTO dashpay_address_mappings
            (address, seed_hash, owner_identity_id, contact_identity_id, address_index, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
        ";

        self.execute(
            sql,
            params![
                address.to_string(),
                seed_hash.as_slice(),
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
                address_index,
            ],
        )?;

        Ok(())
    }

    /// Look up a DashPay address mapping to find which contact relationship it belongs to
    /// Returns (owner_identity_id, contact_identity_id, address_index) if found
    pub fn get_dashpay_address_mapping(
        &self,
        address: &dash_sdk::dpp::dashcore::Address,
    ) -> rusqlite::Result<Option<(Identifier, Identifier, u32)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT owner_identity_id, contact_identity_id, address_index
             FROM dashpay_address_mappings
             WHERE address = ?1",
        )?;

        let result = stmt.query_row(params![address.to_string()], |row| {
            let owner_bytes: Vec<u8> = row.get(0)?;
            let contact_bytes: Vec<u8> = row.get(1)?;
            let address_index: u32 = row.get(2)?;
            Ok((owner_bytes, contact_bytes, address_index))
        });

        match result {
            Ok((owner_bytes, contact_bytes, address_index)) => {
                let owner_id = Identifier::from_bytes(&owner_bytes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let contact_id = Identifier::from_bytes(&contact_bytes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                Ok(Some((owner_id, contact_id, address_index)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Get all DashPay address mappings for an identity
    pub fn get_all_dashpay_address_mappings(
        &self,
        owner_identity_id: &Identifier,
    ) -> rusqlite::Result<Vec<(String, Identifier, u32)>> {
        use rusqlite::types::Type;

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT address, contact_identity_id, address_index
             FROM dashpay_address_mappings
             WHERE owner_identity_id = ?1
             ORDER BY contact_identity_id, address_index",
        )?;

        let mappings = stmt
            .query_map(params![owner_identity_id.to_buffer().to_vec()], |row| {
                let address: String = row.get(0)?;
                let contact_bytes: Vec<u8> = row.get(1)?;
                let address_index: u32 = row.get(2)?;
                Ok((address, contact_bytes, address_index))
            })?
            .map(|row_result| {
                row_result.and_then(|(address, contact_bytes, address_index)| {
                    let contact_id = Identifier::from_bytes(&contact_bytes).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(1, Type::Blob, Box::new(e))
                    })?;
                    Ok((address, contact_id, address_index))
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(mappings)
    }

    /// Get all DashPay address mappings for an identity scoped to a wallet seed.
    pub fn get_dashpay_address_mappings_for_wallet(
        &self,
        owner_identity_id: &Identifier,
        seed_hash: &WalletSeedHash,
    ) -> rusqlite::Result<Vec<(String, Identifier, u32)>> {
        use rusqlite::types::Type;

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT address, contact_identity_id, address_index
             FROM dashpay_address_mappings
             WHERE owner_identity_id = ?1 AND seed_hash = ?2
             ORDER BY contact_identity_id, address_index",
        )?;

        let mappings = stmt
            .query_map(
                params![owner_identity_id.to_buffer().to_vec(), seed_hash.as_slice()],
                |row| {
                    let address: String = row.get(0)?;
                    let contact_bytes: Vec<u8> = row.get(1)?;
                    let address_index: u32 = row.get(2)?;
                    Ok((address, contact_bytes, address_index))
                },
            )?
            .map(|row_result| {
                row_result.and_then(|(address, contact_bytes, address_index)| {
                    let contact_id = Identifier::from_bytes(&contact_bytes).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(1, Type::Blob, Box::new(e))
                    })?;
                    Ok((address, contact_id, address_index))
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(mappings)
    }

    /// Delete all address mappings for a contact relationship
    pub fn delete_dashpay_address_mappings_for_contact(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
    ) -> rusqlite::Result<()> {
        self.execute(
            "DELETE FROM dashpay_address_mappings
             WHERE owner_identity_id = ?1 AND contact_identity_id = ?2",
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
            ],
        )?;
        Ok(())
    }

    pub fn has_dashpay_wallet_tx_scan_marker(
        &self,
        seed_hash: &[u8; 32],
        identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM dashpay_wallet_tx_scan_markers
                WHERE seed_hash = ?1 AND identity_id = ?2 AND network = ?3
            )",
            params![
                seed_hash.as_slice(),
                identity_id.to_buffer().to_vec(),
                network
            ],
            |row| row.get(0),
        )
    }

    pub fn mark_dashpay_wallet_tx_scan_complete(
        &self,
        seed_hash: &[u8; 32],
        identity_id: &Identifier,
        network: &str,
    ) -> rusqlite::Result<()> {
        self.execute(
            "INSERT INTO dashpay_wallet_tx_scan_markers (seed_hash, identity_id, network, completed_at)
             VALUES (?1, ?2, ?3, unixepoch())
             ON CONFLICT(seed_hash, identity_id, network) DO UPDATE SET completed_at = unixepoch()",
            params![seed_hash.as_slice(), identity_id.to_buffer().to_vec(), network],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;

    #[test]
    fn save_payment_with_output_index_upgrades_legacy_row_in_place() {
        let db = create_test_database().expect("test db");
        let from_identity_id = Identifier::random();
        let to_identity_id = Identifier::random();

        let seeded = db
            .save_payment_with_output_index(
                "txid_upgrade",
                None,
                &from_identity_id,
                &to_identity_id,
                42,
                Some("memo"),
                "received",
                Some(1_700_000_000),
            )
            .expect("seed legacy row");
        assert!(seeded.inserted);
        assert!(!seeded.updated_existing);

        let upgraded = db
            .save_payment_with_output_index(
                "txid_upgrade",
                Some(3),
                &from_identity_id,
                &to_identity_id,
                999,
                None,
                "received",
                Some(1_700_000_100),
            )
            .expect("upgrade row");
        assert!(!upgraded.inserted);
        assert!(upgraded.updated_existing);

        let duplicate = db
            .save_payment_with_output_index(
                "txid_upgrade",
                Some(3),
                &from_identity_id,
                &to_identity_id,
                999,
                None,
                "received",
                Some(1_700_000_200),
            )
            .expect("duplicate row");
        assert!(!duplicate.changed());

        let payments = db
            .load_payment_history(&to_identity_id, 10)
            .expect("load payments");
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].output_index, 3);
        assert_eq!(payments[0].amount, 42);
        assert_eq!(payments[0].memo.as_deref(), Some("memo"));
        assert_eq!(payments[0].created_at, 1_700_000_000);
    }

    #[test]
    fn dashpay_wallet_tx_scan_marker_round_trip_is_network_scoped() {
        let db = create_test_database().expect("test db");
        let seed_hash = [0xAB; 32];
        let identity_id = Identifier::random();

        assert!(
            !db.has_dashpay_wallet_tx_scan_marker(&seed_hash, &identity_id, "testnet")
                .expect("marker absent initially")
        );

        db.mark_dashpay_wallet_tx_scan_complete(&seed_hash, &identity_id, "testnet")
            .expect("mark complete");

        assert!(
            db.has_dashpay_wallet_tx_scan_marker(&seed_hash, &identity_id, "testnet")
                .expect("marker present")
        );
        assert!(
            !db.has_dashpay_wallet_tx_scan_marker(&seed_hash, &identity_id, "mainnet")
                .expect("marker isolated by network")
        );
    }

    #[test]
    fn get_all_dashpay_address_mappings_returns_error_for_malformed_contact_id() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();

        db.execute(
            "INSERT INTO dashpay_address_mappings
             (address, seed_hash, owner_identity_id, contact_identity_id, address_index, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, unixepoch())",
            rusqlite::params![
                "yXbQ1xJVDKEXAMPLE9SvM6Vx4dTuk49k19",
                owner_id.to_buffer().to_vec(),
                vec![1_u8, 2, 3],
                0_u32,
            ],
        )
        .expect("insert malformed row");

        let err = db
            .get_all_dashpay_address_mappings(&owner_id)
            .expect_err("malformed identifier should fail");

        assert!(matches!(
            err,
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Blob, _)
        ));
    }

    #[test]
    fn save_new_accepted_contact_clears_scan_markers_for_identity_and_network() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let other_identity = Identifier::random();
        let testnet_seed = [0x11; 32];
        let mainnet_seed = [0x22; 32];
        let other_seed = [0x33; 32];

        db.mark_dashpay_wallet_tx_scan_complete(&testnet_seed, &owner_id, "testnet")
            .expect("mark testnet complete");
        db.mark_dashpay_wallet_tx_scan_complete(&mainnet_seed, &owner_id, "mainnet")
            .expect("mark mainnet complete");
        db.mark_dashpay_wallet_tx_scan_complete(&other_seed, &other_identity, "testnet")
            .expect("mark other identity complete");

        db.save_dashpay_contact(
            &owner_id,
            &contact_id,
            "testnet",
            Some("contact"),
            None,
            None,
            None,
            "accepted",
        )
        .expect("save accepted contact");

        assert!(
            !db.has_dashpay_wallet_tx_scan_marker(&testnet_seed, &owner_id, "testnet")
                .expect("testnet marker cleared")
        );
        assert!(
            db.has_dashpay_wallet_tx_scan_marker(&mainnet_seed, &owner_id, "mainnet")
                .expect("mainnet marker preserved")
        );
        assert!(
            db.has_dashpay_wallet_tx_scan_marker(&other_seed, &other_identity, "testnet")
                .expect("other identity marker preserved")
        );
    }

    #[test]
    fn save_contact_acceptance_transition_clears_markers_once() {
        let db = create_test_database().expect("test db");
        let owner_id = Identifier::random();
        let contact_id = Identifier::random();
        let seed_hash = [0x44; 32];

        db.save_dashpay_contact(
            &owner_id,
            &contact_id,
            "testnet",
            Some("contact"),
            None,
            None,
            None,
            "pending",
        )
        .expect("save pending contact");
        db.mark_dashpay_wallet_tx_scan_complete(&seed_hash, &owner_id, "testnet")
            .expect("mark pending scan");

        db.save_dashpay_contact(
            &owner_id,
            &contact_id,
            "testnet",
            Some("contact"),
            None,
            None,
            None,
            "accepted",
        )
        .expect("promote to accepted");
        assert!(
            !db.has_dashpay_wallet_tx_scan_marker(&seed_hash, &owner_id, "testnet")
                .expect("marker cleared on acceptance")
        );

        db.mark_dashpay_wallet_tx_scan_complete(&seed_hash, &owner_id, "testnet")
            .expect("mark complete again");
        db.save_dashpay_contact(
            &owner_id,
            &contact_id,
            "testnet",
            Some("contact-renamed"),
            Some("Updated"),
            None,
            None,
            "accepted",
        )
        .expect("refresh accepted contact");
        assert!(
            db.has_dashpay_wallet_tx_scan_marker(&seed_hash, &owner_id, "testnet")
                .expect("accepted refresh keeps marker")
        );
    }
}
