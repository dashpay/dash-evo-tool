//! DashPay domain types shared by the `WalletBackend` adapter, backend tasks,
//! and the UI. Pure data — no I/O, no SDK calls.

use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::platform::{Document, Identifier};
use serde::{Deserialize, Serialize};

use super::validation::{TextLengthError, validate_char_count};

/// Independent reads used to populate an identity's contact lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContactsLoadKind {
    Contacts,
    Requests,
}

/// Maximum number of characters stored in a DashPay payment memo.
pub const MAX_PAYMENT_MEMO_CHARS: usize = 100;
/// Maximum number of characters stored in a contact-request account label.
pub const MAX_ACCOUNT_LABEL_CHARS: usize = 100;

/// Validate an optional DashPay payment memo.
pub fn validate_payment_memo(memo: &str) -> Result<(), TextLengthError> {
    validate_char_count(memo, 0, MAX_PAYMENT_MEMO_CHARS)
}

/// Validate a DashPay contact-request account label.
pub fn validate_account_label(label: &str) -> Result<(), TextLengthError> {
    validate_char_count(label, 0, MAX_ACCOUNT_LABEL_CHARS)
}

/// Application limit for a contact's private note, in UTF-8 bytes.
/// The encrypted `contactInfo` payload encodes its length with CompactSize.
pub const MAX_CONTACT_NOTE_BYTES: usize = u8::MAX as usize;
/// DashPay contract limit for the encrypted contact-info payload, including its IV.
pub const MAX_CONTACT_INFO_ENCRYPTED_BYTES: usize = 2048;

/// A private contact note exceeds the application's size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("This note is too long. Shorten it to save it.")]
pub struct ContactNoteTooLong {
    /// Size of the note in UTF-8 bytes.
    pub bytes: usize,
    /// Largest note size the application accepts.
    pub max: usize,
}

/// Validate a contact's private note. The limit counts UTF-8 bytes, not
/// characters, because that is what the payload stores.
pub fn validate_contact_note(note: &str) -> Result<(), ContactNoteTooLong> {
    let bytes = note.len();
    if bytes > MAX_CONTACT_NOTE_BYTES {
        return Err(ContactNoteTooLong {
            bytes,
            max: MAX_CONTACT_NOTE_BYTES,
        });
    }
    Ok(())
}

/// The recipient (`toUserId`) of a DashPay `contactRequest` document.
///
/// Returns `None` when the field is absent or does not hold a readable
/// identifier — a malformed document. Callers decide what that means for them:
/// the request lists keep such a row (they cannot prove it was resolved), while
/// the cancel path refuses to act on one.
pub fn contact_request_recipient(document: &Document) -> Option<Identifier> {
    document
        .properties()
        .get("toUserId")
        .and_then(|value| value.to_identifier().ok())
}

/// A completed profile operation, tied to its owner and shared cache revision.
#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    pub network: dash_sdk::dpp::dashcore::Network,
    pub owner: Identifier,
    pub revision: u64,
    /// Display name, biography, and avatar URL; `None` is authoritative absence.
    pub profile: Option<(String, String, String)>,
}

/// DashPay profile data — the local snapshot of an identity's published profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// What a contact details write does to one optional detail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ContactInfoField<T> {
    /// Keep the value currently saved for the contact.
    #[default]
    Preserve,
    /// Store this value, including `None` when clearing an optional field.
    Replace(T),
}

/// The private details an identity keeps about one contact: its nickname,
/// note, and whether it is hidden.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContactDetails {
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub hidden: bool,
}

impl ContactDetails {
    /// Whether nothing is saved for the contact.
    pub fn is_empty(&self) -> bool {
        self.nickname.is_none() && self.note.is_none() && !self.hidden
    }
}

/// The details the contacts list shows for one contact, and that a save
/// keeps for every field it does not replace.
///
/// A published DIP-0015 document wins: Platform holds the latest saved state
/// from any device. Without one, platform-wallet's details on this device
/// win; a legacy-key document stands in only while platform-wallet has never
/// held details for the contact (`synced_before` false). Once it has, its
/// details win even when empty, so a cleared note or an unhide is never
/// undone by an old legacy document.
pub fn shown_contact_details(
    published: Option<ContactDetails>,
    local: Option<ContactDetails>,
    synced_before: bool,
    legacy: Option<ContactDetails>,
) -> ContactDetails {
    if let Some(published) = published {
        return published;
    }
    if synced_before {
        return local.unwrap_or_default();
    }
    local
        .filter(|details| !details.is_empty())
        .or(legacy)
        .unwrap_or_default()
}

/// Whether platform-wallet's side holds details worth recording as its own:
/// a published DIP-0015 document (even an empty one), or non-empty details on
/// this device.
pub fn holds_details(published: Option<&ContactDetails>, local: Option<&ContactDetails>) -> bool {
    published.is_some() || local.is_some_and(|details| !details.is_empty())
}

/// Explicit field-by-field intent for a contact details write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactInfoUpdate {
    /// Nickname update intent.
    pub nickname: ContactInfoField<Option<String>>,
    /// Note update intent.
    pub note: ContactInfoField<Option<String>>,
    /// Visibility update intent.
    pub display_hidden: ContactInfoField<bool>,
}

impl ContactInfoUpdate {
    /// Change visibility while preserving every unrelated contact detail.
    pub fn visibility(display_hidden: bool) -> Self {
        Self {
            nickname: ContactInfoField::Preserve,
            note: ContactInfoField::Preserve,
            display_hidden: ContactInfoField::Replace(display_hidden),
        }
    }

    /// Replace only the private note, preserving the latest stored contact details.
    pub fn note(note: Option<String>) -> Self {
        Self {
            note: ContactInfoField::Replace(note),
            display_hidden: ContactInfoField::Preserve,
            ..Self::visibility(false)
        }
    }

    /// Replace every editable field with caller-owned values.
    pub fn replace_all(
        nickname: Option<String>,
        note: Option<String>,
        display_hidden: bool,
    ) -> Self {
        Self {
            nickname: ContactInfoField::Replace(nickname),
            note: ContactInfoField::Replace(note),
            display_hidden: ContactInfoField::Replace(display_hidden),
        }
    }

    /// Replace only the fields that differ between `opened`, the details an
    /// editor started from, and `edited`, what the user left in it; every
    /// other field keeps the latest saved value.
    ///
    /// An editor that starts from a local copy must save this way: replacing
    /// a field the user did not touch would overwrite a newer value saved
    /// from another device with the local copy's older one.
    pub fn edits(opened: &ContactDetails, edited: ContactDetails) -> Self {
        fn field<T: PartialEq>(opened: &T, edited: T) -> ContactInfoField<T> {
            if *opened == edited {
                ContactInfoField::Preserve
            } else {
                ContactInfoField::Replace(edited)
            }
        }
        Self {
            nickname: field(&opened.nickname, edited.nickname),
            note: field(&opened.note, edited.note),
            display_hidden: field(&opened.hidden, edited.hidden),
        }
    }

    /// The details to save: `current` with this update's replacements.
    pub fn apply_to(&self, current: ContactDetails) -> ContactDetails {
        fn pick<T: Clone>(field: &ContactInfoField<T>, current: T) -> T {
            match field {
                ContactInfoField::Preserve => current,
                ContactInfoField::Replace(value) => value.clone(),
            }
        }
        ContactDetails {
            nickname: pick(&self.nickname, current.nickname),
            note: pick(&self.note, current.note),
            hidden: pick(&self.display_hidden, current.hidden),
        }
    }
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// An established contact as the DashPay screens show it: the private
/// details the owner keeps about them plus their public profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactData {
    pub identity_id: Identifier,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub is_hidden: bool,
    pub account_reference: u32,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
}

impl ContactData {
    /// The private details the owner keeps about this contact.
    pub fn details(&self) -> ContactDetails {
        ContactDetails {
            nickname: self.nickname.clone(),
            note: self.note.clone(),
            hidden: self.is_hidden,
        }
    }
}

/// One identity's contact request exchange with one counterparty: what an
/// Accept, Decline or Cancel acts on. Platform holds at most one request each
/// way between two identities, so the pair names the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContactRequestKey {
    /// The identity acting on the request.
    pub owner: Identifier,
    /// The other identity: the sender of a received request, the recipient
    /// of a sent one.
    pub counterparty: Identifier,
}

/// A contact request still waiting for an answer, identified by the other
/// identity: DashPay keeps at most one request per direction and pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingContactRequest {
    /// The sender of an incoming request, the recipient of an outgoing one.
    pub counterparty_id: Identifier,
    /// Unix milliseconds the request was created at; `0` when unknown.
    pub created_at_ms: u64,
    pub account_reference: u32,
    /// Whether the request carries a proof that lets it be accepted at once.
    pub has_auto_accept_proof: bool,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// What this device knows about an identity's own DashPay profile.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ProfileState {
    /// Not synced from Platform yet.
    #[default]
    Unknown,
    /// Synced; the identity has published no profile.
    Absent,
    Present(StoredProfile),
}

impl ProfileState {
    /// The published profile, if one is known.
    pub fn profile(&self) -> Option<&StoredProfile> {
        match self {
            Self::Present(profile) => Some(profile),
            Self::Unknown | Self::Absent => None,
        }
    }
}

/// One identity's DashPay state: what platform-wallet synced from Platform,
/// combined with this device's own records (withdrawn and declined requests,
/// private memos, payment times).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DashPaySnapshot {
    pub profile: ProfileState,
    /// Established contacts, hidden ones included (see
    /// [`ContactData::is_hidden`]).
    pub contacts: Vec<ContactData>,
    pub incoming: Vec<PendingContactRequest>,
    pub outgoing: Vec<PendingContactRequest>,
    /// Newest first.
    pub payments: Vec<StoredPayment>,
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

/// Least time between two snapshot refreshes the frame loop asks for.
const SNAPSHOT_REFRESH_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(1);
/// Longest time the frame loop goes without a snapshot refresh.
const SNAPSHOT_REFRESH_MAX_GAP: std::time::Duration = std::time::Duration::from_secs(15);

/// Whether the frame loop should ask for the DashPay snapshots to be
/// republished: after a sync pass completed, or once the last refresh is old,
/// but never twice within a second. `since_last` is `None` before the first.
pub fn snapshot_refresh_due(pass_completed: bool, since_last: Option<std::time::Duration>) -> bool {
    match since_last {
        None => true,
        Some(elapsed) if elapsed < SNAPSHOT_REFRESH_MIN_GAP => false,
        Some(elapsed) => pass_completed || elapsed >= SNAPSHOT_REFRESH_MAX_GAP,
    }
}

/// Recover the bare transaction id from a payment storage key.
///
/// Older versions recorded each incoming payment under `"{txid}:{vout}"`;
/// every other payment is keyed by its bare txid, which is returned unchanged.
/// A transaction id never contains a `:`, so the last-colon split is
/// unambiguous.
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
    use dash_sdk::dpp::document::{Document as DppDocument, DocumentV0};
    use dash_sdk::dpp::platform_value::Value;
    use std::collections::BTreeMap;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    /// A `contactRequest`-shaped document. `to` of `None` omits `toUserId`
    /// entirely, modelling a malformed document.
    fn request_doc(to: Option<Identifier>) -> Document {
        let mut properties = BTreeMap::new();
        if let Some(to) = to {
            properties.insert("toUserId".to_string(), Value::Identifier(to.to_buffer()));
        }
        DppDocument::V0(DocumentV0 {
            id: id(99),
            owner_id: id(1),
            creator_id: None,
            properties,
            revision: Some(1),
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        })
    }

    #[test]
    fn a_contact_note_fits_up_to_the_payload_limit_in_bytes() {
        assert_eq!(MAX_CONTACT_NOTE_BYTES, 255);
        assert_eq!(validate_contact_note(""), Ok(()));
        assert_eq!(validate_contact_note(&"a".repeat(255)), Ok(()));
        assert_eq!(
            validate_contact_note(&"a".repeat(256)),
            Err(ContactNoteTooLong {
                bytes: 256,
                max: 255
            })
        );
        // 128 two-byte letters are 128 characters but 256 bytes.
        assert_eq!(
            validate_contact_note(&"é".repeat(128)),
            Err(ContactNoteTooLong {
                bytes: 256,
                max: 255
            })
        );
    }

    #[test]
    fn a_note_update_replaces_only_the_note() {
        let update = ContactInfoUpdate::note(Some("Met at the meetup".into()));
        assert_eq!(
            update.note,
            ContactInfoField::Replace(Some("Met at the meetup".into()))
        );
        assert_eq!(update.nickname, ContactInfoField::Preserve);
        assert_eq!(update.display_hidden, ContactInfoField::Preserve);

        let cleared = ContactInfoUpdate::note(None);
        assert_eq!(cleared.note, ContactInfoField::Replace(None));
        assert_eq!(cleared.display_hidden, ContactInfoField::Preserve);
    }

    /// A write keeps every detail it does not replace.
    #[test]
    fn an_update_keeps_the_details_it_does_not_replace() {
        let current = ContactDetails {
            nickname: Some("Bao".into()),
            note: Some("old".into()),
            hidden: true,
        };
        assert_eq!(
            ContactInfoUpdate::note(Some("new".into())).apply_to(current.clone()),
            ContactDetails {
                note: Some("new".into()),
                ..current.clone()
            }
        );
        assert_eq!(
            ContactInfoUpdate::visibility(false).apply_to(current.clone()),
            ContactDetails {
                hidden: false,
                ..current.clone()
            }
        );
        assert_eq!(
            ContactInfoUpdate::replace_all(None, None, false).apply_to(current),
            ContactDetails::default()
        );
    }

    /// Only the fields an editor changed are replaced.
    #[test]
    fn edits_replace_only_the_changed_fields() {
        let opened = ContactDetails {
            nickname: Some("Bao".into()),
            note: None,
            hidden: false,
        };
        assert_eq!(
            ContactInfoUpdate::edits(&opened, opened.clone()),
            ContactInfoUpdate {
                nickname: ContactInfoField::Preserve,
                note: ContactInfoField::Preserve,
                display_hidden: ContactInfoField::Preserve,
            }
        );
        assert_eq!(
            ContactInfoUpdate::edits(
                &opened,
                ContactDetails {
                    nickname: None,
                    note: Some("n".into()),
                    hidden: false,
                }
            ),
            ContactInfoUpdate {
                nickname: ContactInfoField::Replace(None),
                note: ContactInfoField::Replace(Some("n".into())),
                display_hidden: ContactInfoField::Preserve,
            }
        );
    }

    #[test]
    fn contact_request_recipient_reads_the_to_user_id() {
        assert_eq!(
            contact_request_recipient(&request_doc(Some(id(2)))),
            Some(id(2))
        );
    }

    #[test]
    fn a_contact_request_without_a_recipient_is_unreadable() {
        // Callers must be able to tell "recipient is X" from "cannot attribute
        // this request" — the latter is what guards the cancel path.
        assert_eq!(contact_request_recipient(&request_doc(None)), None);
    }

    #[test]
    fn snapshots_refresh_after_a_pass_or_when_old_but_not_twice_a_second() {
        use std::time::Duration;
        assert!(
            snapshot_refresh_due(false, None),
            "the first frame refreshes"
        );
        assert!(snapshot_refresh_due(true, Some(Duration::from_secs(2))));
        assert!(!snapshot_refresh_due(false, Some(Duration::from_secs(2))));
        assert!(!snapshot_refresh_due(
            true,
            Some(Duration::from_millis(500))
        ));
        assert!(snapshot_refresh_due(false, Some(Duration::from_secs(15))));
    }

    #[test]
    fn a_per_output_key_reads_as_its_bare_txid() {
        assert_eq!(payment_txid_from_storage_key("deadbeef:7"), "deadbeef");
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

    #[test]
    fn dashpay_text_limits_count_characters() {
        assert!(validate_payment_memo(&"é".repeat(100)).is_ok());
        assert!(validate_payment_memo(&"m".repeat(101)).is_err());
        assert!(validate_account_label(&"é".repeat(100)).is_ok());
        assert!(validate_account_label(&"l".repeat(101)).is_err());
    }
}
