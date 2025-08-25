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
                identity_id BLOB PRIMARY KEY,
                display_name TEXT,
                bio TEXT,
                avatar_url TEXT,
                avatar_hash BLOB,
                avatar_fingerprint BLOB,
                public_message TEXT,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        )?;

        // Contacts table (extends the existing contact_private_info)
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contacts (
                owner_identity_id BLOB NOT NULL,
                contact_identity_id BLOB NOT NULL,
                username TEXT,
                display_name TEXT,
                avatar_url TEXT,
                public_message TEXT,
                contact_status TEXT DEFAULT 'pending',
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch()),
                last_seen INTEGER,
                PRIMARY KEY (owner_identity_id, contact_identity_id)
            )",
            [],
        )?;

        // Contact requests table
        tx.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contact_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
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
                tx_id TEXT UNIQUE NOT NULL,
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

        Ok(())
    }

    /// Initialize all DashPay-related database tables
    pub fn init_dashpay_tables(&self) -> rusqlite::Result<()> {
        // Profiles table
        self.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_profiles (
                identity_id BLOB PRIMARY KEY,
                display_name TEXT,
                bio TEXT,
                avatar_url TEXT,
                avatar_hash BLOB,
                avatar_fingerprint BLOB,
                public_message TEXT,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        )?;

        // Contacts table (extends the existing contact_private_info)
        self.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contacts (
                owner_identity_id BLOB NOT NULL,
                contact_identity_id BLOB NOT NULL,
                username TEXT,
                display_name TEXT,
                avatar_url TEXT,
                public_message TEXT,
                contact_status TEXT DEFAULT 'pending',
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch()),
                last_seen INTEGER,
                PRIMARY KEY (owner_identity_id, contact_identity_id)
            )",
            [],
        )?;

        // Contact requests table
        self.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_contact_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_identity_id BLOB NOT NULL,
                to_identity_id BLOB NOT NULL,
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
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_contact_requests_from 
             ON dashpay_contact_requests(from_identity_id)",
            [],
        )?;

        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_contact_requests_to 
             ON dashpay_contact_requests(to_identity_id)",
            [],
        )?;

        // Payments/transactions table
        self.execute(
            "CREATE TABLE IF NOT EXISTS dashpay_payments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_id TEXT UNIQUE NOT NULL,
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
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_payments_from 
             ON dashpay_payments(from_identity_id)",
            [],
        )?;

        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_payments_to 
             ON dashpay_payments(to_identity_id)",
            [],
        )?;

        Ok(())
    }

    // Profile operations

    pub fn save_dashpay_profile(
        &self,
        identity_id: &Identifier,
        display_name: Option<&str>,
        bio: Option<&str>,
        avatar_url: Option<&str>,
        public_message: Option<&str>,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT OR REPLACE INTO dashpay_profiles 
            (identity_id, display_name, bio, avatar_url, public_message, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
        ";

        let result = self.execute(
            sql,
            params![
                identity_id.to_buffer().to_vec(),
                display_name,
                bio,
                avatar_url,
                public_message,
            ],
        );

        result?;
        Ok(())
    }

    pub fn load_dashpay_profile(
        &self,
        identity_id: &Identifier,
    ) -> rusqlite::Result<Option<StoredProfile>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT identity_id, display_name, bio, avatar_url, avatar_hash, 
                    avatar_fingerprint, public_message, created_at, updated_at 
             FROM dashpay_profiles 
             WHERE identity_id = ?1",
        )?;

        let result = stmt.query_row(params![identity_id.to_buffer().to_vec()], |row| {
            Ok(StoredProfile {
                identity_id: row.get(0)?,
                display_name: row.get(1)?,
                bio: row.get(2)?,
                avatar_url: row.get(3)?,
                avatar_hash: row.get(4)?,
                avatar_fingerprint: row.get(5)?,
                public_message: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
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
        username: Option<&str>,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
        public_message: Option<&str>,
        contact_status: &str,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT OR REPLACE INTO dashpay_contacts 
            (owner_identity_id, contact_identity_id, username, display_name, 
             avatar_url, public_message, contact_status, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
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
    ) -> rusqlite::Result<Vec<StoredContact>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT owner_identity_id, contact_identity_id, username, display_name, 
                    avatar_url, public_message, contact_status, created_at, updated_at, last_seen
             FROM dashpay_contacts 
             WHERE owner_identity_id = ?1
             ORDER BY updated_at DESC",
        )?;

        let contacts = stmt
            .query_map(params![owner_identity_id.to_buffer().to_vec()], |row| {
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
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(contacts)
    }

    pub fn update_contact_last_seen(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
    ) -> rusqlite::Result<()> {
        let sql = "
            UPDATE dashpay_contacts 
            SET last_seen = unixepoch(), updated_at = unixepoch()
            WHERE owner_identity_id = ?1 AND contact_identity_id = ?2
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
            ],
        )?;
        Ok(())
    }

    // Contact request operations

    pub fn save_contact_request(
        &self,
        from_identity_id: &Identifier,
        to_identity_id: &Identifier,
        to_username: Option<&str>,
        account_label: Option<&str>,
        request_type: &str,
    ) -> rusqlite::Result<i64> {
        let sql = "
            INSERT INTO dashpay_contact_requests 
            (from_identity_id, to_identity_id, to_username, account_label, request_type)
            VALUES (?1, ?2, ?3, ?4, ?5)
        ";

        let conn = self.conn.lock().unwrap();
        conn.execute(
            sql,
            params![
                from_identity_id.to_buffer().to_vec(),
                to_identity_id.to_buffer().to_vec(),
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
        request_type: &str,
    ) -> rusqlite::Result<Vec<StoredContactRequest>> {
        let conn = self.conn.lock().unwrap();
        let sql = if request_type == "sent" {
            "SELECT id, from_identity_id, to_identity_id, to_username, account_label, 
                    request_type, status, created_at, responded_at, expires_at
             FROM dashpay_contact_requests 
             WHERE from_identity_id = ?1 AND request_type = 'sent' AND status = 'pending'
             ORDER BY created_at DESC"
        } else {
            "SELECT id, from_identity_id, to_identity_id, to_username, account_label, 
                    request_type, status, created_at, responded_at, expires_at
             FROM dashpay_contact_requests 
             WHERE to_identity_id = ?1 AND request_type = 'received' AND status = 'pending'
             ORDER BY created_at DESC"
        };

        let mut stmt = conn.prepare(sql)?;
        let requests = stmt
            .query_map(params![identity_id.to_buffer().to_vec()], |row| {
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

        Ok(())
    }
}
