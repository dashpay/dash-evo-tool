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

/// Relationship state of a DashPay contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactStatus {
    Pending,
    Accepted,
    Blocked,
}

/// Direction of a DashPay contact request relative to the local identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactRequestDirection {
    Sent,
    Received,
}

/// Lifecycle state of a DashPay contact request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactRequestStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
}

/// Direction of a DashPay payment relative to the local identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentDirection {
    Sent,
    Received,
}

/// Lifecycle state of a DashPay payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Pending,
    Confirmed,
    Failed,
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
    pub contact_status: ContactStatus,
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
    pub request_type: ContactRequestDirection,
    pub status: ContactRequestStatus,
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
    pub payment_type: PaymentDirection,
    pub status: PaymentStatus,
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

/// A received output observed on-chain that may be an incoming DashPay
/// contact payment.
///
/// The [`EventBridge`](crate::wallet_backend::EventBridge) extracts these
/// from freshly-seen wallet transactions and hands them to the
/// detect-match-record path, which resolves each `address` against the
/// per-identity DashPay address map. Outputs whose address is not a
/// registered contact-receiving address are ignored — this carries every
/// received output, not only the DashPay ones, so the detector owns the
/// matching decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedIncomingOutput {
    /// Hex transaction id of the observed transaction.
    pub txid: String,
    /// Index of this output within the transaction (the `vout`). A single
    /// transaction can pay two different contact addresses, so the payment is
    /// keyed by `(txid, vout)` — `txid` alone would let the second output
    /// overwrite the first.
    pub vout: u32,
    /// Base58 receiving address the output paid into.
    pub address: String,
    /// Output value in duffs.
    pub amount_duffs: u64,
}

/// Build the storage key that distinguishes each output of one transaction.
///
/// Upstream `record_dashpay_payment` keys its payment map by an opaque `tx_id`
/// `String` with last-write-wins semantics, so two contact outputs in the same
/// transaction would collide on the bare txid. Keying by `"{txid}:{vout}"`
/// keeps every output a distinct record while remaining an idempotent upsert
/// per `(txid, vout)` on re-scan.
pub fn payment_storage_key(txid: &str, vout: u32) -> String {
    format!("{txid}:{vout}")
}

/// Recover the bare transaction id from a [`payment_storage_key`].
///
/// Splits on the last `:` and validates that the suffix is a `vout` integer;
/// returns the input unchanged when it carries no `:vout` suffix (a plain txid,
/// e.g. a legacy or sent-payment record keyed by txid alone). The transaction
/// id itself never contains a `:`, so the last-colon split is unambiguous.
pub fn payment_txid_from_storage_key(key: &str) -> &str {
    match key.rsplit_once(':') {
        Some((txid, vout)) if !vout.is_empty() && vout.bytes().all(|b| b.is_ascii_digit()) => txid,
        _ => key,
    }
}

/// A cached avatar image, stored DET-side so avatars survive offline and are
/// not re-fetched from the network on every contact view.
///
/// Keyed by the avatar URL. The raw image `bytes` are validated before
/// caching, so a cache hit can be decoded directly. `sha256` is the content
/// hash used to detect a changed image at the same URL (cache invalidation),
/// and `fetched_at_ms` is the wall-clock fetch time for age-based eviction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAvatar {
    /// Raw, already-validated image bytes.
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes` — detects a content change at the same URL.
    pub sha256: Vec<u8>,
    /// Unix milliseconds at fetch time, for age-based invalidation.
    pub fetched_at_ms: i64,
}

/// DET-local private contact memo (nickname / notes / hidden flag).
///
/// Mirrors the legacy `contact_private_info` SQLite row shape but lives
/// entirely in the per-network k/v sidecar. No upstream counterpart —
/// DashPay carries this state encrypted in `contactInfo` documents, and
/// DET keeps a local plaintext snapshot for offline-friendly display.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactPrivateInfo {
    pub nickname: String,
    pub notes: String,
    pub is_hidden: bool,
}

/// Maximum `displayName` length in Unicode characters (DIP-0015). Empty is legal.
pub const MAX_DISPLAY_NAME_CHARS: usize = 25;
/// Maximum `publicMessage` (bio) length in Unicode characters (DIP-0015).
pub const MAX_BIO_CHARS: usize = 140;
/// Maximum `avatarUrl` length in Unicode characters (DIP-0015).
pub const MAX_AVATAR_URL_CHARS: usize = 2048;

/// A single DashPay profile field violation, returned by [`validate_profile_fields`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileFieldError {
    /// Display name exceeds [`MAX_DISPLAY_NAME_CHARS`].
    DisplayNameTooLong { len: usize, max: usize },
    /// Bio exceeds [`MAX_BIO_CHARS`].
    BioTooLong { len: usize, max: usize },
    /// Avatar URL exceeds [`MAX_AVATAR_URL_CHARS`].
    AvatarUrlTooLong { len: usize, max: usize },
    /// Avatar URL is set but does not start with `http://` or `https://`.
    AvatarUrlInvalidScheme,
}

impl ProfileFieldError {
    /// User-facing, i18n-ready message describing the violation.
    pub fn message(&self) -> String {
        match self {
            ProfileFieldError::DisplayNameTooLong { len, max } => {
                format!("Display name is {len} characters, but the maximum is {max}.")
            }
            ProfileFieldError::BioTooLong { len, max } => {
                format!("Bio is {len} characters, but the maximum is {max}.")
            }
            ProfileFieldError::AvatarUrlTooLong { len, max } => {
                format!("Avatar URL is {len} characters, but the maximum is {max}.")
            }
            ProfileFieldError::AvatarUrlInvalidScheme => {
                "Avatar URL must start with http:// or https://.".to_string()
            }
        }
    }
}

/// Validate DashPay profile fields per DIP-0015, the single source of truth for
/// both the profile editors and the backend enforcement layer.
///
/// Lengths are measured in Unicode characters (`chars().count()`), matching the
/// protocol. Every field may be empty — an unset display name, bio, or avatar is
/// valid. The avatar URL is trimmed before its scheme and length are checked.
/// Returns every violation found (empty when the input is valid).
pub fn validate_profile_fields(
    display_name: &str,
    bio: &str,
    avatar_url: &str,
) -> Vec<ProfileFieldError> {
    let mut errors = Vec::new();

    let name_len = display_name.chars().count();
    if name_len > MAX_DISPLAY_NAME_CHARS {
        errors.push(ProfileFieldError::DisplayNameTooLong {
            len: name_len,
            max: MAX_DISPLAY_NAME_CHARS,
        });
    }

    let bio_len = bio.chars().count();
    if bio_len > MAX_BIO_CHARS {
        errors.push(ProfileFieldError::BioTooLong {
            len: bio_len,
            max: MAX_BIO_CHARS,
        });
    }

    let url = avatar_url.trim();
    let url_len = url.chars().count();
    if url_len > MAX_AVATAR_URL_CHARS {
        errors.push(ProfileFieldError::AvatarUrlTooLong {
            len: url_len,
            max: MAX_AVATAR_URL_CHARS,
        });
    }
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        errors.push(ProfileFieldError::AvatarUrlInvalidScheme);
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_key_distinguishes_outputs_of_one_tx() {
        let a = payment_storage_key("abc123", 0);
        let b = payment_storage_key("abc123", 1);
        assert_ne!(a, b, "two outputs of one tx must produce distinct keys");
        assert_eq!(a, "abc123:0");
        assert_eq!(b, "abc123:1");
    }

    #[test]
    fn storage_key_round_trips_to_bare_txid() {
        let key = payment_storage_key("deadbeef", 7);
        assert_eq!(payment_txid_from_storage_key(&key), "deadbeef");
    }

    #[test]
    fn bare_txid_without_vout_suffix_is_returned_unchanged() {
        // A sent-payment / legacy record keyed by txid alone has no ":vout".
        assert_eq!(payment_txid_from_storage_key("plainTxid"), "plainTxid");
    }

    #[test]
    fn non_numeric_suffix_is_not_treated_as_vout() {
        // Defensive: a colon followed by non-digits is not a vout suffix, so the
        // whole string is the txid (txids themselves never contain a colon).
        assert_eq!(payment_txid_from_storage_key("tx:abc"), "tx:abc");
        assert_eq!(payment_txid_from_storage_key("tx:"), "tx:");
    }

    #[test]
    fn valid_profile_fields_report_no_errors() {
        assert!(
            validate_profile_fields("Alex", "A short bio.", "https://example.com/a.png").is_empty()
        );
    }

    #[test]
    fn empty_profile_fields_are_legal() {
        // Every field is optional per DIP-0015 — an empty display name, bio, and
        // avatar URL together are a valid (blank) profile.
        assert!(validate_profile_fields("", "", "").is_empty());
    }

    #[test]
    fn display_name_over_limit_is_rejected() {
        let name = "a".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        let errors = validate_profile_fields(&name, "", "");
        assert_eq!(
            errors,
            vec![ProfileFieldError::DisplayNameTooLong {
                len: MAX_DISPLAY_NAME_CHARS + 1,
                max: MAX_DISPLAY_NAME_CHARS,
            }]
        );
    }

    #[test]
    fn length_is_measured_in_characters_not_bytes() {
        // 25 multi-byte characters are within the 25-character limit even though
        // the byte length far exceeds it — the check must use character count.
        let name: String = "é".repeat(MAX_DISPLAY_NAME_CHARS);
        assert!(
            name.len() > MAX_DISPLAY_NAME_CHARS,
            "precondition: bytes exceed chars"
        );
        assert!(validate_profile_fields(&name, "", "").is_empty());
    }

    #[test]
    fn bio_over_limit_is_rejected() {
        let bio = "b".repeat(MAX_BIO_CHARS + 1);
        assert_eq!(
            validate_profile_fields("", &bio, ""),
            vec![ProfileFieldError::BioTooLong {
                len: MAX_BIO_CHARS + 1,
                max: MAX_BIO_CHARS,
            }]
        );
    }

    #[test]
    fn avatar_url_over_limit_is_rejected() {
        let url = format!("https://example.com/{}", "x".repeat(MAX_AVATAR_URL_CHARS));
        let errors = validate_profile_fields("", "", &url);
        assert!(errors.contains(&ProfileFieldError::AvatarUrlTooLong {
            len: url.chars().count(),
            max: MAX_AVATAR_URL_CHARS,
        }));
    }

    #[test]
    fn avatar_url_without_http_scheme_is_rejected() {
        assert_eq!(
            validate_profile_fields("", "", "ftp://example.com/a.png"),
            vec![ProfileFieldError::AvatarUrlInvalidScheme]
        );
    }

    #[test]
    fn avatar_url_scheme_check_ignores_surrounding_whitespace() {
        assert!(validate_profile_fields("", "", "  https://example.com/a.png  ").is_empty());
    }
}
