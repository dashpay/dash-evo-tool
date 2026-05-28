//! DashPay domain types shared by the `WalletBackend` adapter, backend tasks,
//! and the UI. Pure data — no I/O, no SDK calls.

use serde::{Deserialize, Serialize};

/// DashPay profile data — the local snapshot of an identity's published profile.
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

/// DashPay contact — an accepted or pending relationship between two identities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredContact {
    pub owner_identity_id: Vec<u8>,
    pub contact_identity_id: Vec<u8>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub public_message: Option<String>,
    /// One of: `"pending"`, `"accepted"`, `"blocked"`.
    pub contact_status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen: Option<i64>,
}

/// DashPay contact request — pending, accepted, rejected, or expired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredContactRequest {
    pub id: i64,
    pub from_identity_id: Vec<u8>,
    pub to_identity_id: Vec<u8>,
    pub to_username: Option<String>,
    pub account_label: Option<String>,
    /// One of: `"sent"`, `"received"`.
    pub request_type: String,
    /// One of: `"pending"`, `"accepted"`, `"rejected"`, `"expired"`.
    pub status: String,
    pub created_at: i64,
    pub responded_at: Option<i64>,
    pub expires_at: Option<i64>,
}

/// DashPay payment record. `amount` is in credits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPayment {
    pub id: i64,
    pub tx_id: String,
    pub from_identity_id: Vec<u8>,
    pub to_identity_id: Vec<u8>,
    pub amount: i64,
    pub memo: Option<String>,
    /// One of: `"sent"`, `"received"`.
    pub payment_type: String,
    /// One of: `"pending"`, `"confirmed"`, `"failed"`.
    pub status: String,
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
}

/// DashPay contact address index tracking per DIP-0015.
///
/// Tracks address indices used for sending to / receiving from a specific
/// contact, plus how many addresses have been registered with the bloom filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactAddressIndex {
    pub owner_identity_id: Vec<u8>,
    pub contact_identity_id: Vec<u8>,
    /// Next address index to use when sending TO this contact.
    pub next_send_index: u32,
    /// Highest address index seen when receiving FROM this contact (for bloom filter).
    pub highest_receive_index: u32,
    /// Number of addresses registered in the bloom filter for this contact.
    pub bloom_registered_count: u32,
}
