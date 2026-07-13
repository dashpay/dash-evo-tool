//! DashPay adapter for `WalletBackend`.
//!
//! Translates upstream presence-based DashPay state
//! (`platform_wallet::wallet::identity::types::dashpay::*`) into the
//! DET-shape `Stored*` records the UI and backend-task layer understand, and
//! owns the DET-local overlays those records need but upstream does not model.
//!
//! ## What this adapter owns
//!
//! - **Status derivation**: upstream stores DashPay state as presence
//!   (a row in `sent_contact_requests` ⇒ outgoing pending; a row in
//!   `established_contacts` ⇒ accepted; etc.). DET's `Stored*` records model
//!   the same state as an explicit `status`. This module performs that
//!   translation at read time — there is no cache and no extra source of
//!   truth.
//! - **DET-local overlays via the k/v sidecar**: a small set of contact /
//!   contact-request attributes have no upstream surface — `blocked`,
//!   `rejected`, DET-local `created_at` / `updated_at` timestamps, private
//!   memos, and per-contact address-index cursors. This module both reads and
//!   writes them; missing keys yield safe defaults (not blocked, not rejected,
//!   timestamps `0`).
//! - **Sync trigger**: [`WalletBackend::dashpay_sync`] refreshes upstream
//!   contact requests and profiles before a read; the view itself observes
//!   whatever state upstream holds afterwards.
//!
//! ## What this adapter does NOT do
//!
//! - It does not fetch profile / DPNS data from the network. Fields DET
//!   populates from cross-references (a contact's display name from their
//!   DashPay profile, a contact request's `to_username` from DPNS) come
//!   through as `None`; those joins live outside this adapter.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::wallet::Wallet;
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::platform::Identifier;
use zeroize::Zeroizing;

use platform_wallet::wallet::identity::types::dashpay::contact_request::ContactRequest;
use platform_wallet::wallet::identity::types::dashpay::established_contact::EstablishedContact;
use platform_wallet::wallet::identity::types::dashpay::payment::{
    PaymentDirection, PaymentEntry, PaymentStatus,
};
use platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile;
use platform_wallet::{PlatformWallet, calculate_account_reference, derive_contact_xpub};

use crate::backend_task::error::TaskError;
use crate::model::dashpay::{
    ContactAddressIndex, ContactPrivateInfo, ContactRequestDirection, ContactRequestStatus,
    ContactStatus, PaymentDirection as DetPaymentDirection, PaymentStatus as DetPaymentStatus,
    StoredContact, StoredContactRequest, StoredPayment, StoredProfile,
};
use crate::wallet_backend::kv::{DetKv, kv_get_or_default};
use crate::wallet_backend::{DetScope, WalletBackend};

// ---------------------------------------------------------------------------
// Contact xpub derivation seam (DIP-14 / DIP-15)
// ---------------------------------------------------------------------------
//
// `key_wallet` / `platform_wallet` derivation types (`Wallet`,
// `ExtendedPubKey`, `ContactXpubData`) stay inside this module. Callers
// receive only the plain byte/integer material a DashPay contact-request
// document carries. This keeps the M-DONT-LEAK-TYPES boundary intact: the
// raw 64-byte HD seed and the upstream wallet type never cross out of the
// `wallet_backend` seam.

/// Plain, type-leak-free material extracted from a contact relationship's
/// extended public key, ready to drop into a DashPay `contactRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactXpubMaterial {
    /// Parent key fingerprint (first 4 bytes of HASH160 of the parent key).
    pub parent_fingerprint: [u8; 4],
    /// Chain code of the contact xpub (32 bytes).
    pub chain_code: [u8; 32],
    /// Compressed public key of the contact xpub (33 bytes).
    pub public_key: [u8; 33],
    /// DIP-15 account reference for the relationship.
    pub account_reference: u32,
}

/// Derive the contact-relationship xpub material from the wallet's real HD
/// seed, using upstream `platform_wallet::derive_contact_xpub` as the single
/// source of truth for the `m/9'/coin'/15'/account'/(sender)/(recipient)`
/// path.
///
/// The published `public_key` / `chain_code` / `parent_fingerprint` are what
/// the contact pays into; the receive-side address derivation must therefore
/// run against the same seed and path. `account_reference` is computed via
/// upstream `calculate_account_reference` so the `ExtendedPubKey` never leaves
/// this seam.
///
/// * `seed_bytes`        - The wallet's 64-byte BIP-39 HD seed.
/// * `network`           - Network for coin-type selection in the path.
/// * `account_index`     - DIP-15 account index (hardened path segment). Both
///   callers pass `0`, so the path segment (`account 0'`) and the
///   `account_reference` HMAC below agree; a non-zero value here must be
///   threaded to the receive side too or the two would desync.
/// * `sender_id`         - The sender (our) identity.
/// * `recipient_id`      - The recipient (contact) identity.
/// * `sender_secret_key` - The sender's 32-byte ECDH secret, keying the
///   account-reference HMAC.
pub(crate) fn derive_contact_xpub_material(
    seed_bytes: &[u8; 64],
    network: Network,
    account_index: u32,
    sender_id: &Identifier,
    recipient_id: &Identifier,
    sender_secret_key: &[u8; 32],
) -> Result<ContactXpubMaterial, TaskError> {
    let wallet = Wallet::from_seed_bytes(*seed_bytes, network, WalletAccountCreationOptions::None)
        .map_err(|source| TaskError::SeedWalletBuildFailed { source })?;

    let data = derive_contact_xpub(&wallet, network, account_index, sender_id, recipient_id)
        .map_err(|e| TaskError::WalletBackend {
            source: Box::new(e),
        })?;

    let account_reference =
        calculate_account_reference(sender_secret_key, &data.compact_xpub(), account_index, 0);

    Ok(ContactXpubMaterial {
        parent_fingerprint: data.compact.parent_fingerprint,
        chain_code: data.compact.chain_code,
        public_key: data.compact.public_key,
        account_reference,
    })
}

/// Derive the DIP-0015 contactInfo encryption key pair from the wallet's real
/// HD seed.
///
/// DIP-0015 roots both keys at the encryption root `m/9'/coin'/15'/0'`, then:
/// - **Key1** (`encToUserId`): `root/(2^16)'/index'`
/// - **Key2** (`privateData`): `root/(2^16 + 1)'/index'`
///
/// The coin type is selected per network so testnet keys match the
/// spec-compliant counterparty. Both `contacts` (decrypt) and `contact_info`
/// (encrypt) callers derive through this single helper, so the BIP-32
/// derivation lives in exactly one place and the raw seed never leaves the
/// `wallet_backend` seam.
///
/// * `seed_bytes`       - The wallet's 64-byte BIP-39 HD seed (borrowed).
/// * `network`          - Network for coin-type selection in the path.
/// * `derivation_index` - The contactInfo document's derivation index.
#[allow(clippy::type_complexity)]
pub(crate) fn derive_contact_info_encryption_keys(
    seed_bytes: &[u8; 64],
    network: Network,
    derivation_index: u32,
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError> {
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath, ExtendedPrivKey};
    use std::str::FromStr;

    let derive_err =
        |e: dash_sdk::dpp::key_wallet::bip32::Error| TaskError::ContactKeyDerivationFailed {
            source: Box::new(e),
        };

    let master_xprv = ExtendedPrivKey::new_master(network, seed_bytes).map_err(derive_err)?;

    // Encryption root path: m/9'/coin'/15'/0'
    let coin_type = crate::model::wallet::coin_type_for_network(network);
    let root_path =
        DerivationPath::from_str(&format!("m/9'/{coin_type}'/15'/0'")).map_err(derive_err)?;

    let secp = dash_sdk::dpp::dashcore::secp256k1::Secp256k1::new();
    let root = master_xprv
        .derive_priv(&secp, &root_path)
        .map_err(derive_err)?;

    // Key1 (encToUserId): root/(2^16)'/index'
    let key1 = root
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(65536).map_err(derive_err)?],
        )
        .map_err(derive_err)?
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(derivation_index).map_err(derive_err)?],
        )
        .map_err(derive_err)?;

    // Key2 (privateData): root/(2^16 + 1)'/index'
    let key2 = root
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(65537).map_err(derive_err)?],
        )
        .map_err(derive_err)?
        .derive_priv(
            &secp,
            &[ChildNumber::from_hardened_idx(derivation_index).map_err(derive_err)?],
        )
        .map_err(derive_err)?;

    Ok((
        Zeroizing::new(key1.private_key.secret_bytes()),
        Zeroizing::new(key2.private_key.secret_bytes()),
    ))
}

// ---------------------------------------------------------------------------
// K/V sidecar key prefixes
// ---------------------------------------------------------------------------
//
// Two sidecar families (`timestamps`, `addr_map`) use `DetScope::Global`
// against the per-network upstream persister. The network already partitions
// the database file, so no `<network>:` prefix is needed inside the key. Four
// families (`blocked`, `rejected`, `private`, `address_index`) use
// `DetScope::Identity(&owner)` — the owner is carried by the scope, so the
// key contains only the counterparty id; the upstream soft-cascade reaps them
// when the owner identity row is deleted.

/// Mark a contact as blocked. Value: empty (`()`). Presence is the signal.
/// Scope: [`DetScope::Identity(&owner)`] — blocking is the acting identity's
/// own decision, so the marker must not bleed across identities that share a
/// wallet. Key shape: `det:dashpay:blocked:<counterparty_b58>`.
const KV_PREFIX_BLOCKED: &str = "det:dashpay:blocked:";
/// Mark a contact request as rejected. Value: empty (`()`). Presence is the
/// signal. Scope: [`DetScope::Identity(&owner)`] — rejection is the acting
/// identity's own decision and must not bleed across identities. Key shape:
/// `det:dashpay:rejected:<counterparty_b58>`.
const KV_PREFIX_REJECTED: &str = "det:dashpay:rejected:";
/// DET-local `(created_at, updated_at)` timestamps for an entity (contact, request).
/// Value: `(i64, i64)` encoded by the [`DetKv`] schema. Scope: [`DetScope::Global`].
const KV_PREFIX_TIMESTAMPS: &str = "det:dashpay:timestamps:";
/// DET-local private memo for a contact (nickname / notes / hidden).
/// Value: bincode-encoded [`ContactPrivateInfo`].
/// Scope: [`DetScope::Identity(&owner)`]. Key shape: `det:dashpay:private:<contact_b58>`.
const KV_PREFIX_PRIVATE: &str = "det:dashpay:private:";
/// Per-contact address index state (DIP-0015 send/receive cursors + bloom
/// registered count). Value: bincode-encoded [`ContactAddressIndex`].
/// Scope: [`DetScope::Identity(&owner)`]. Key shape: `det:dashpay:address_index:<contact_b58>`.
const KV_PREFIX_ADDRESS_INDEX: &str = "det:dashpay:address_index:";
/// Reverse lookup from a wallet address back to the `(owner, contact)`
/// relationship that derived it. Value: bincode-encoded contact
/// [`Identifier`] (the owner is already embedded in the key).
/// Key shape: `det:dashpay:addr_map:<owner_b58>:<address>`.
const KV_PREFIX_ADDR_MAP: &str = "det:dashpay:addr_map:";

/// Contact-request expiry threshold. A pending outgoing request older
/// than this is surfaced as `"expired"` rather than `"pending"`. DET
/// has no protocol-level expiry — this is purely a UX gate so the
/// outbox doesn't accumulate stale requests forever.
const DASHPAY_REQUEST_EXPIRY_DAYS: i64 = 7;

// ---------------------------------------------------------------------------
// Public view
// ---------------------------------------------------------------------------

/// Read-only view onto the upstream DashPay state, expressed in
/// DET-side `Stored*` shapes.
///
/// Borrows the [`WalletBackend`] so its callers can hand a `DashpayView`
/// to existing code without taking ownership.
#[derive(Clone)]
pub struct DashpayView<'a> {
    backend: &'a WalletBackend,
}

impl<'a> DashpayView<'a> {
    pub(super) fn new(backend: &'a WalletBackend) -> Self {
        Self { backend }
    }

    /// Resolve the `ManagedIdentity` for `owner` in whichever registered
    /// wallet manages it, and hand it — together with the DET k/v sidecar — to
    /// `f`. Returns `None` when no registered wallet manages `owner` (unknown
    /// or wrong-network identity), so each caller maps that to its own empty
    /// default. The wallet-state read guard is held for the closure's
    /// duration, so `f` must stay synchronous (pure translation only).
    async fn with_managed<R>(
        &self,
        owner: &Identifier,
        f: impl FnOnce(&platform_wallet::wallet::identity::state::ManagedIdentity, &DetKv) -> R,
    ) -> Option<R> {
        let wallet = self.backend.find_wallet_for_identity(owner).await?;
        let kv = self.backend.kv();
        let state = wallet.state().await;
        let managed = state.identity_manager.managed_identity(owner)?;
        Some(f(managed, &kv))
    }

    /// All contacts for `owner` — established (`accepted`), outstanding
    /// outgoing (`pending`), and DET-local sidecar (`blocked`).
    ///
    /// Returns an empty vector when `owner` is unknown to upstream.
    pub async fn contacts(&self, owner: &Identifier) -> Vec<StoredContact> {
        self.with_managed(owner, |managed, kv| {
            let dashpay = managed.dashpay();
            let mut out: Vec<StoredContact> = Vec::new();

            // 1. Established (`accepted`) contacts.
            for contact in dashpay.established_contacts().values() {
                let contact_id = &contact.contact_identity_id;
                let blocked = kv_contains(kv, owner, KV_PREFIX_BLOCKED, contact_id);
                let status = if blocked {
                    ContactStatus::Blocked
                } else {
                    ContactStatus::Accepted
                };
                let (created_at, updated_at) = kv_timestamps(kv, contact_id);
                out.push(established_to_det(
                    owner, contact, status, created_at, updated_at,
                ));
            }

            // 2. Sent-but-not-yet-reciprocated outgoing requests → `pending` contacts.
            //    Skip recipients we already have an established row for above.
            for recipient_id in dashpay.sent_contact_requests().keys() {
                if dashpay.established_contacts().contains_key(recipient_id) {
                    continue;
                }
                let blocked = kv_contains(kv, owner, KV_PREFIX_BLOCKED, recipient_id);
                let status = if blocked {
                    ContactStatus::Blocked
                } else {
                    ContactStatus::Pending
                };
                let (created_at, updated_at) = kv_timestamps(kv, recipient_id);
                out.push(request_to_det_contact(
                    owner,
                    recipient_id,
                    status,
                    created_at,
                    updated_at,
                ));
            }

            out
        })
        .await
        .unwrap_or_default()
    }

    /// Outstanding contact requests for `owner` — sent (outgoing, status
    /// derived from upstream presence + sidecar) and received (incoming,
    /// status derived likewise).
    ///
    /// Returns an empty vector when `owner` is unknown to upstream.
    pub async fn contact_requests(&self, owner: &Identifier) -> Vec<StoredContactRequest> {
        self.with_managed(owner, |managed, kv| {
            let dashpay = managed.dashpay();
            let mut out: Vec<StoredContactRequest> = Vec::new();

            let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;

            // Outgoing requests (`request_type = "sent"`).
            for (recipient_id, request) in dashpay.sent_contact_requests().iter() {
                let status = derive_request_status(
                    owner,
                    /* request_id_for_sidecar = */ recipient_id,
                    /* has_matching_established = */
                    dashpay.established_contacts().contains_key(recipient_id),
                    request.created_at,
                    now_ms,
                    kv,
                );
                out.push(request_to_det_request(
                    owner,
                    recipient_id,
                    request,
                    ContactRequestDirection::Sent,
                    status,
                ));
            }

            // Incoming requests (`request_type = "received"`).
            for (sender_id, request) in dashpay.incoming_contact_requests().iter() {
                let status = derive_request_status(
                    owner,
                    sender_id,
                    dashpay.established_contacts().contains_key(sender_id),
                    request.created_at,
                    now_ms,
                    kv,
                );
                out.push(request_to_det_request(
                    owner,
                    sender_id,
                    request,
                    ContactRequestDirection::Received,
                    status,
                ));
            }

            out
        })
        .await
        .unwrap_or_default()
    }

    /// Payment history for `owner`, newest entries first. Returns an
    /// empty vector when `owner` is unknown to upstream.
    pub async fn payments(&self, owner: &Identifier) -> Vec<StoredPayment> {
        self.with_managed(owner, |managed, kv| {
            let mut out: Vec<StoredPayment> = managed
                .dashpay()
                .payments
                .iter()
                .map(|(storage_key, entry)| payment_to_det(owner, storage_key, entry, kv))
                .collect();
            // Upstream stores payments keyed by the storage key in a BTreeMap
            // (lexicographic order). DET's UI sorts by `created_at DESC`; since
            // sidecar timestamps default to 0 when unset, fall back to that
            // ordering — newest first when timestamps exist, otherwise stable on
            // the storage key.
            out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            out
        })
        .await
        .unwrap_or_default()
    }

    /// DashPay profile for `owner`, or `None` when upstream has none
    /// (either `owner` is unknown, or its identity bucket has no
    /// `DashPayProfile` yet).
    pub async fn profile(&self, owner: &Identifier) -> Option<StoredProfile> {
        self.with_managed(owner, |managed, kv| {
            let profile = managed.dashpay().profile.as_ref()?;
            let (created_at, updated_at) = kv_timestamps(kv, owner);
            Some(profile_to_det(owner, profile, created_at, updated_at))
        })
        .await
        .flatten()
    }
}

// ---------------------------------------------------------------------------
// Pure translators — unit-tested without an upstream backend.
// ---------------------------------------------------------------------------

fn established_to_det(
    owner: &Identifier,
    contact: &EstablishedContact,
    status: ContactStatus,
    created_at: i64,
    updated_at: i64,
) -> StoredContact {
    StoredContact {
        owner_identity_id: owner.to_buffer().to_vec(),
        contact_identity_id: contact.contact_identity_id.to_buffer().to_vec(),
        // Username / display_name / avatar_url / public_message come
        // from DPNS + the contact's `DashPayProfile`, neither of which
        // is reachable from `EstablishedContact` alone. D2 wires the
        // cross-reads; D1 leaves them as `None`.
        username: None,
        display_name: contact.alias.clone(),
        avatar_url: None,
        public_message: contact.note.clone(),
        contact_status: status,
        created_at,
        updated_at,
        last_seen: None,
    }
}

fn request_to_det_contact(
    owner: &Identifier,
    counterparty: &Identifier,
    status: ContactStatus,
    created_at: i64,
    updated_at: i64,
) -> StoredContact {
    StoredContact {
        owner_identity_id: owner.to_buffer().to_vec(),
        contact_identity_id: counterparty.to_buffer().to_vec(),
        username: None,
        display_name: None,
        avatar_url: None,
        public_message: None,
        contact_status: status,
        created_at,
        updated_at,
        last_seen: None,
    }
}

fn request_to_det_request(
    owner: &Identifier,
    counterparty: &Identifier,
    request: &ContactRequest,
    request_type: ContactRequestDirection,
    status: ContactRequestStatus,
) -> StoredContactRequest {
    let (from_id, to_id) = if request_type == ContactRequestDirection::Sent {
        (owner, counterparty)
    } else {
        (counterparty, owner)
    };
    StoredContactRequest {
        // No autoincrement id at the upstream layer — DET's `id` column
        // was a SQLite PK, not part of the protocol. D2 callers that
        // need a stable handle should key on `(from, to)` instead.
        id: 0,
        from_identity_id: from_id.to_buffer().to_vec(),
        to_identity_id: to_id.to_buffer().to_vec(),
        // DPNS join lives outside this adapter (D2).
        to_username: None,
        // No `account_label` on the upstream model; the contract field
        // is encrypted (`encrypted_account_label: Option<Vec<u8>>`) and
        // surfacing it would leak ciphertext into a UX-facing string.
        account_label: None,
        request_type,
        status,
        // Upstream provides `created_at` directly — no sidecar read needed.
        created_at: request.created_at as i64,
        responded_at: None,
        // DET-side UX expiry: `created_at` plus the threshold the status
        // derivation uses (both in milliseconds). `None` only if the sum
        // would overflow `i64`, which a real timestamp never reaches.
        expires_at: request_expires_at_ms(request.created_at),
    }
}

fn payment_to_det(
    owner: &Identifier,
    storage_key: &str,
    entry: &PaymentEntry,
    kv: &DetKv,
) -> StoredPayment {
    use crate::model::dashpay::payment_txid_from_storage_key;

    let (from_id, to_id, payment_type) = match entry.direction {
        PaymentDirection::Sent => (owner, &entry.counterparty_id, DetPaymentDirection::Sent),
        PaymentDirection::Received => {
            (&entry.counterparty_id, owner, DetPaymentDirection::Received)
        }
    };
    let status = match entry.status {
        PaymentStatus::Pending => DetPaymentStatus::Pending,
        PaymentStatus::Confirmed => DetPaymentStatus::Confirmed,
        PaymentStatus::Failed => DetPaymentStatus::Failed,
    };
    // The upstream map key is `(txid, vout)` for incoming payments (a single
    // tx can pay two contact outputs). Timestamps are keyed by the same
    // composite key so each output keeps its own; the surfaced `tx_id` is the
    // bare transaction id, which is what the UI displays and copies.
    let (created_at, confirmed_at) = kv_payment_timestamps(kv, storage_key);
    StoredPayment {
        id: 0,
        tx_id: payment_txid_from_storage_key(storage_key).to_string(),
        from_identity_id: from_id.to_buffer().to_vec(),
        to_identity_id: to_id.to_buffer().to_vec(),
        amount: entry.amount_duffs as i64,
        memo: entry.memo.clone(),
        payment_type,
        status,
        created_at,
        confirmed_at,
    }
}

fn profile_to_det(
    owner: &Identifier,
    profile: &DashPayProfile,
    created_at: i64,
    updated_at: i64,
) -> StoredProfile {
    StoredProfile {
        identity_id: owner.to_buffer().to_vec(),
        display_name: profile.display_name.clone(),
        bio: profile.bio.clone(),
        avatar_url: profile.avatar_url.clone(),
        avatar_hash: profile.avatar_hash.map(|h| h.to_vec()),
        avatar_fingerprint: profile.avatar_fingerprint.map(|f| f.to_vec()),
        // Raw avatar bytes are intentionally never on the upstream
        // model (DIP-15: only the hash + fingerprint survive). DET's
        // avatar_bytes column is post-fetch cache — outside this seam.
        avatar_bytes: None,
        public_message: profile.public_message.clone(),
        created_at,
        updated_at,
    }
}

/// Derive a contact-request status from upstream presence + sidecar.
///
/// Precedence: `accepted` > `rejected` > `expired` > `pending`. A
/// pending request older than [`DASHPAY_REQUEST_EXPIRY_DAYS`] (per
/// `created_at_ms` vs `now_ms`) reports as `"expired"`. The `rejected`
/// marker is read under `owner`'s Identity scope so one identity's rejection
/// never colours another identity's view of the same counterparty.
fn derive_request_status(
    owner: &Identifier,
    counterparty: &Identifier,
    has_matching_established: bool,
    created_at_ms: u64,
    now_ms: u64,
    kv: &DetKv,
) -> ContactRequestStatus {
    if has_matching_established {
        return ContactRequestStatus::Accepted;
    }
    if kv_contains(kv, owner, KV_PREFIX_REJECTED, counterparty) {
        return ContactRequestStatus::Rejected;
    }
    let age_ms = now_ms.saturating_sub(created_at_ms);
    if age_ms > request_expiry_threshold_ms() {
        return ContactRequestStatus::Expired;
    }
    ContactRequestStatus::Pending
}

/// The [`DASHPAY_REQUEST_EXPIRY_DAYS`] window expressed in milliseconds, the
/// unit upstream `created_at` timestamps use.
fn request_expiry_threshold_ms() -> u64 {
    (DASHPAY_REQUEST_EXPIRY_DAYS as u64).saturating_mul(86_400_000)
}

/// Wall-clock expiry for a contact request: `created_at` plus the UX expiry
/// window, both in milliseconds. `None` only if the sum overflows `i64`.
fn request_expires_at_ms(created_at_ms: u64) -> Option<i64> {
    created_at_ms
        .checked_add(request_expiry_threshold_ms())
        .and_then(|ms| i64::try_from(ms).ok())
}

// ---------------------------------------------------------------------------
// Internal: serialized send-index increment
// ---------------------------------------------------------------------------

/// Atomically read-then-increment the persisted `next_send_index` for
/// `(owner, contact)` against `kv`. `lock` serializes the read-modify-write
/// window across the process. Returns the index value the caller should use
/// for the next outgoing payment; the persisted counter is advanced to
/// `returned_value + 1` before returning.
///
/// Split out from [`WalletBackend::dashpay_increment_send_index`] so the
/// concurrency test can exercise the locking discipline without standing up
/// a full backend (which requires SDK + SQLite persister).
///
/// # Panics
///
/// Panics if `lock` is poisoned — same contract as the public wrapper.
pub(super) fn increment_send_index_locked(
    lock: &std::sync::Mutex<()>,
    kv: &DetKv,
    owner: &Identifier,
    contact: &Identifier,
) -> Result<u32, TaskError> {
    let _guard = lock.lock().expect("dashpay address-index mutex poisoned");

    let owner_buf = owner.to_buffer();
    let scope = DetScope::Identity(&owner_buf);
    let key = sidecar_key(KV_PREFIX_ADDRESS_INDEX, contact);
    let mut state: ContactAddressIndex = kv
        .get::<ContactAddressIndex>(scope, &key)
        .map_err(|e| TaskError::DashpaySidecarStorage { source: e })?
        .unwrap_or_else(|| ContactAddressIndex {
            owner_identity_id: owner.to_buffer().to_vec(),
            contact_identity_id: contact.to_buffer().to_vec(),
            next_send_index: 0,
            highest_receive_index: 0,
            bloom_registered_count: 0,
        });
    let value = state.next_send_index;
    state.next_send_index = state.next_send_index.saturating_add(1);
    kv.put::<ContactAddressIndex>(scope, &key, &state)
        .map_err(|e| TaskError::DashpaySidecarStorage { source: e })?;
    Ok(value)
}

// ---------------------------------------------------------------------------
// K/V sidecar helpers
// ---------------------------------------------------------------------------

/// Build a sidecar key as `<prefix><id_b58>`. The scope
/// ([`DetScope::Global`] vs [`DetScope::Identity`]) is applied by the caller,
/// not encoded here — a Global-scoped key carries the full entity id while an
/// Identity-scoped key carries only the counterparty id (the owner rides on
/// the scope), but in both cases the key body is identical, so one builder
/// serves both.
fn sidecar_key(prefix: &str, id: &Identifier) -> String {
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    format!("{prefix}{}", id.to_string(Encoding::Base58))
}

/// Sidecar key for a `(owner, address)` reverse lookup. `address` is the
/// plain `Address::to_string()` form — the network's address-version byte
/// is already encoded into the string so no extra prefix is needed.
fn addr_map_sidecar_key(owner: &Identifier, address: &str) -> String {
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    format!(
        "{KV_PREFIX_ADDR_MAP}{}:{address}",
        owner.to_string(Encoding::Base58)
    )
}

/// Test the presence of an owner-scoped marker (blocked / rejected) for a
/// counterparty. The marker lives under `owner`'s [`DetScope::Identity`] so it
/// never bleeds across identities that share a wallet.
fn kv_contains(kv: &DetKv, owner: &Identifier, prefix: &str, counterparty: &Identifier) -> bool {
    // Presence-only entries: value is `()`. `Ok(Some(_))` ⇒ present.
    let owner_buf = owner.to_buffer();
    matches!(
        kv.get::<()>(
            DetScope::Identity(&owner_buf),
            &sidecar_key(prefix, counterparty)
        ),
        Ok(Some(()))
    )
}

fn kv_timestamps(kv: &DetKv, id: &Identifier) -> (i64, i64) {
    let key = sidecar_key(KV_PREFIX_TIMESTAMPS, id);
    kv_get_or_default(kv, DetScope::Global, &key, "dashpay_contact_timestamps")
}

fn kv_payment_timestamps(kv: &DetKv, tx_id: &str) -> (i64, Option<i64>) {
    let key = format!("{KV_PREFIX_TIMESTAMPS}tx:{tx_id}");
    kv_get_or_default(kv, DetScope::Global, &key, "dashpay_payment_timestamps")
}

// ---------------------------------------------------------------------------
// WalletBackend integration
// ---------------------------------------------------------------------------

impl WalletBackend {
    /// Read-only DashPay accessor. Cheap to construct (borrow only).
    pub fn dashpay_view(&self) -> DashpayView<'_> {
        DashpayView::new(self)
    }

    /// Trigger an upstream DashPay refresh (contact requests + profiles)
    /// for the wallet that owns `owner`. Callers invoke this BEFORE a
    /// read on user-initiated refresh actions so the [`DashpayView`]
    /// observes fresh state.
    ///
    /// Returns `Ok(())` if the owner is not known to any registered
    /// wallet — passive screen loads that pre-empt sync would otherwise
    /// fail noisily on cold start before any wallet is wired.
    pub async fn dashpay_sync(
        &self,
        owner: &Identifier,
    ) -> Result<(), crate::backend_task::error::TaskError> {
        let Some(wallet) = self.find_wallet_for_identity(owner).await else {
            tracing::debug!(
                owner = %owner.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                "WalletBackend::dashpay_sync: no managing wallet found; skipping"
            );
            return Ok(());
        };
        let identity = wallet.identity();
        let dashpay = identity.dashpay();
        dashpay.sync_contact_requests().await.map_err(|e| {
            crate::backend_task::error::TaskError::WalletBackend {
                source: Box::new(e),
            }
        })?;
        dashpay.sync_profiles().await.map_err(|e| {
            crate::backend_task::error::TaskError::WalletBackend {
                source: Box::new(e),
            }
        })?;
        Ok(())
    }

    /// Persist a DashPay profile against the upstream `ManagedIdentity` for
    /// `owner`, persisting the resulting changeset immediately. Pass `None`
    /// to clear the profile.
    ///
    /// Returns `Ok(())` when no registered wallet manages `owner` — the
    /// caller is operating on an out-of-wallet identity (e.g. observed
    /// profile) and there is nothing to mirror locally.
    pub async fn dashpay_set_profile(
        &self,
        owner: &Identifier,
        profile: Option<DashPayProfile>,
    ) -> Result<(), TaskError> {
        let Some(wallet) = self.find_wallet_for_identity(owner).await else {
            tracing::debug!(
                owner = %owner.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                "WalletBackend::dashpay_set_profile: no managing wallet found; skipping"
            );
            return Ok(());
        };
        let persister = wallet.persister().clone();
        let mut state = wallet.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(owner) else {
            return Ok(());
        };
        managed.set_dashpay_profile(profile, &persister);
        Ok(())
    }

    /// Record a DashPay payment entry against the upstream `ManagedIdentity`
    /// for `owner`. Upstream stores payments keyed by `tx_id` with
    /// last-write-wins semantics, so this method is also the correct way
    /// to update a payment's status (e.g. `Pending` → `Confirmed`).
    ///
    /// Returns `Ok(())` when no registered wallet manages `owner`.
    pub async fn dashpay_record_payment(
        &self,
        owner: &Identifier,
        tx_id: String,
        entry: PaymentEntry,
    ) -> Result<(), TaskError> {
        let Some(wallet) = self.find_wallet_for_identity(owner).await else {
            tracing::debug!(
                owner = %owner.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                "WalletBackend::dashpay_record_payment: no managing wallet found; skipping"
            );
            return Ok(());
        };
        let persister = wallet.persister().clone();
        let mut state = wallet.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(owner) else {
            return Ok(());
        };
        managed
            .record_dashpay_payment(tx_id, entry, &persister)
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e.into()),
            })?;
        Ok(())
    }

    /// Toggle the DET-local "blocked" marker for `contact_id`, scoped to the
    /// acting `owner` identity. The marker has no upstream counterpart —
    /// DashPay does not block on-chain — so it lives entirely in the per-owner
    /// sidecar that [`DashpayView`] reads at view time. Owner-scoping keeps one
    /// identity's block list from colouring another's view of the same contact.
    pub fn dashpay_mark_blocked(
        &self,
        owner: &Identifier,
        contact_id: &Identifier,
    ) -> Result<(), TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_BLOCKED, contact_id);
        self.kv()
            .put::<()>(DetScope::Identity(&owner_buf), &key, &())
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Clear the DET-local "blocked" marker for `contact_id` under `owner`'s
    /// scope. Idempotent — clearing an absent marker is `Ok(())`.
    pub fn dashpay_unmark_blocked(
        &self,
        owner: &Identifier,
        contact_id: &Identifier,
    ) -> Result<(), TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_BLOCKED, contact_id);
        self.kv()
            .delete(DetScope::Identity(&owner_buf), &key)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Record that `owner` has rejected an incoming contact request from
    /// `counterparty_id` (or, equivalently, withdrew the sent request from
    /// their point of view). Scoped to `owner`'s Identity so the rejection is
    /// private to that identity. The sidecar key matches what [`DashpayView`]
    /// consults when deriving request status.
    pub fn dashpay_mark_rejected(
        &self,
        owner: &Identifier,
        counterparty_id: &Identifier,
    ) -> Result<(), TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_REJECTED, counterparty_id);
        self.kv()
            .put::<()>(DetScope::Identity(&owner_buf), &key, &())
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Whether `owner` has resolved the contact request from `counterparty_id`
    /// locally — declined an incoming one, or withdrawn a sent one. Reads the
    /// same owner-scoped marker [`dashpay_mark_rejected`](Self::dashpay_mark_rejected)
    /// writes and [`DashpayView`] derives its status from.
    ///
    /// A `contactRequest` document is immutable and undeletable on Platform, so
    /// this marker is the only thing that can retire a resolved request from a
    /// listing.
    pub fn dashpay_is_rejected(&self, owner: &Identifier, counterparty_id: &Identifier) -> bool {
        kv_contains(&self.kv(), owner, KV_PREFIX_REJECTED, counterparty_id)
    }

    /// Write DET-local `(created_at_ms, updated_at_ms)` timestamps for an
    /// entity (contact, request, profile owner) into the k/v sidecar. These
    /// timestamps surface verbatim through the [`DashpayView`] adapter.
    pub fn dashpay_set_timestamps(
        &self,
        entity_id: &Identifier,
        created_at: i64,
        updated_at: i64,
    ) -> Result<(), TaskError> {
        let key = sidecar_key(KV_PREFIX_TIMESTAMPS, entity_id);
        self.kv()
            .put::<(i64, i64)>(DetScope::Global, &key, &(created_at, updated_at))
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Write DET-local `(created_at_ms, confirmed_at_ms)` timestamps for a
    /// payment in the k/v sidecar, keyed by transaction id. Upstream
    /// `PaymentEntry` carries no timestamps of its own, so this is the
    /// authoritative source consulted by [`DashpayView::payments`].
    pub fn dashpay_set_payment_timestamps(
        &self,
        tx_id: &str,
        created_at: i64,
        confirmed_at: Option<i64>,
    ) -> Result<(), TaskError> {
        let key = format!("{KV_PREFIX_TIMESTAMPS}tx:{tx_id}");
        self.kv()
            .put::<(i64, Option<i64>)>(DetScope::Global, &key, &(created_at, confirmed_at))
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Read the DET-local private memo for `(owner, contact)`. Returns
    /// `Ok(None)` when no memo has been written yet — callers should
    /// treat that as an empty memo, not as an error.
    pub fn dashpay_get_private_info(
        &self,
        owner: &Identifier,
        contact: &Identifier,
    ) -> Result<Option<ContactPrivateInfo>, TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_PRIVATE, contact);
        self.kv()
            .get::<ContactPrivateInfo>(DetScope::Identity(&owner_buf), &key)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Upsert the DET-local private memo for `(owner, contact)`.
    pub fn dashpay_set_private_info(
        &self,
        owner: &Identifier,
        contact: &Identifier,
        info: &ContactPrivateInfo,
    ) -> Result<(), TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_PRIVATE, contact);
        self.kv()
            .put::<ContactPrivateInfo>(DetScope::Identity(&owner_buf), &key, info)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Read the persisted address-index state for `(owner, contact)`.
    /// Missing keys yield `Ok(None)` — callers treat that as "no payments
    /// exchanged yet" and start from index 0.
    pub fn dashpay_get_address_index(
        &self,
        owner: &Identifier,
        contact: &Identifier,
    ) -> Result<Option<ContactAddressIndex>, TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_ADDRESS_INDEX, contact);
        self.kv()
            .get::<ContactAddressIndex>(DetScope::Identity(&owner_buf), &key)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Upsert the address-index state for `(owner, contact)`. This is the
    /// non-incrementing setter — used by the receive-side path that knows
    /// the new `highest_receive_index` or `bloom_registered_count`
    /// outright. Concurrent writers race; callers that need atomic
    /// read-modify-write (the send-side increment) must use
    /// [`Self::dashpay_increment_send_index`] instead.
    pub fn dashpay_set_address_index(
        &self,
        owner: &Identifier,
        contact: &Identifier,
        index: &ContactAddressIndex,
    ) -> Result<(), TaskError> {
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_ADDRESS_INDEX, contact);
        self.kv()
            .put::<ContactAddressIndex>(DetScope::Identity(&owner_buf), &key, index)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Atomically read-then-increment the `next_send_index` for
    /// `(owner, contact)`. Returns the index value the caller should use
    /// when deriving the next outgoing payment address; the persisted
    /// counter is advanced to `returned_value + 1` before returning.
    ///
    /// Serializes across the process via a backend-internal mutex, so
    /// concurrent send dispatches never hand out the same index.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned. The mutex protects a
    /// `()` payload; poisoning implies a prior panic mid-increment, which
    /// is a programming error worth surfacing rather than masking.
    pub fn dashpay_increment_send_index(
        &self,
        owner: &Identifier,
        contact: &Identifier,
    ) -> Result<u32, TaskError> {
        increment_send_index_locked(
            &self.inner.dashpay_address_index_lock,
            &self.kv(),
            owner,
            contact,
        )
    }

    /// Resolve a wallet address back to `(contact, address_index)` — the
    /// contact whose relationship with `owner` derived it and the index
    /// at which the address was derived. `Ok(None)` means the address is
    /// unknown to the DashPay overlay — e.g. a regular receiving address,
    /// or a contact that pre-dates this sidecar.
    pub fn dashpay_get_address_mapping(
        &self,
        owner: &Identifier,
        address: &str,
    ) -> Result<Option<(Identifier, u32)>, TaskError> {
        let key = addr_map_sidecar_key(owner, address);
        let value: Option<([u8; 32], u32)> = self
            .kv()
            .get::<([u8; 32], u32)>(DetScope::Global, &key)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })?;
        Ok(value.map(|(bytes, idx)| (Identifier::from(bytes), idx)))
    }

    /// Persist the `(owner, address) → (contact, address_index)` reverse
    /// mapping used by the incoming-payment detector. `address_index` is
    /// the position the address was derived at within the contact-scoped
    /// receive xpub.
    pub fn dashpay_set_address_mapping(
        &self,
        owner: &Identifier,
        address: &str,
        contact: &Identifier,
        address_index: u32,
    ) -> Result<(), TaskError> {
        let key = addr_map_sidecar_key(owner, address);
        self.kv()
            .put::<([u8; 32], u32)>(
                DetScope::Global,
                &key,
                &(contact.to_buffer(), address_index),
            )
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Drop every Identity-scoped DashPay overlay for `owner` — the
    /// per-contact private memos, address-index cursors, and the blocked /
    /// rejected markers.
    ///
    /// The remaining Global-scoped overlays (timestamps, reverse address map)
    /// are not owner-scoped and are swept by the `det:dashpay:` Global prefix in
    /// [`crate::context::AppContext::clear_network_database`]; this method
    /// covers the overlays that live under [`DetScope::Identity`] of the owner,
    /// which that Global sweep can no longer reach.
    pub fn dashpay_clear_owner_overlays(&self, owner: &Identifier) -> Result<(), TaskError> {
        let owner_buf = owner.to_buffer();
        let scope = DetScope::Identity(&owner_buf);
        let kv = self.kv();
        for prefix in [
            KV_PREFIX_PRIVATE,
            KV_PREFIX_ADDRESS_INDEX,
            KV_PREFIX_BLOCKED,
            KV_PREFIX_REJECTED,
        ] {
            let keys = kv
                .list(scope, Some(prefix))
                .map_err(|e| TaskError::DashpaySidecarStorage { source: e })?;
            for key in keys {
                kv.delete(scope, &key)
                    .map_err(|e| TaskError::DashpaySidecarStorage { source: e })?;
            }
        }
        Ok(())
    }

    /// Locate the `PlatformWallet` whose `IdentityManager` owns `identity_id`.
    ///
    /// Scans the sync wallet cache, then probes each wallet's
    /// `identity_manager` for the id. `None` if no registered wallet
    /// knows about it (e.g. pre-registration, wrong network, observed-
    /// only identities that were never indexed).
    async fn find_wallet_for_identity(
        &self,
        identity_id: &Identifier,
    ) -> Option<Arc<PlatformWallet>> {
        let wallets: Vec<Arc<PlatformWallet>> = {
            // Snapshot the cached wallets so we don't hold the std RwLock
            // across `await` boundaries.
            let map = self.inner.wallets.read().ok()?;
            map.values().cloned().collect()
        };
        for wallet in wallets {
            let state = wallet.state().await;
            if state
                .identity_manager
                .managed_identity(identity_id)
                .is_some()
            {
                drop(state);
                return Some(wallet);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Tests — pure translators only. The wallet-resolving methods are
// covered by D2's integration tests once the read paths route through.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn id_from_byte(b: u8) -> Identifier {
        Identifier::from([b; 32])
    }

    fn mk_request(sender: u8, recipient: u8, created_at: u64) -> ContactRequest {
        ContactRequest::new(
            id_from_byte(sender),
            id_from_byte(recipient),
            0,
            0,
            0,
            vec![0u8; 96],
            100_000,
            created_at,
        )
    }

    #[test]
    fn established_translates_alias_into_display_name() {
        let owner = id_from_byte(1);
        let contact_id = id_from_byte(2);
        let mut contact =
            EstablishedContact::new(contact_id, mk_request(1, 2, 100), mk_request(2, 1, 200));
        contact.set_alias("Buddy".to_string());
        contact.set_note("Met at conf".to_string());

        let det = established_to_det(&owner, &contact, ContactStatus::Accepted, 1_000, 2_000);
        assert_eq!(det.owner_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.contact_identity_id, contact_id.to_buffer().to_vec());
        assert_eq!(det.display_name.as_deref(), Some("Buddy"));
        assert_eq!(det.public_message.as_deref(), Some("Met at conf"));
        assert_eq!(det.contact_status, ContactStatus::Accepted);
        assert_eq!(det.created_at, 1_000);
        assert_eq!(det.updated_at, 2_000);
        // Fields requiring DPNS / profile cross-read stay None in D1.
        assert!(det.username.is_none());
        assert!(det.avatar_url.is_none());
        assert!(det.last_seen.is_none());
    }

    #[test]
    fn request_translates_into_pending_contact() {
        let owner = id_from_byte(1);
        let recipient = id_from_byte(2);

        let det = request_to_det_contact(&owner, &recipient, ContactStatus::Pending, 0, 0);
        assert_eq!(det.contact_status, ContactStatus::Pending);
        assert_eq!(det.owner_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.contact_identity_id, recipient.to_buffer().to_vec());
    }

    #[test]
    fn outgoing_request_translation_preserves_direction() {
        let owner = id_from_byte(1);
        let recipient = id_from_byte(2);
        let request = mk_request(1, 2, 123);

        let det = request_to_det_request(
            &owner,
            &recipient,
            &request,
            ContactRequestDirection::Sent,
            ContactRequestStatus::Pending,
        );
        assert_eq!(det.from_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.to_identity_id, recipient.to_buffer().to_vec());
        assert_eq!(det.request_type, ContactRequestDirection::Sent);
        assert_eq!(det.status, ContactRequestStatus::Pending);
        assert_eq!(det.created_at, 123);
        // Encrypted label is never surfaced as a plaintext `account_label`.
        assert!(det.account_label.is_none());
    }

    #[test]
    fn incoming_request_translation_flips_direction() {
        let owner = id_from_byte(1);
        let sender = id_from_byte(2);
        let request = mk_request(2, 1, 456);

        let det = request_to_det_request(
            &owner,
            &sender,
            &request,
            ContactRequestDirection::Received,
            ContactRequestStatus::Pending,
        );
        assert_eq!(det.from_identity_id, sender.to_buffer().to_vec());
        assert_eq!(det.to_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.request_type, ContactRequestDirection::Received);
    }

    #[test]
    fn sent_payment_translation_uses_owner_as_sender() {
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        let entry = PaymentEntry::new_sent(counterparty, 12_345, Some("lunch".to_string()));

        let det = payment_to_det(&owner, "tx-abc", &entry, &empty_kv());
        assert_eq!(det.tx_id, "tx-abc");
        assert_eq!(det.from_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.to_identity_id, counterparty.to_buffer().to_vec());
        assert_eq!(det.payment_type, DetPaymentDirection::Sent);
        assert_eq!(det.status, DetPaymentStatus::Pending);
        assert_eq!(det.amount, 12_345);
        assert_eq!(det.memo.as_deref(), Some("lunch"));
        assert_eq!(det.created_at, 0);
        assert!(det.confirmed_at.is_none());
    }

    #[test]
    fn received_payment_translation_uses_owner_as_recipient() {
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        let entry = PaymentEntry::new_received(counterparty, 7_500, None);

        let det = payment_to_det(&owner, "tx-def", &entry, &empty_kv());
        assert_eq!(det.from_identity_id, counterparty.to_buffer().to_vec());
        assert_eq!(det.to_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.payment_type, DetPaymentDirection::Received);
        assert_eq!(det.status, DetPaymentStatus::Confirmed);
    }

    #[test]
    fn composite_storage_key_surfaces_bare_txid() {
        let owner = id_from_byte(1);
        let c0 = id_from_byte(2);
        let c1 = id_from_byte(3);
        // Two outputs of ONE transaction to two different contacts: distinct
        // storage keys, both surface the same bare txid but their own
        // counterparty + amount (no last-write-wins clobber).
        let e0 = PaymentEntry::new_received(c0, 1_000, None);
        let e1 = PaymentEntry::new_received(c1, 2_000, None);
        let d0 = payment_to_det(&owner, "txhash:0", &e0, &empty_kv());
        let d1 = payment_to_det(&owner, "txhash:1", &e1, &empty_kv());
        assert_eq!(d0.tx_id, "txhash");
        assert_eq!(d1.tx_id, "txhash");
        assert_eq!(d0.from_identity_id, c0.to_buffer().to_vec());
        assert_eq!(d1.from_identity_id, c1.to_buffer().to_vec());
        assert_eq!(d0.amount, 1_000);
        assert_eq!(d1.amount, 2_000);
    }

    #[test]
    fn profile_translation_carries_hash_and_fingerprint() {
        let owner = id_from_byte(1);
        let profile = DashPayProfile {
            display_name: Some("Alice".into()),
            bio: Some("Hello".into()),
            avatar_url: Some("https://example.com/a.png".into()),
            avatar_hash: Some([7u8; 32]),
            avatar_fingerprint: Some([3u8; 8]),
            public_message: Some("Public!".into()),
        };

        let det = profile_to_det(&owner, &profile, 11, 22);
        assert_eq!(det.identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.display_name.as_deref(), Some("Alice"));
        assert_eq!(det.bio.as_deref(), Some("Hello"));
        assert_eq!(det.avatar_url.as_deref(), Some("https://example.com/a.png"));
        assert_eq!(det.avatar_hash.as_deref(), Some(&[7u8; 32][..]));
        assert_eq!(det.avatar_fingerprint.as_deref(), Some(&[3u8; 8][..]));
        assert!(
            det.avatar_bytes.is_none(),
            "raw bytes never come through this seam"
        );
        assert_eq!(det.created_at, 11);
        assert_eq!(det.updated_at, 22);
    }

    #[test]
    fn request_status_derivation_uses_established_then_sidecar() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        // Fresh request, no expiry yet.
        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&owner, &counterparty, true, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Accepted,
            "matching established contact wins"
        );
        assert_eq!(
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Pending,
            "no established + no rejection sidecar + fresh = pending"
        );
    }

    #[test]
    fn rejected_request_status_reads_sidecar_when_present() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        let owner_buf = owner.to_buffer();
        kv.put::<()>(
            DetScope::Identity(&owner_buf),
            &sidecar_key(KV_PREFIX_REJECTED, &counterparty),
            &(),
        )
        .unwrap();
        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Rejected
        );
    }

    #[test]
    fn expired_request_status_when_older_than_threshold() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        let now_ms: u64 = 10_000_000_000_000;
        // Older than the 7-day threshold by one minute.
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        let created_at_ms: u64 = now_ms - threshold_ms - 60_000;
        assert_eq!(
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Expired,
            "older-than-threshold pending request reports as expired"
        );
    }

    #[test]
    fn fresh_request_just_under_threshold_stays_pending() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        let now_ms: u64 = 10_000_000_000_000;
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        // One minute younger than the threshold.
        let created_at_ms: u64 = now_ms - threshold_ms + 60_000;
        assert_eq!(
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Pending
        );
    }

    #[test]
    fn expires_at_is_created_at_plus_threshold() {
        let created_at_ms: u64 = 1_700_000_000_000;
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        assert_eq!(
            request_expires_at_ms(created_at_ms),
            Some((created_at_ms + threshold_ms) as i64),
            "expiry is created_at plus the 7-day threshold, in ms"
        );
    }

    #[test]
    fn expires_at_none_on_overflow() {
        assert_eq!(
            request_expires_at_ms(u64::MAX),
            None,
            "an overflowing timestamp yields no expiry rather than wrapping"
        );
    }

    #[test]
    fn blocked_contact_overrides_accepted_status() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact_id = id_from_byte(2);
        let owner_buf = owner.to_buffer();
        kv.put::<()>(
            DetScope::Identity(&owner_buf),
            &sidecar_key(KV_PREFIX_BLOCKED, &contact_id),
            &(),
        )
        .unwrap();

        let mut contact =
            EstablishedContact::new(contact_id, mk_request(1, 2, 100), mk_request(2, 1, 200));
        contact.set_alias("Friend".into());

        let status = if kv_contains(&kv, &owner, KV_PREFIX_BLOCKED, &contact_id) {
            ContactStatus::Blocked
        } else {
            ContactStatus::Accepted
        };
        let det = established_to_det(&owner, &contact, status, 0, 0);
        assert_eq!(det.contact_status, ContactStatus::Blocked);
        assert_eq!(det.display_name.as_deref(), Some("Friend"));
    }

    #[test]
    fn timestamps_default_to_zero_on_missing_sidecar() {
        let kv = empty_kv();
        let id = id_from_byte(2);
        assert_eq!(kv_timestamps(&kv, &id), (0, 0));
    }

    #[test]
    fn timestamps_round_trip_through_sidecar() {
        let kv = empty_kv();
        let id = id_from_byte(2);
        kv.put::<(i64, i64)>(
            DetScope::Global,
            &sidecar_key(KV_PREFIX_TIMESTAMPS, &id),
            &(111, 222),
        )
        .unwrap();
        assert_eq!(kv_timestamps(&kv, &id), (111, 222));
    }

    #[test]
    fn payment_timestamps_round_trip() {
        let kv = empty_kv();
        let tx_id = "tx-xyz";
        kv.put::<(i64, Option<i64>)>(
            DetScope::Global,
            &format!("{KV_PREFIX_TIMESTAMPS}tx:{tx_id}"),
            &(100, Some(200)),
        )
        .unwrap();
        assert_eq!(kv_payment_timestamps(&kv, tx_id), (100, Some(200)));
    }

    /// D3 contract: the key encoding used by the write helpers
    /// (`dashpay_mark_blocked`, `dashpay_mark_rejected`,
    /// `dashpay_set_timestamps`, `dashpay_set_payment_timestamps`) must
    /// match the encoding the read helpers (`kv_contains`,
    /// `kv_timestamps`, `kv_payment_timestamps`) consult — otherwise
    /// every write is invisible to the view.
    ///
    /// The blocked / rejected markers are owner-scoped (Wave 2 / F40): the
    /// write lands under the owner's `DetScope::Identity` keyed only on the
    /// counterparty, and `kv_contains` reads from the same place.
    #[test]
    fn d3_blocked_marker_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let owner_buf = owner.to_buffer();
        let contact = id_from_byte(7);
        let key = sidecar_key(KV_PREFIX_BLOCKED, &contact);
        // What `dashpay_mark_blocked` writes:
        kv.put::<()>(DetScope::Identity(&owner_buf), &key, &())
            .unwrap();
        // What `DashpayView::contacts` reads:
        assert!(kv_contains(&kv, &owner, KV_PREFIX_BLOCKED, &contact));

        // And `dashpay_unmark_blocked` (delete) clears it.
        kv.delete(DetScope::Identity(&owner_buf), &key).unwrap();
        assert!(!kv_contains(&kv, &owner, KV_PREFIX_BLOCKED, &contact));
    }

    #[test]
    fn d3_rejected_marker_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let owner_buf = owner.to_buffer();
        let counterparty = id_from_byte(8);
        // What `dashpay_mark_rejected` writes:
        kv.put::<()>(
            DetScope::Identity(&owner_buf),
            &sidecar_key(KV_PREFIX_REJECTED, &counterparty),
            &(),
        )
        .unwrap();
        // What `derive_request_status` reads:
        assert!(kv_contains(&kv, &owner, KV_PREFIX_REJECTED, &counterparty));

        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Rejected
        );
    }

    #[test]
    fn d3_timestamp_sidecar_round_trips() {
        let kv = empty_kv();
        let entity = id_from_byte(9);
        // What `dashpay_set_timestamps` writes:
        kv.put::<(i64, i64)>(
            DetScope::Global,
            &sidecar_key(KV_PREFIX_TIMESTAMPS, &entity),
            &(123, 456),
        )
        .unwrap();
        // What `DashpayView::contacts` reads:
        assert_eq!(kv_timestamps(&kv, &entity), (123, 456));
    }

    #[test]
    fn d3_payment_timestamp_sidecar_round_trips() {
        let kv = empty_kv();
        let tx_id = "abcd1234";
        // What `dashpay_set_payment_timestamps` writes:
        kv.put::<(i64, Option<i64>)>(
            DetScope::Global,
            &format!("{KV_PREFIX_TIMESTAMPS}tx:{tx_id}"),
            &(789, Some(1000)),
        )
        .unwrap();
        // What `DashpayView::payments` reads:
        assert_eq!(kv_payment_timestamps(&kv, tx_id), (789, Some(1000)));
    }

    #[test]
    fn d3_block_then_list_contacts_yields_blocked_status() {
        // Simulates: send → block → list. After D3 wires
        // `dashpay_mark_blocked`, the contact's status flips to
        // "blocked" without touching the upstream `EstablishedContact`
        // (DashPay has no on-chain block flag — DET sidecar is the
        // source of truth).
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let owner_buf = owner.to_buffer();
        let contact_id = id_from_byte(2);

        // Pre-state: a single established contact exists upstream.
        let mut contact =
            EstablishedContact::new(contact_id, mk_request(1, 2, 100), mk_request(2, 1, 200));
        contact.set_alias("Pal".into());

        // What `dashpay_mark_blocked(&owner, &contact_id)` writes:
        kv.put::<()>(
            DetScope::Identity(&owner_buf),
            &sidecar_key(KV_PREFIX_BLOCKED, &contact_id),
            &(),
        )
        .unwrap();

        // What the view derivation produces — same precedence as
        // `DashpayView::contacts`: blocked wins over accepted.
        let status = if kv_contains(&kv, &owner, KV_PREFIX_BLOCKED, &contact_id) {
            ContactStatus::Blocked
        } else {
            ContactStatus::Accepted
        };
        let det = established_to_det(&owner, &contact, status, 0, 0);
        assert_eq!(det.contact_status, ContactStatus::Blocked);
        assert_eq!(det.display_name.as_deref(), Some("Pal"));
    }

    #[test]
    fn d3_reject_then_list_contact_requests_yields_rejected_status() {
        // Simulates: send → reject → list. After D3 wires
        // `dashpay_mark_rejected`, the outgoing request's status flips
        // to "rejected" without touching upstream presence (rejected
        // requests are not removed from `sent_contact_requests`).
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let owner_buf = owner.to_buffer();
        let counterparty = id_from_byte(2);
        kv.put::<()>(
            DetScope::Identity(&owner_buf),
            &sidecar_key(KV_PREFIX_REJECTED, &counterparty),
            &(),
        )
        .unwrap();

        let now_ms: u64 = 2_000_000_000_000;
        let created_at_ms: u64 = now_ms - 1_000;
        let derived =
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv);
        assert_eq!(derived, ContactRequestStatus::Rejected);

        // And the threshold-expiry override does not fire for rejected
        // requests — `rejected` precedence is higher than `expired`.
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        let old_created = now_ms - threshold_ms - 60_000;
        let derived_old =
            derive_request_status(&owner, &counterparty, false, old_created, now_ms, &kv);
        assert_eq!(derived_old, ContactRequestStatus::Rejected);
    }

    #[test]
    fn d3_seven_day_old_pending_request_reports_expired() {
        // Send → wait > 7 days → list → assert expired. The DET-side
        // expiry threshold lives in `DASHPAY_REQUEST_EXPIRY_DAYS` and
        // is a UX gate; upstream stores no protocol-level expiry.
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let counterparty = id_from_byte(2);
        let now_ms: u64 = 50_000_000_000_000;
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        // 7 days + a margin of safety.
        let created_at_ms: u64 = now_ms - threshold_ms - 86_400_000;
        assert_eq!(
            derive_request_status(&owner, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Expired
        );
    }

    /// F40: two identities that share a wallet must not see each other's
    /// blocked / rejected markers. Identity A blocks/rejects a counterparty;
    /// identity B's view of that same counterparty stays clean.
    #[test]
    fn markers_are_isolated_per_owner_identity() {
        let kv = empty_kv();
        let owner_a = id_from_byte(1);
        let owner_b = id_from_byte(2);
        let counterparty = id_from_byte(3);
        let a_buf = owner_a.to_buffer();

        // Owner A blocks and rejects the counterparty.
        kv.put::<()>(
            DetScope::Identity(&a_buf),
            &sidecar_key(KV_PREFIX_BLOCKED, &counterparty),
            &(),
        )
        .unwrap();
        kv.put::<()>(
            DetScope::Identity(&a_buf),
            &sidecar_key(KV_PREFIX_REJECTED, &counterparty),
            &(),
        )
        .unwrap();

        // Owner A sees them...
        assert!(kv_contains(&kv, &owner_a, KV_PREFIX_BLOCKED, &counterparty));
        assert!(kv_contains(
            &kv,
            &owner_a,
            KV_PREFIX_REJECTED,
            &counterparty
        ));

        // ...owner B does not.
        assert!(
            !kv_contains(&kv, &owner_b, KV_PREFIX_BLOCKED, &counterparty),
            "owner B must not see owner A's blocked marker"
        );
        assert!(
            !kv_contains(&kv, &owner_b, KV_PREFIX_REJECTED, &counterparty),
            "owner B must not see owner A's rejected marker"
        );

        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&owner_a, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Rejected,
            "A's own rejection colours A's view"
        );
        assert_eq!(
            derive_request_status(&owner_b, &counterparty, false, created_at_ms, now_ms, &kv),
            ContactRequestStatus::Pending,
            "B's view of the same counterparty is unaffected"
        );
    }

    // -------------------------------------------------------------------
    // D4b: address-index + private-info k/v primitives (key shape +
    // round-trip via the same `DetKv` adapter used in production). The
    // wallet-resolving methods on `WalletBackend` itself need an
    // upstream backend and are covered by the e2e suite.
    // -------------------------------------------------------------------

    #[test]
    fn d4b_contact_sidecar_key_is_contact_only_base58() {
        // Wave 2: the owner moved into the `DetScope::Identity` scope, so
        // the per-contact key carries only the contact, not `<owner>:<contact>`.
        let contact = id_from_byte(2);
        let key = sidecar_key(KV_PREFIX_PRIVATE, &contact);
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let expected = format!(
            "det:dashpay:private:{}",
            contact.to_string(Encoding::Base58)
        );
        assert_eq!(key, expected);
    }

    #[test]
    fn d4b_addr_map_sidecar_key_carries_owner_and_address() {
        let owner = id_from_byte(1);
        let addr = "yXyqJv6gP2c8RXAhYQ7v6XwxSqUf7vXKfA";
        let key = addr_map_sidecar_key(&owner, addr);
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let expected = format!(
            "det:dashpay:addr_map:{}:{}",
            owner.to_string(Encoding::Base58),
            addr
        );
        assert_eq!(key, expected);
    }

    #[test]
    fn d4b_private_info_round_trips_in_owner_identity_scope() {
        let kv = empty_kv();
        let owner = id_from_byte(1).to_buffer();
        let contact = id_from_byte(2);
        let info = ContactPrivateInfo {
            nickname: "Alice".into(),
            notes: "met at conf".into(),
            is_hidden: true,
        };
        let key = sidecar_key(KV_PREFIX_PRIVATE, &contact);
        let scope = DetScope::Identity(&owner);
        kv.put::<ContactPrivateInfo>(scope, &key, &info).unwrap();
        let got: ContactPrivateInfo = kv
            .get::<ContactPrivateInfo>(scope, &key)
            .unwrap()
            .expect("written value should round-trip");
        assert_eq!(got, info);
        // Invisible from the Global scope it used to live in.
        assert!(
            kv.get::<ContactPrivateInfo>(DetScope::Global, &key)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn d4b_private_info_missing_key_returns_none() {
        let kv = empty_kv();
        let owner = id_from_byte(1).to_buffer();
        let contact = id_from_byte(2);
        let key = sidecar_key(KV_PREFIX_PRIVATE, &contact);
        assert!(
            kv.get::<ContactPrivateInfo>(DetScope::Identity(&owner), &key)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn d4b_address_index_round_trips_in_owner_identity_scope() {
        let kv = empty_kv();
        let owner_id = id_from_byte(1);
        let owner = owner_id.to_buffer();
        let contact = id_from_byte(2);
        let idx = ContactAddressIndex {
            owner_identity_id: owner_id.to_buffer().to_vec(),
            contact_identity_id: contact.to_buffer().to_vec(),
            next_send_index: 7,
            highest_receive_index: 3,
            bloom_registered_count: 20,
        };
        let key = sidecar_key(KV_PREFIX_ADDRESS_INDEX, &contact);
        let scope = DetScope::Identity(&owner);
        kv.put::<ContactAddressIndex>(scope, &key, &idx).unwrap();
        let got = kv
            .get::<ContactAddressIndex>(scope, &key)
            .unwrap()
            .expect("written value should round-trip");
        assert_eq!(got.next_send_index, 7);
        assert_eq!(got.highest_receive_index, 3);
        assert_eq!(got.bloom_registered_count, 20);
    }

    #[test]
    fn d4b_private_info_is_isolated_per_owner_scope() {
        // The owner used to be encoded in the key; now it is the scope.
        // Two owners sharing a contact must not see each other's memo.
        let kv = empty_kv();
        let owner_a = id_from_byte(1).to_buffer();
        let owner_b = id_from_byte(2).to_buffer();
        let contact = id_from_byte(3);
        let key = sidecar_key(KV_PREFIX_PRIVATE, &contact);
        kv.put::<ContactPrivateInfo>(
            DetScope::Identity(&owner_a),
            &key,
            &ContactPrivateInfo {
                nickname: "for-a".into(),
                notes: String::new(),
                is_hidden: false,
            },
        )
        .unwrap();
        assert!(
            kv.get::<ContactPrivateInfo>(DetScope::Identity(&owner_b), &key)
                .unwrap()
                .is_none(),
            "owner B must not see owner A's per-contact memo"
        );
    }

    #[test]
    fn d4b_address_mapping_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);
        let addr = "yXyqJv6gP2c8RXAhYQ7v6XwxSqUf7vXKfA";
        let key = addr_map_sidecar_key(&owner, addr);
        // D4c extended the value schema to carry the address_index alongside
        // the contact id, so the incoming-payment detector can promote the
        // receive cursor without a separate lookup.
        kv.put::<([u8; 32], u32)>(DetScope::Global, &key, &(contact.to_buffer(), 7))
            .unwrap();
        let got: Option<([u8; 32], u32)> =
            kv.get::<([u8; 32], u32)>(DetScope::Global, &key).unwrap();
        assert_eq!(got, Some((contact.to_buffer(), 7)));
    }

    #[test]
    fn d4b_contact_key_distinguishes_contacts() {
        // The owner now lives in the scope; the key only distinguishes
        // contacts within one owner's Identity scope.
        let a = id_from_byte(1);
        let b = id_from_byte(2);
        let key_a = sidecar_key(KV_PREFIX_PRIVATE, &a);
        let key_b = sidecar_key(KV_PREFIX_PRIVATE, &b);
        assert_ne!(key_a, key_b, "distinct contacts must yield distinct keys");
    }

    // -------------------------------------------------------------------
    // D4c: concurrency contract for `dashpay_increment_send_index`.
    // -------------------------------------------------------------------

    /// 100 concurrent increment calls against the same `(owner, contact)`
    /// must hand out exactly the values `0..=99`, no duplicates, and leave
    /// the persisted counter at `100`. Exercises the same locking
    /// discipline the public `WalletBackend::dashpay_increment_send_index`
    /// relies on, via the extracted [`increment_send_index_locked`] helper —
    /// constructing a real backend would pull in SDK + persister which the
    /// unit-test target deliberately avoids.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn d4c_concurrent_send_index_increments_yield_unique_values() {
        let kv = empty_kv();
        let lock = Arc::new(std::sync::Mutex::new(()));
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);

        // Wrap kv in Arc so each task can move a clone of the handle.
        let kv = Arc::new(kv);

        let mut handles = Vec::with_capacity(100);
        for _ in 0..100 {
            let lock = Arc::clone(&lock);
            let kv = Arc::clone(&kv);
            handles.push(tokio::spawn(async move {
                increment_send_index_locked(&lock, &kv, &owner, &contact)
                    .expect("increment must succeed")
            }));
        }

        let mut values: Vec<u32> = Vec::with_capacity(100);
        for h in handles {
            values.push(h.await.expect("task panicked"));
        }
        values.sort_unstable();

        let expected: Vec<u32> = (0..100).collect();
        assert_eq!(
            values, expected,
            "100 concurrent increments must return distinct values 0..=99"
        );

        // Final persisted counter advances to 100, read back from the
        // owner's Identity scope where the increment helper now writes.
        let owner_buf = owner.to_buffer();
        let key = sidecar_key(KV_PREFIX_ADDRESS_INDEX, &contact);
        let final_state: ContactAddressIndex = kv
            .get::<ContactAddressIndex>(DetScope::Identity(&owner_buf), &key)
            .expect("kv read")
            .expect("counter must have been initialized");
        assert_eq!(final_state.next_send_index, 100);
    }

    // -------------------------------------------------------------------
    // D4d / Wave 2: the network-clear path
    // (`AppContext::clear_network_database`) must hit every overlay the
    // adapter writes. Wave 2 split the overlays across two scopes: the
    // Global-scoped markers/timestamps/addr-map come out in one
    // `det:dashpay:` prefix sweep; the per-contact private memo and
    // address-index moved to each owner's `DetScope::Identity` scope and
    // are cleared per owner. A miss in either half leaks DET-only state
    // across a "Clear network data" action.
    // -------------------------------------------------------------------

    /// D4d-Sweep1: the three Global overlays share the `det:dashpay:`
    /// prefix and come out of one Global sweep; the four Identity-scoped
    /// overlays do NOT (they live under the owner scope). Wave 2 + F40 moved
    /// the blocked / rejected markers into the owner scope alongside the
    /// private memo and address-index cursors.
    #[test]
    fn d4d_global_overlays_share_prefix_identity_overlays_do_not() {
        let kv = empty_kv();
        let owner_id = id_from_byte(1);
        let owner = owner_id.to_buffer();
        let contact = id_from_byte(2);
        let addr = "yXyqJv6gP2c8RXAhYQ7v6XwxSqUf7vXKfA";

        // Three Global overlays (timestamps, payment timestamps, addr map).
        kv.put::<(i64, i64)>(
            DetScope::Global,
            &sidecar_key(KV_PREFIX_TIMESTAMPS, &contact),
            &(111, 222),
        )
        .unwrap();
        kv.put::<(i64, Option<i64>)>(
            DetScope::Global,
            &format!("{KV_PREFIX_TIMESTAMPS}tx:abc123"),
            &(333, Some(444)),
        )
        .unwrap();
        kv.put::<([u8; 32], u32)>(
            DetScope::Global,
            &addr_map_sidecar_key(&owner_id, addr),
            &(contact.to_buffer(), 1),
        )
        .unwrap();

        // Four Identity-scoped overlays under the owner.
        kv.put::<ContactPrivateInfo>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_PRIVATE, &contact),
            &ContactPrivateInfo::default(),
        )
        .unwrap();
        kv.put::<ContactAddressIndex>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_ADDRESS_INDEX, &contact),
            &ContactAddressIndex {
                owner_identity_id: owner_id.to_buffer().to_vec(),
                contact_identity_id: contact.to_buffer().to_vec(),
                next_send_index: 1,
                highest_receive_index: 0,
                bloom_registered_count: 0,
            },
        )
        .unwrap();
        kv.put::<()>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_BLOCKED, &contact),
            &(),
        )
        .unwrap();
        kv.put::<()>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_REJECTED, &contact),
            &(),
        )
        .unwrap();

        let global = kv
            .list(DetScope::Global, Some("det:dashpay:"))
            .expect("global sidecar listing must succeed");
        assert_eq!(
            global.len(),
            3,
            "three Global overlays enumerated: {global:?}"
        );
        for k in &global {
            assert!(k.starts_with("det:dashpay:"), "non-DashPay key: {k}");
        }

        // The Identity-scoped overlays are invisible to the Global sweep
        // but present under the owner scope.
        let owned = kv
            .list(DetScope::Identity(&owner), Some("det:dashpay:"))
            .expect("owner sidecar listing must succeed");
        assert_eq!(owned.len(), 4, "four owner-scoped overlays: {owned:?}");
    }

    /// D4d-Sweep2: the combined clear (Global prefix sweep + per-owner
    /// Identity clear) drains every overlay — mirrors what
    /// `AppContext::clear_network_database` does post-Wave-2.
    #[test]
    fn d4d_combined_clear_drains_global_and_owner_overlays() {
        let kv = empty_kv();
        let owner = id_from_byte(1).to_buffer();
        let contact = id_from_byte(2);

        // A Global overlay (timestamps) plus the four owner-scoped overlays.
        kv.put::<(i64, i64)>(
            DetScope::Global,
            &sidecar_key(KV_PREFIX_TIMESTAMPS, &contact),
            &(1, 2),
        )
        .unwrap();
        kv.put::<()>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_BLOCKED, &contact),
            &(),
        )
        .unwrap();
        kv.put::<()>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_REJECTED, &contact),
            &(),
        )
        .unwrap();
        kv.put::<ContactPrivateInfo>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_PRIVATE, &contact),
            &ContactPrivateInfo {
                nickname: "alice".into(),
                notes: "n".into(),
                is_hidden: false,
            },
        )
        .unwrap();
        kv.put::<ContactAddressIndex>(
            DetScope::Identity(&owner),
            &sidecar_key(KV_PREFIX_ADDRESS_INDEX, &contact),
            &ContactAddressIndex {
                owner_identity_id: owner.to_vec(),
                contact_identity_id: contact.to_buffer().to_vec(),
                next_send_index: 3,
                highest_receive_index: 0,
                bloom_registered_count: 0,
            },
        )
        .unwrap();
        // Drop one unrelated global key to confirm the sweep is scoped.
        kv.put::<u32>(DetScope::Global, "mainnet:scheduled_votes:1", &7)
            .unwrap();

        // Global prefix sweep.
        for k in kv.list(DetScope::Global, Some("det:dashpay:")).unwrap() {
            kv.delete(DetScope::Global, &k).unwrap();
        }
        // Per-owner Identity-scope clear — mirrors `dashpay_clear_owner_overlays`.
        for prefix in [
            KV_PREFIX_PRIVATE,
            KV_PREFIX_ADDRESS_INDEX,
            KV_PREFIX_BLOCKED,
            KV_PREFIX_REJECTED,
        ] {
            for k in kv.list(DetScope::Identity(&owner), Some(prefix)).unwrap() {
                kv.delete(DetScope::Identity(&owner), &k).unwrap();
            }
        }

        assert!(
            kv.list(DetScope::Global, Some("det:dashpay:"))
                .unwrap()
                .is_empty(),
            "Global DashPay overlays must be empty after the sweep"
        );
        assert!(
            kv.list(DetScope::Identity(&owner), Some("det:dashpay:"))
                .unwrap()
                .is_empty(),
            "owner-scoped DashPay overlays must be empty after the clear"
        );
        // Unrelated key survives.
        assert_eq!(
            kv.get::<u32>(DetScope::Global, "mainnet:scheduled_votes:1")
                .unwrap(),
            Some(7)
        );
    }

    /// D4d-Sweep3: the prefix sweep is precision-scoped — keys for
    /// unrelated overlays (settings, scheduled votes, shielded sidecar,
    /// etc.) must not be caught by `det:dashpay:`.
    #[test]
    fn d4d_prefix_sweep_skips_non_dashpay_keys() {
        let kv = empty_kv();
        kv.put::<u32>(DetScope::Global, "mainnet:settings:v1", &1)
            .unwrap();
        kv.put::<u32>(DetScope::Global, "mainnet:shielded:sync_cursor", &2)
            .unwrap();
        kv.put::<u32>(DetScope::Global, "det:other_domain:x", &3)
            .unwrap();

        let keys = kv.list(DetScope::Global, Some("det:dashpay:")).unwrap();
        assert!(
            keys.is_empty(),
            "prefix sweep must not see non-DashPay overlays: {keys:?}"
        );
    }

    // -------------------------------------------------------------------
    // In-memory KvStore for the translator tests.
    // -------------------------------------------------------------------

    fn empty_kv() -> DetKv {
        use crate::wallet_backend::kv_test_support::InMemoryKv;

        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    // -----------------------------------------------------------------------
    // Contact xpub derivation seam
    // -----------------------------------------------------------------------

    const TEST_SEED: [u8; 64] = [0x42u8; 64];
    const TEST_SECRET: [u8; 32] = [0x07u8; 32];

    // F3 pinned vectors — captured from the implementation for the fixed
    // inputs `(TEST_SEED, Testnet, account_index 0, sender 0x11, recipient
    // 0x22, TEST_SECRET)`. They guard the DashPay derivation seam against a
    // silent upstream drift; update only with a deliberate review.
    const ACCOUNT_REFERENCE_TESTNET_PINNED: u32 = 173_734_745;
    const CONTACT_INFO_KEY1_TESTNET_PINNED: [u8; 32] = [
        158, 54, 59, 142, 202, 216, 7, 142, 110, 245, 93, 36, 97, 242, 74, 190, 160, 34, 128, 221,
        103, 201, 77, 14, 76, 12, 80, 209, 66, 120, 131, 231,
    ];
    const CONTACT_INFO_KEY2_TESTNET_PINNED: [u8; 32] = [
        38, 164, 234, 130, 35, 156, 42, 172, 46, 176, 30, 207, 166, 199, 230, 203, 100, 89, 48, 37,
        54, 201, 175, 48, 191, 160, 21, 137, 219, 78, 222, 242,
    ];

    fn contact_ids() -> (Identifier, Identifier) {
        (id_from_byte(0x11), id_from_byte(0x22))
    }

    /// F3 — pinned vector for `account_reference`. Locks the DIP-15
    /// account-reference HMAC against a fixed `(seed, sender, recipient,
    /// secret)` so an upstream change to `calculate_account_reference` or the
    /// xpub derivation path cannot silently shift the value contacts pay
    /// against. The expected value was captured from the implementation and
    /// pinned; a mismatch is a deliberate-review signal, not a flake.
    #[test]
    fn f3_account_reference_pinned_vector() {
        let (sender, recipient) = contact_ids();
        let m = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive");
        assert_eq!(
            m.account_reference, ACCOUNT_REFERENCE_TESTNET_PINNED,
            "account_reference drifted from its pinned vector"
        );
    }

    /// F3 — `account_reference` differs across networks for the same inputs,
    /// because the coin-type segment of the derivation path differs. Guards
    /// against a regression that would make testnet and mainnet collide.
    #[test]
    fn f3_account_reference_testnet_differs_from_mainnet() {
        let (sender, recipient) = contact_ids();
        let testnet = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("testnet");
        let mainnet = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Mainnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("mainnet");
        assert_ne!(
            testnet.account_reference, mainnet.account_reference,
            "testnet and mainnet account_reference must not collide"
        );
        assert_ne!(
            testnet.public_key, mainnet.public_key,
            "testnet and mainnet contact xpub must not collide"
        );
    }

    /// F3 — pinned vector for the DIP-0015 contactInfo encryption key pair.
    /// Locks `derive_contact_info_encryption_keys` against a fixed
    /// `(seed, network, index)` so an upstream BIP-32 change cannot silently
    /// rotate the keys that decrypt existing contact-info documents.
    #[test]
    fn f3_contact_info_encryption_keys_pinned_vector() {
        let (key1, key2) =
            derive_contact_info_encryption_keys(&TEST_SEED, Network::Testnet, 0).expect("derive");
        assert_eq!(
            *key1, CONTACT_INFO_KEY1_TESTNET_PINNED,
            "contactInfo key1 drifted from its pinned vector"
        );
        assert_eq!(
            *key2, CONTACT_INFO_KEY2_TESTNET_PINNED,
            "contactInfo key2 drifted from its pinned vector"
        );
        // The two DIP-15 keys are derived under distinct hardened branches —
        // they must never be equal.
        assert_ne!(*key1, *key2, "key1 and key2 must differ");
    }

    /// F3 — the contactInfo encryption keys differ across networks for the
    /// same `(seed, index)`, because the coin-type path segment differs.
    #[test]
    fn f3_contact_info_encryption_keys_testnet_differs_from_mainnet() {
        let (t1, t2) =
            derive_contact_info_encryption_keys(&TEST_SEED, Network::Testnet, 0).expect("testnet");
        let (m1, m2) =
            derive_contact_info_encryption_keys(&TEST_SEED, Network::Mainnet, 0).expect("mainnet");
        assert_ne!(*t1, *m1, "key1 must differ across networks");
        assert_ne!(*t2, *m2, "key2 must differ across networks");
    }

    #[test]
    fn contact_xpub_material_is_deterministic() {
        let (sender, recipient) = contact_ids();
        let a = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive");
        let b = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive");
        assert_eq!(a, b, "same inputs must yield the same material");
    }

    #[test]
    fn contact_xpub_material_differs_by_relationship() {
        let (sender, recipient) = contact_ids();
        let forward = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive forward");
        let reverse = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &recipient,
            &sender,
            &TEST_SECRET,
        )
        .expect("derive reverse");
        assert_ne!(
            forward.public_key, reverse.public_key,
            "the relationship is directional; swapping sender/recipient must change the key"
        );
    }

    #[test]
    fn contact_xpub_public_key_is_valid_compressed() {
        let (sender, recipient) = contact_ids();
        let m = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive");
        assert!(
            m.public_key[0] == 0x02 || m.public_key[0] == 0x03,
            "compressed secp256k1 public key must start with 0x02 or 0x03"
        );
    }

    /// Correctness substrate: on **mainnet**, the xpub the seam
    /// publishes in the contact request equals the xpub DET's receive-side
    /// derivation (`derive_dashpay_incoming_xpub`) produces from the same seed
    /// and path. This pins the upstream `derive_contact_xpub` output against
    /// DET's local DIP-14 path where they agree (both use coin-type 5').
    #[test]
    fn seam_xpub_matches_receive_side_derivation_on_mainnet() {
        use crate::model::dashpay_derivation::derive_dashpay_incoming_xpub;

        let (sender, recipient) = contact_ids();
        let seam = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Mainnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("seam derive");

        let local =
            derive_dashpay_incoming_xpub(&TEST_SEED, Network::Mainnet, 0, &sender, &recipient)
                .expect("local derive");

        assert_eq!(
            seam.public_key,
            local.public_key.serialize(),
            "published public key must match the receive-side xpub on mainnet"
        );
        assert_eq!(
            seam.chain_code,
            local.chain_code.to_bytes(),
            "published chain code must match the receive-side xpub on mainnet"
        );
        assert_eq!(
            seam.parent_fingerprint,
            local.parent_fingerprint.to_bytes(),
            "published parent fingerprint must match the receive-side xpub on mainnet"
        );
    }

    /// Fund-routing invariant on **testnet**: the xpub the seam
    /// publishes in the contact request equals the xpub DET's receive-side
    /// derivation (`derive_dashpay_incoming_xpub`) produces from the same seed
    /// and path. Both now select coin-type `1'` on testnet, so the contact
    /// pays into addresses the user's wallet actually scans.
    #[test]
    fn seam_xpub_matches_receive_side_on_testnet() {
        use crate::model::dashpay_derivation::derive_dashpay_incoming_xpub;

        let (sender, recipient) = contact_ids();
        let seam = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("seam derive");

        let local =
            derive_dashpay_incoming_xpub(&TEST_SEED, Network::Testnet, 0, &sender, &recipient)
                .expect("local derive");

        assert_eq!(
            seam.public_key,
            local.public_key.serialize(),
            "published public key must match the receive-side xpub on testnet"
        );
        assert_eq!(
            seam.chain_code,
            local.chain_code.to_bytes(),
            "published chain code must match the receive-side xpub on testnet"
        );
        assert_eq!(
            seam.parent_fingerprint,
            local.parent_fingerprint.to_bytes(),
            "published parent fingerprint must match the receive-side xpub on testnet"
        );
    }

    /// Fund-routing invariant, both networks, all published fields.
    /// The send-side seam (`derive_contact_xpub_material`) and the receive-side
    /// scanning derivation (`derive_dashpay_incoming_xpub`) must agree on the
    /// full triple (public_key, chain_code, parent_fingerprint) for the same
    /// `(sender, recipient)` on Mainnet AND Testnet. Any disagreement means a
    /// contact pays into addresses the recipient never scans.
    #[test]
    fn seam_matches_receive_side_full_fields_on_both_networks() {
        use crate::model::dashpay_derivation::derive_dashpay_incoming_xpub;

        let (sender, recipient) = contact_ids();
        for network in [Network::Mainnet, Network::Testnet] {
            let seam = derive_contact_xpub_material(
                &TEST_SEED,
                network,
                0,
                &sender,
                &recipient,
                &TEST_SECRET,
            )
            .expect("seam derive");

            let local = derive_dashpay_incoming_xpub(&TEST_SEED, network, 0, &sender, &recipient)
                .expect("local derive");

            assert_eq!(
                seam.public_key,
                local.public_key.serialize(),
                "public key must match send↔receive on {network:?}"
            );
            assert_eq!(
                seam.chain_code,
                local.chain_code.to_bytes(),
                "chain code must match send↔receive on {network:?}"
            );
            assert_eq!(
                seam.parent_fingerprint,
                local.parent_fingerprint.to_bytes(),
                "parent fingerprint must match send↔receive on {network:?}"
            );
        }
    }

    /// The 32-byte ECDH secret key vs the 64-byte HD seed are different inputs:
    /// the placeholder fed the secret key in as if it were the HD seed, which
    /// produced an xpub unrelated to the wallet's real tree. This guards the
    /// behavioural delta — the real HD seed must drive the derivation.
    #[test]
    fn hd_seed_drives_derivation_not_the_ecdh_secret() {
        let (sender, recipient) = contact_ids();
        let from_seed = derive_contact_xpub_material(
            &TEST_SEED,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive from real seed");

        // Reproduce the old placeholder: 32-byte secret right-padded to a
        // 64-byte "seed". This must NOT match the real-seed derivation.
        let mut placeholder_seed = [0u8; 64];
        placeholder_seed[..32].copy_from_slice(&TEST_SECRET);
        let from_placeholder = derive_contact_xpub_material(
            &placeholder_seed,
            Network::Testnet,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("derive from placeholder");

        assert_ne!(
            from_seed.public_key, from_placeholder.public_key,
            "deriving from the real HD seed must differ from the old private-key placeholder"
        );
    }

    /// SEC-W-001 multi-wallet fund-routing invariant. When a DashPay identity
    /// has more than one associated wallet, the send-side wallet selection
    /// (`QualifiedIdentity::dashpay_wallet_seed_hash`, used by
    /// `contact_requests` / `contact_info`) and the receive-side selection
    /// (`QualifiedIdentity::dashpay_wallet`, used by `incoming_payments`) MUST
    /// resolve to the same wallet — the lowest seed hash — or a contact would
    /// pay into addresses the recipient never scans.
    ///
    /// Proves the full chain:
    /// 1. both accessors pick the same (lowest-hash) wallet;
    /// 2. the contact xpub the send side publishes from the selected seed
    ///    equals the xpub the receive side scans from the selected seed
    ///    (published == scanned);
    /// 3. the *other* wallet's seed derives a different xpub — i.e. picking the
    ///    wrong wallet would route funds astray, so the agreement is load-bearing.
    #[test]
    fn multi_wallet_send_and_receive_select_same_wallet() {
        use crate::model::dashpay_derivation::derive_dashpay_incoming_xpub;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        use crate::model::wallet::Wallet;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use std::collections::BTreeMap;
        use std::sync::{Arc, RwLock};

        let network = Network::Testnet;
        let (sender, recipient) = contact_ids();

        // Two distinct HD seeds → two wallets with distinct seed hashes.
        let seed_a = [0x11u8; 64];
        let seed_b = [0x99u8; 64];
        let wallet_a = Wallet::new_from_seed(seed_a, network, None, None).expect("wallet a");
        let wallet_b = Wallet::new_from_seed(seed_b, network, None, None).expect("wallet b");
        let hash_a = wallet_a.seed_hash();
        let hash_b = wallet_b.seed_hash();
        assert_ne!(hash_a, hash_b, "test seeds must produce distinct hashes");

        // The selected (lowest-hash) wallet's seed, and the other one.
        let (selected_hash, selected_seed, other_seed) = if hash_a <= hash_b {
            (hash_a, seed_a, seed_b)
        } else {
            (hash_b, seed_b, seed_a)
        };

        let mut associated_wallets = BTreeMap::new();
        associated_wallets.insert(hash_a, Arc::new(RwLock::new(wallet_a)));
        associated_wallets.insert(hash_b, Arc::new(RwLock::new(wallet_b)));

        let identity = Identity::create_basic_identity(sender, PlatformVersion::latest())
            .expect("basic identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets,
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network,
        };

        // (1) Both sides select the same wallet — the lowest seed hash.
        let send_side = qi.dashpay_wallet_seed_hash().expect("send-side selection");
        let (recv_side, _) = qi.dashpay_wallet().expect("receive-side selection");
        assert_eq!(
            send_side, recv_side,
            "send-side and receive-side must select the same wallet"
        );
        assert_eq!(
            send_side, selected_hash,
            "selection must be the lowest seed hash"
        );

        // (2) Published (send side) == scanned (receive side) for the selected seed.
        let published = derive_contact_xpub_material(
            &selected_seed,
            network,
            0,
            &sender,
            &recipient,
            &TEST_SECRET,
        )
        .expect("send-side material");
        let scanned = derive_dashpay_incoming_xpub(&selected_seed, network, 0, &sender, &recipient)
            .expect("receive-side xpub");
        assert_eq!(
            published.public_key,
            scanned.public_key.serialize(),
            "published xpub must equal the scanned xpub for the selected wallet"
        );

        // (3) The other wallet's seed derives a different xpub — agreement matters.
        let other_scanned =
            derive_dashpay_incoming_xpub(&other_seed, network, 0, &sender, &recipient)
                .expect("other-side xpub");
        assert_ne!(
            published.public_key,
            other_scanned.public_key.serialize(),
            "the unselected wallet's seed must derive a different xpub"
        );
    }
}
