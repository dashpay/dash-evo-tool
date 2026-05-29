//! DashPay read-only adapter for `WalletBackend`.
//!
//! Translates upstream presence-based DashPay reads
//! (`platform_wallet::wallet::identity::types::dashpay::*`) into the
//! DET-shape `Stored*` records the existing UI and backend-task layer
//! already understand. Read-only foundation for the unwire stack: D2
//! routes existing read paths through this view; D3 wires writes; D4
//! drops the DET DashPay tables.
//!
//! ## What this adapter owns
//!
//! - **Status derivation**: upstream stores DashPay state as presence
//!   (a row exists in `sent_contact_requests` ⇒ outgoing pending; a
//!   row exists in `established_contacts` ⇒ accepted; etc.). DET's
//!   schema models the same state as an explicit `status` string.
//!   This module performs that translation at read time — there is
//!   no cache and no extra source of truth.
//! - **DET-local overlays via the k/v sidecar**: a small set of
//!   contact / contact-request attributes have no upstream surface
//!   yet (`blocked`, `rejected`, DET-local `created_at` /
//!   `updated_at` timestamps). D1 only *reads* these keys; missing
//!   keys yield safe defaults (not blocked, not rejected, timestamps
//!   `0`). D3 will start writing them.
//!
//! ## What this adapter does NOT do
//!
//! - It never calls [`platform_wallet::IdentityWallet::dashpay_sync`].
//!   Reads observe whatever state upstream has after its own sync.
//! - It does not fetch profile / DPNS data from the network. Fields
//!   that DET historically populated from cross-references (a
//!   contact's display name from their DashPay profile, a contact
//!   request's `to_username` from DPNS) come through as `None` until
//!   D2 wires those joins.
//! - It does not touch the DET SQLite DashPay tables — D4 drops
//!   those, but D1 leaves them alone.

use std::sync::Arc;

use dash_sdk::platform::Identifier;
use platform_wallet::PlatformWallet;
use platform_wallet::wallet::identity::types::dashpay::contact_request::ContactRequest;
use platform_wallet::wallet::identity::types::dashpay::established_contact::EstablishedContact;
use platform_wallet::wallet::identity::types::dashpay::payment::{
    PaymentDirection, PaymentEntry, PaymentStatus,
};
use platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile;

use crate::backend_task::error::TaskError;
use crate::model::dashpay::{
    ContactAddressIndex, ContactPrivateInfo, StoredContact, StoredContactRequest, StoredPayment,
    StoredProfile,
};
use crate::wallet_backend::WalletBackend;
use crate::wallet_backend::kv::DetKv;

// ---------------------------------------------------------------------------
// K/V sidecar key prefixes
// ---------------------------------------------------------------------------
//
// All sidecar keys are scoped to the global slot of the per-network
// upstream persister. The network already partitions the database
// file, so no additional `<network>:` prefix is needed inside the
// key itself.

/// Mark a contact as blocked. Value: empty (`()`). Presence is the signal.
const KV_PREFIX_BLOCKED: &str = "det:dashpay:blocked:";
/// Mark a contact request as rejected. Value: empty (`()`). Presence is the signal.
const KV_PREFIX_REJECTED: &str = "det:dashpay:rejected:";
/// DET-local `(created_at, updated_at)` timestamps for an entity (contact, request).
/// Value: `(i64, i64)` encoded by the [`DetKv`] schema.
const KV_PREFIX_TIMESTAMPS: &str = "det:dashpay:timestamps:";
/// DET-local private memo for a contact (nickname / notes / hidden).
/// Value: bincode-encoded [`ContactPrivateInfo`].
/// Key shape: `det:dashpay:private:<owner_b58>:<contact_b58>`.
const KV_PREFIX_PRIVATE: &str = "det:dashpay:private:";
/// Per-contact address index state (DIP-0015 send/receive cursors + bloom
/// registered count). Value: bincode-encoded [`ContactAddressIndex`].
/// Key shape: `det:dashpay:address_index:<owner_b58>:<contact_b58>`.
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
pub const DASHPAY_REQUEST_EXPIRY_DAYS: i64 = 7;

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

    /// All contacts for `owner` — established (`accepted`), outstanding
    /// outgoing (`pending`), and DET-local sidecar (`blocked`).
    ///
    /// Returns an empty vector when `owner` is unknown to upstream.
    pub async fn contacts(&self, owner: &Identifier) -> Vec<StoredContact> {
        let Some(wallet) = self.backend.find_wallet_for_identity(owner).await else {
            return Vec::new();
        };
        let kv = self.backend.kv();
        let state = wallet.state().await;
        let info = &*state;
        let Some(managed) = info.identity_manager.managed_identity(owner) else {
            return Vec::new();
        };

        let mut out: Vec<StoredContact> = Vec::new();

        // 1. Established (`accepted`) contacts.
        for contact in managed.established_contacts.values() {
            let contact_id = &contact.contact_identity_id;
            let blocked = kv_contains(&kv, KV_PREFIX_BLOCKED, contact_id);
            let status = if blocked { "blocked" } else { "accepted" };
            let (created_at, updated_at) = kv_timestamps(&kv, contact_id);
            out.push(established_to_det(
                owner, contact, status, created_at, updated_at,
            ));
        }

        // 2. Sent-but-not-yet-reciprocated outgoing requests → `pending` contacts.
        //    Skip recipients we already have an established row for above.
        for (recipient_id, request) in managed.sent_contact_requests.iter() {
            if managed.established_contacts.contains_key(recipient_id) {
                continue;
            }
            let blocked = kv_contains(&kv, KV_PREFIX_BLOCKED, recipient_id);
            let status = if blocked { "blocked" } else { "pending" };
            let (created_at, updated_at) = kv_timestamps(&kv, recipient_id);
            out.push(request_to_det_contact(
                owner,
                recipient_id,
                request,
                status,
                created_at,
                updated_at,
            ));
        }

        out
    }

    /// Outstanding contact requests for `owner` — sent (outgoing, status
    /// derived from upstream presence + sidecar) and received (incoming,
    /// status derived likewise).
    ///
    /// Returns an empty vector when `owner` is unknown to upstream.
    pub async fn contact_requests(&self, owner: &Identifier) -> Vec<StoredContactRequest> {
        let Some(wallet) = self.backend.find_wallet_for_identity(owner).await else {
            return Vec::new();
        };
        let kv = self.backend.kv();
        let state = wallet.state().await;
        let info = &*state;
        let Some(managed) = info.identity_manager.managed_identity(owner) else {
            return Vec::new();
        };

        let mut out: Vec<StoredContactRequest> = Vec::new();

        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;

        // Outgoing requests (`request_type = "sent"`).
        for (recipient_id, request) in managed.sent_contact_requests.iter() {
            let status = derive_request_status(
                /* request_id_for_sidecar = */ recipient_id,
                /* has_matching_established = */
                managed.established_contacts.contains_key(recipient_id),
                request.created_at,
                now_ms,
                &kv,
            );
            out.push(request_to_det_request(
                owner,
                recipient_id,
                request,
                "sent",
                &status,
            ));
        }

        // Incoming requests (`request_type = "received"`).
        for (sender_id, request) in managed.incoming_contact_requests.iter() {
            let status = derive_request_status(
                sender_id,
                managed.established_contacts.contains_key(sender_id),
                request.created_at,
                now_ms,
                &kv,
            );
            out.push(request_to_det_request(
                owner, sender_id, request, "received", &status,
            ));
        }

        out
    }

    /// Payment history for `owner`, newest entries first. Returns an
    /// empty vector when `owner` is unknown to upstream.
    pub async fn payments(&self, owner: &Identifier) -> Vec<StoredPayment> {
        let Some(wallet) = self.backend.find_wallet_for_identity(owner).await else {
            return Vec::new();
        };
        let kv = self.backend.kv();
        let state = wallet.state().await;
        let info = &*state;
        let Some(managed) = info.identity_manager.managed_identity(owner) else {
            return Vec::new();
        };

        let mut out: Vec<StoredPayment> = managed
            .dashpay_payments
            .iter()
            .map(|(tx_id, entry)| payment_to_det(owner, tx_id, entry, &kv))
            .collect();
        // Upstream stores payments keyed by tx_id in a BTreeMap (lexicographic
        // order). DET's UI sorts by `created_at DESC`; since sidecar timestamps
        // default to 0 when unset, fall back to that ordering — newest first
        // when timestamps exist, otherwise stable on tx_id.
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    /// DashPay profile for `owner`, or `None` when upstream has none
    /// (either `owner` is unknown, or its identity bucket has no
    /// `DashPayProfile` yet).
    pub async fn profile(&self, owner: &Identifier) -> Option<StoredProfile> {
        let wallet = self.backend.find_wallet_for_identity(owner).await?;
        let kv = self.backend.kv();
        let state = wallet.state().await;
        let info = &*state;
        let managed = info.identity_manager.managed_identity(owner)?;
        let profile = managed.dashpay_profile.as_ref()?;
        let (created_at, updated_at) = kv_timestamps(&kv, owner);
        Some(profile_to_det(owner, profile, created_at, updated_at))
    }
}

// ---------------------------------------------------------------------------
// Pure translators — unit-tested without an upstream backend.
// ---------------------------------------------------------------------------

fn established_to_det(
    owner: &Identifier,
    contact: &EstablishedContact,
    status: &str,
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
        contact_status: status.to_string(),
        created_at,
        updated_at,
        last_seen: None,
    }
}

fn request_to_det_contact(
    owner: &Identifier,
    counterparty: &Identifier,
    _request: &ContactRequest,
    status: &str,
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
        contact_status: status.to_string(),
        created_at,
        updated_at,
        last_seen: None,
    }
}

fn request_to_det_request(
    owner: &Identifier,
    counterparty: &Identifier,
    request: &ContactRequest,
    request_type: &str,
    status: &str,
) -> StoredContactRequest {
    let (from_id, to_id) = if request_type == "sent" {
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
        request_type: request_type.to_string(),
        status: status.to_string(),
        // Upstream provides `created_at` directly — no sidecar read needed.
        created_at: request.created_at as i64,
        responded_at: None,
        // Threshold-based expiry derivation is not yet wired (no DET-side
        // threshold constant). D2 picks this up.
        expires_at: None,
    }
}

fn payment_to_det(
    owner: &Identifier,
    tx_id: &str,
    entry: &PaymentEntry,
    kv: &DetKv,
) -> StoredPayment {
    let (from_id, to_id, payment_type) = match entry.direction {
        PaymentDirection::Sent => (owner, &entry.counterparty_id, "sent"),
        PaymentDirection::Received => (&entry.counterparty_id, owner, "received"),
    };
    let status = match entry.status {
        PaymentStatus::Pending => "pending",
        PaymentStatus::Confirmed => "confirmed",
        PaymentStatus::Failed => "failed",
    };
    // Use the tx_id string as the sidecar key (no Identifier conversion).
    let (created_at, confirmed_at) = kv_payment_timestamps(kv, tx_id);
    StoredPayment {
        id: 0,
        tx_id: tx_id.to_string(),
        from_identity_id: from_id.to_buffer().to_vec(),
        to_identity_id: to_id.to_buffer().to_vec(),
        amount: entry.amount_duffs as i64,
        memo: entry.memo.clone(),
        payment_type: payment_type.to_string(),
        status: status.to_string(),
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
/// `created_at_ms` vs `now_ms`) reports as `"expired"`.
fn derive_request_status(
    counterparty: &Identifier,
    has_matching_established: bool,
    created_at_ms: u64,
    now_ms: u64,
    kv: &DetKv,
) -> String {
    if has_matching_established {
        return "accepted".to_string();
    }
    if kv_contains(kv, KV_PREFIX_REJECTED, counterparty) {
        return "rejected".to_string();
    }
    let age_ms = now_ms.saturating_sub(created_at_ms);
    let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64).saturating_mul(86_400_000);
    if age_ms > threshold_ms {
        return "expired".to_string();
    }
    "pending".to_string()
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

    let key = pair_sidecar_key(KV_PREFIX_ADDRESS_INDEX, owner, contact);
    let mut state: ContactAddressIndex = kv
        .get::<ContactAddressIndex>(None, &key)
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
    kv.put::<ContactAddressIndex>(None, &key, &state)
        .map_err(|e| TaskError::DashpaySidecarStorage { source: e })?;
    Ok(value)
}

// ---------------------------------------------------------------------------
// K/V sidecar helpers
// ---------------------------------------------------------------------------

fn sidecar_key(prefix: &str, id: &Identifier) -> String {
    format!(
        "{prefix}{}",
        id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
    )
}

/// Sidecar key for a `(owner, contact)` overlay (private memo, address index).
/// Format: `<prefix><owner_b58>:<contact_b58>`.
fn pair_sidecar_key(prefix: &str, owner: &Identifier, contact: &Identifier) -> String {
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    format!(
        "{prefix}{}:{}",
        owner.to_string(Encoding::Base58),
        contact.to_string(Encoding::Base58)
    )
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

fn kv_contains(kv: &DetKv, prefix: &str, id: &Identifier) -> bool {
    // Presence-only entries: value is `()`. `Ok(Some(_))` ⇒ present.
    matches!(kv.get::<()>(None, &sidecar_key(prefix, id)), Ok(Some(())))
}

fn kv_timestamps(kv: &DetKv, id: &Identifier) -> (i64, i64) {
    let key = sidecar_key(KV_PREFIX_TIMESTAMPS, id);
    match kv.get::<(i64, i64)>(None, &key) {
        Ok(Some(ts)) => ts,
        // Missing or decode error → safe default. Fresh users on a
        // pre-D3 build will hit this; an explicit log is intentional
        // only on decode failure since plain absence is the steady
        // state today.
        Ok(None) => (0, 0),
        Err(e) => {
            tracing::debug!(
                key = %key,
                error = ?e,
                "DashpayView timestamp sidecar decode failed; defaulting to zeros"
            );
            (0, 0)
        }
    }
}

fn kv_payment_timestamps(kv: &DetKv, tx_id: &str) -> (i64, Option<i64>) {
    let key = format!("{KV_PREFIX_TIMESTAMPS}tx:{tx_id}");
    match kv.get::<(i64, Option<i64>)>(None, &key) {
        Ok(Some(ts)) => ts,
        Ok(None) => (0, None),
        Err(e) => {
            tracing::debug!(
                key = %key,
                error = ?e,
                "DashpayView payment timestamp sidecar decode failed; defaulting to zeros"
            );
            (0, None)
        }
    }
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
        wallet.identity().dashpay_sync().await.map_err(|e| {
            crate::backend_task::error::TaskError::WalletBackend {
                source: Box::new(e),
            }
        })
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
        managed.record_dashpay_payment(tx_id, entry, &persister);
        Ok(())
    }

    /// Toggle the DET-local "blocked" marker for a contact identity in the
    /// k/v sidecar. The marker has no upstream counterpart — DashPay does
    /// not block on-chain — so it lives entirely in the per-network
    /// sidecar that [`DashpayView`] reads at view time.
    pub fn dashpay_mark_blocked(&self, contact_id: &Identifier) -> Result<(), TaskError> {
        let key = sidecar_key(KV_PREFIX_BLOCKED, contact_id);
        self.kv()
            .put::<()>(None, &key, &())
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Clear the DET-local "blocked" marker for a contact identity.
    /// Idempotent — clearing an absent marker is `Ok(())`.
    pub fn dashpay_unmark_blocked(&self, contact_id: &Identifier) -> Result<(), TaskError> {
        let key = sidecar_key(KV_PREFIX_BLOCKED, contact_id);
        self.kv()
            .delete(None, &key)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Record that the user has rejected an incoming contact request from
    /// `counterparty_id` (or, equivalently, the sent request to them was
    /// withdrawn from the user's point of view). The sidecar key matches
    /// what [`DashpayView`] consults when deriving request status.
    pub fn dashpay_mark_rejected(&self, counterparty_id: &Identifier) -> Result<(), TaskError> {
        let key = sidecar_key(KV_PREFIX_REJECTED, counterparty_id);
        self.kv()
            .put::<()>(None, &key, &())
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
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
            .put::<(i64, i64)>(None, &key, &(created_at, updated_at))
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
            .put::<(i64, Option<i64>)>(None, &key, &(created_at, confirmed_at))
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
        let key = pair_sidecar_key(KV_PREFIX_PRIVATE, owner, contact);
        self.kv()
            .get::<ContactPrivateInfo>(None, &key)
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
    }

    /// Upsert the DET-local private memo for `(owner, contact)`.
    pub fn dashpay_set_private_info(
        &self,
        owner: &Identifier,
        contact: &Identifier,
        info: &ContactPrivateInfo,
    ) -> Result<(), TaskError> {
        let key = pair_sidecar_key(KV_PREFIX_PRIVATE, owner, contact);
        self.kv()
            .put::<ContactPrivateInfo>(None, &key, info)
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
        let key = pair_sidecar_key(KV_PREFIX_ADDRESS_INDEX, owner, contact);
        self.kv()
            .get::<ContactAddressIndex>(None, &key)
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
        let key = pair_sidecar_key(KV_PREFIX_ADDRESS_INDEX, owner, contact);
        self.kv()
            .put::<ContactAddressIndex>(None, &key, index)
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
            .get::<([u8; 32], u32)>(None, &key)
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
            .put::<([u8; 32], u32)>(None, &key, &(contact.to_buffer(), address_index))
            .map_err(|e| TaskError::DashpaySidecarStorage { source: e })
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

        let det = established_to_det(&owner, &contact, "accepted", 1_000, 2_000);
        assert_eq!(det.owner_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.contact_identity_id, contact_id.to_buffer().to_vec());
        assert_eq!(det.display_name.as_deref(), Some("Buddy"));
        assert_eq!(det.public_message.as_deref(), Some("Met at conf"));
        assert_eq!(det.contact_status, "accepted");
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
        let request = mk_request(1, 2, 123);

        let det = request_to_det_contact(&owner, &recipient, &request, "pending", 0, 0);
        assert_eq!(det.contact_status, "pending");
        assert_eq!(det.owner_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.contact_identity_id, recipient.to_buffer().to_vec());
    }

    #[test]
    fn outgoing_request_translation_preserves_direction() {
        let owner = id_from_byte(1);
        let recipient = id_from_byte(2);
        let request = mk_request(1, 2, 123);

        let det = request_to_det_request(&owner, &recipient, &request, "sent", "pending");
        assert_eq!(det.from_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.to_identity_id, recipient.to_buffer().to_vec());
        assert_eq!(det.request_type, "sent");
        assert_eq!(det.status, "pending");
        assert_eq!(det.created_at, 123);
        // Encrypted label is never surfaced as a plaintext `account_label`.
        assert!(det.account_label.is_none());
    }

    #[test]
    fn incoming_request_translation_flips_direction() {
        let owner = id_from_byte(1);
        let sender = id_from_byte(2);
        let request = mk_request(2, 1, 456);

        let det = request_to_det_request(&owner, &sender, &request, "received", "pending");
        assert_eq!(det.from_identity_id, sender.to_buffer().to_vec());
        assert_eq!(det.to_identity_id, owner.to_buffer().to_vec());
        assert_eq!(det.request_type, "received");
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
        assert_eq!(det.payment_type, "sent");
        assert_eq!(det.status, "pending");
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
        assert_eq!(det.payment_type, "received");
        assert_eq!(det.status, "confirmed");
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
        let counterparty = id_from_byte(2);
        // Fresh request, no expiry yet.
        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&counterparty, true, created_at_ms, now_ms, &kv),
            "accepted",
            "matching established contact wins"
        );
        assert_eq!(
            derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv),
            "pending",
            "no established + no rejection sidecar + fresh = pending"
        );
    }

    #[test]
    fn rejected_request_status_reads_sidecar_when_present() {
        let kv = empty_kv();
        let counterparty = id_from_byte(2);
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_REJECTED, &counterparty), &())
            .unwrap();
        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv),
            "rejected"
        );
    }

    #[test]
    fn expired_request_status_when_older_than_threshold() {
        let kv = empty_kv();
        let counterparty = id_from_byte(2);
        let now_ms: u64 = 10_000_000_000_000;
        // Older than the 7-day threshold by one minute.
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        let created_at_ms: u64 = now_ms - threshold_ms - 60_000;
        assert_eq!(
            derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv),
            "expired",
            "older-than-threshold pending request reports as expired"
        );
    }

    #[test]
    fn fresh_request_just_under_threshold_stays_pending() {
        let kv = empty_kv();
        let counterparty = id_from_byte(2);
        let now_ms: u64 = 10_000_000_000_000;
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        // One minute younger than the threshold.
        let created_at_ms: u64 = now_ms - threshold_ms + 60_000;
        assert_eq!(
            derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv),
            "pending"
        );
    }

    #[test]
    fn blocked_contact_overrides_accepted_status() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact_id = id_from_byte(2);
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_BLOCKED, &contact_id), &())
            .unwrap();

        let mut contact =
            EstablishedContact::new(contact_id, mk_request(1, 2, 100), mk_request(2, 1, 200));
        contact.set_alias("Friend".into());

        let status = if kv_contains(&kv, KV_PREFIX_BLOCKED, &contact_id) {
            "blocked"
        } else {
            "accepted"
        };
        let det = established_to_det(&owner, &contact, status, 0, 0);
        assert_eq!(det.contact_status, "blocked");
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
        kv.put::<(i64, i64)>(None, &sidecar_key(KV_PREFIX_TIMESTAMPS, &id), &(111, 222))
            .unwrap();
        assert_eq!(kv_timestamps(&kv, &id), (111, 222));
    }

    #[test]
    fn payment_timestamps_round_trip() {
        let kv = empty_kv();
        let tx_id = "tx-xyz";
        kv.put::<(i64, Option<i64>)>(
            None,
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
    /// These tests use the same `sidecar_key` builder + the read helpers
    /// directly, simulating a write-then-read round-trip without
    /// constructing a full `WalletBackend`.
    #[test]
    fn d3_blocked_marker_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let contact = id_from_byte(7);
        // What `dashpay_mark_blocked` writes:
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_BLOCKED, &contact), &())
            .unwrap();
        // What `DashpayView::contacts` reads:
        assert!(kv_contains(&kv, KV_PREFIX_BLOCKED, &contact));

        // And `dashpay_unmark_blocked` (delete) clears it.
        kv.delete(None, &sidecar_key(KV_PREFIX_BLOCKED, &contact))
            .unwrap();
        assert!(!kv_contains(&kv, KV_PREFIX_BLOCKED, &contact));
    }

    #[test]
    fn d3_rejected_marker_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let counterparty = id_from_byte(8);
        // What `dashpay_mark_rejected` writes:
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_REJECTED, &counterparty), &())
            .unwrap();
        // What `derive_request_status` reads:
        assert!(kv_contains(&kv, KV_PREFIX_REJECTED, &counterparty));

        let now_ms: u64 = 1_000_000_000_000;
        let created_at_ms: u64 = now_ms - 60_000;
        assert_eq!(
            derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv),
            "rejected"
        );
    }

    #[test]
    fn d3_timestamp_sidecar_round_trips() {
        let kv = empty_kv();
        let entity = id_from_byte(9);
        // What `dashpay_set_timestamps` writes:
        kv.put::<(i64, i64)>(
            None,
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
            None,
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
        let contact_id = id_from_byte(2);

        // Pre-state: a single established contact exists upstream.
        let mut contact =
            EstablishedContact::new(contact_id, mk_request(1, 2, 100), mk_request(2, 1, 200));
        contact.set_alias("Pal".into());

        // What `dashpay_mark_blocked(&contact_id)` writes:
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_BLOCKED, &contact_id), &())
            .unwrap();

        // What the view derivation produces — same precedence as
        // `DashpayView::contacts`: blocked wins over accepted.
        let status = if kv_contains(&kv, KV_PREFIX_BLOCKED, &contact_id) {
            "blocked"
        } else {
            "accepted"
        };
        let det = established_to_det(&owner, &contact, status, 0, 0);
        assert_eq!(det.contact_status, "blocked");
        assert_eq!(det.display_name.as_deref(), Some("Pal"));
    }

    #[test]
    fn d3_reject_then_list_contact_requests_yields_rejected_status() {
        // Simulates: send → reject → list. After D3 wires
        // `dashpay_mark_rejected`, the outgoing request's status flips
        // to "rejected" without touching upstream presence (rejected
        // requests are not removed from `sent_contact_requests`).
        let kv = empty_kv();
        let counterparty = id_from_byte(2);
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_REJECTED, &counterparty), &())
            .unwrap();

        let now_ms: u64 = 2_000_000_000_000;
        let created_at_ms: u64 = now_ms - 1_000;
        let derived = derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv);
        assert_eq!(derived, "rejected");

        // And the threshold-expiry override does not fire for rejected
        // requests — `rejected` precedence is higher than `expired`.
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        let old_created = now_ms - threshold_ms - 60_000;
        let derived_old = derive_request_status(&counterparty, false, old_created, now_ms, &kv);
        assert_eq!(derived_old, "rejected");
    }

    #[test]
    fn d3_seven_day_old_pending_request_reports_expired() {
        // Send → wait > 7 days → list → assert expired. The DET-side
        // expiry threshold lives in `DASHPAY_REQUEST_EXPIRY_DAYS` and
        // is a UX gate; upstream stores no protocol-level expiry.
        let kv = empty_kv();
        let counterparty = id_from_byte(2);
        let now_ms: u64 = 50_000_000_000_000;
        let threshold_ms = (DASHPAY_REQUEST_EXPIRY_DAYS as u64) * 86_400_000;
        // 7 days + a margin of safety.
        let created_at_ms: u64 = now_ms - threshold_ms - 86_400_000;
        assert_eq!(
            derive_request_status(&counterparty, false, created_at_ms, now_ms, &kv),
            "expired"
        );
    }

    // -------------------------------------------------------------------
    // D4b: address-index + private-info k/v primitives (key shape +
    // round-trip via the same `DetKv` adapter used in production). The
    // wallet-resolving methods on `WalletBackend` itself need an
    // upstream backend and are covered by the e2e suite.
    // -------------------------------------------------------------------

    #[test]
    fn d4b_pair_sidecar_key_uses_base58_colon_form() {
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);
        let key = pair_sidecar_key(KV_PREFIX_PRIVATE, &owner, &contact);
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let expected = format!(
            "det:dashpay:private:{}:{}",
            owner.to_string(Encoding::Base58),
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
    fn d4b_private_info_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);
        let info = ContactPrivateInfo {
            nickname: "Alice".into(),
            notes: "met at conf".into(),
            is_hidden: true,
        };
        let key = pair_sidecar_key(KV_PREFIX_PRIVATE, &owner, &contact);
        kv.put::<ContactPrivateInfo>(None, &key, &info).unwrap();
        let got: ContactPrivateInfo = kv
            .get::<ContactPrivateInfo>(None, &key)
            .unwrap()
            .expect("written value should round-trip");
        assert_eq!(got, info);
    }

    #[test]
    fn d4b_private_info_missing_key_returns_none() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);
        let key = pair_sidecar_key(KV_PREFIX_PRIVATE, &owner, &contact);
        assert!(kv.get::<ContactPrivateInfo>(None, &key).unwrap().is_none());
    }

    #[test]
    fn d4b_address_index_round_trips_through_sidecar_key() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);
        let idx = ContactAddressIndex {
            owner_identity_id: owner.to_buffer().to_vec(),
            contact_identity_id: contact.to_buffer().to_vec(),
            next_send_index: 7,
            highest_receive_index: 3,
            bloom_registered_count: 20,
        };
        let key = pair_sidecar_key(KV_PREFIX_ADDRESS_INDEX, &owner, &contact);
        kv.put::<ContactAddressIndex>(None, &key, &idx).unwrap();
        let got = kv
            .get::<ContactAddressIndex>(None, &key)
            .unwrap()
            .expect("written value should round-trip");
        assert_eq!(got.next_send_index, 7);
        assert_eq!(got.highest_receive_index, 3);
        assert_eq!(got.bloom_registered_count, 20);
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
        kv.put::<([u8; 32], u32)>(None, &key, &(contact.to_buffer(), 7))
            .unwrap();
        let got: Option<([u8; 32], u32)> = kv.get::<([u8; 32], u32)>(None, &key).unwrap();
        assert_eq!(got, Some((contact.to_buffer(), 7)));
    }

    #[test]
    fn d4b_pair_key_distinguishes_owner_from_contact() {
        let a = id_from_byte(1);
        let b = id_from_byte(2);
        let key_a_b = pair_sidecar_key(KV_PREFIX_PRIVATE, &a, &b);
        let key_b_a = pair_sidecar_key(KV_PREFIX_PRIVATE, &b, &a);
        assert_ne!(
            key_a_b, key_b_a,
            "address-index/private overlays are not symmetric in (owner, contact)"
        );
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

        // Final persisted counter advances to 100.
        let key = pair_sidecar_key(KV_PREFIX_ADDRESS_INDEX, &owner, &contact);
        let final_state: ContactAddressIndex = kv
            .get::<ContactAddressIndex>(None, &key)
            .expect("kv read")
            .expect("counter must have been initialized");
        assert_eq!(final_state.next_send_index, 100);
    }

    // -------------------------------------------------------------------
    // D4d: the `det:dashpay:` prefix sweep used by
    // `AppContext::clear_network_database` must hit every overlay the
    // adapter writes — private memo, blocked / rejected markers,
    // timestamps, address index, address mapping. A miss here would
    // leak DET-only state across a "Clear network data" action.
    // -------------------------------------------------------------------

    /// D4d-Sweep1: every adapter-written sidecar key starts with the
    /// shared `det:dashpay:` prefix, so a single `list_global` enumerates
    /// all of them in one pass.
    #[test]
    fn d4d_all_dashpay_sidecar_keys_share_the_prefix() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);
        let addr = "yXyqJv6gP2c8RXAhYQ7v6XwxSqUf7vXKfA";

        // Plant one of every overlay shape DashPay writes.
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_BLOCKED, &contact), &())
            .unwrap();
        kv.put::<()>(None, &sidecar_key(KV_PREFIX_REJECTED, &contact), &())
            .unwrap();
        kv.put::<(i64, i64)>(
            None,
            &sidecar_key(KV_PREFIX_TIMESTAMPS, &contact),
            &(111, 222),
        )
        .unwrap();
        kv.put::<(i64, Option<i64>)>(
            None,
            &format!("{KV_PREFIX_TIMESTAMPS}tx:abc123"),
            &(333, Some(444)),
        )
        .unwrap();
        kv.put::<ContactPrivateInfo>(
            None,
            &pair_sidecar_key(KV_PREFIX_PRIVATE, &owner, &contact),
            &ContactPrivateInfo::default(),
        )
        .unwrap();
        kv.put::<ContactAddressIndex>(
            None,
            &pair_sidecar_key(KV_PREFIX_ADDRESS_INDEX, &owner, &contact),
            &ContactAddressIndex {
                owner_identity_id: owner.to_buffer().to_vec(),
                contact_identity_id: contact.to_buffer().to_vec(),
                next_send_index: 1,
                highest_receive_index: 0,
                bloom_registered_count: 0,
            },
        )
        .unwrap();
        kv.put::<([u8; 32], u32)>(
            None,
            &addr_map_sidecar_key(&owner, addr),
            &(contact.to_buffer(), 1),
        )
        .unwrap();

        let keys = kv
            .list(None, Some("det:dashpay:"))
            .expect("sidecar listing must succeed");
        // 7 writes, each with a unique key.
        assert_eq!(keys.len(), 7, "every overlay must be enumerated: {keys:?}");
        for k in &keys {
            assert!(
                k.starts_with("det:dashpay:"),
                "non-DashPay key surfaced: {k}"
            );
        }
    }

    /// D4d-Sweep2: iterating the prefix listing and deleting drains the
    /// sidecar — mirrors what `AppContext::clear_network_database` does
    /// post-D4d.
    #[test]
    fn d4d_prefix_sweep_drains_dashpay_sidecar() {
        let kv = empty_kv();
        let owner = id_from_byte(1);
        let contact = id_from_byte(2);

        kv.put::<()>(None, &sidecar_key(KV_PREFIX_BLOCKED, &contact), &())
            .unwrap();
        kv.put::<ContactPrivateInfo>(
            None,
            &pair_sidecar_key(KV_PREFIX_PRIVATE, &owner, &contact),
            &ContactPrivateInfo {
                nickname: "alice".into(),
                notes: "n".into(),
                is_hidden: false,
            },
        )
        .unwrap();
        // Drop one unrelated global key to confirm the sweep is scoped.
        kv.put::<u32>(None, "mainnet:scheduled_votes:1", &7)
            .unwrap();

        let keys = kv.list(None, Some("det:dashpay:")).unwrap();
        for k in &keys {
            kv.delete(None, k).unwrap();
        }

        assert!(
            kv.list(None, Some("det:dashpay:")).unwrap().is_empty(),
            "DashPay sidecar must be empty after the sweep"
        );
        // Unrelated key survives.
        assert_eq!(
            kv.get::<u32>(None, "mainnet:scheduled_votes:1").unwrap(),
            Some(7)
        );
    }

    /// D4d-Sweep3: the prefix sweep is precision-scoped — keys for
    /// unrelated overlays (settings, scheduled votes, shielded sidecar,
    /// etc.) must not be caught by `det:dashpay:`.
    #[test]
    fn d4d_prefix_sweep_skips_non_dashpay_keys() {
        let kv = empty_kv();
        kv.put::<u32>(None, "mainnet:settings:v1", &1).unwrap();
        kv.put::<u32>(None, "mainnet:shielded:sync_cursor", &2)
            .unwrap();
        kv.put::<u32>(None, "det:other_domain:x", &3).unwrap();

        let keys = kv.list(None, Some("det:dashpay:")).unwrap();
        assert!(
            keys.is_empty(),
            "prefix sweep must not see non-DashPay overlays: {keys:?}"
        );
    }

    // -------------------------------------------------------------------
    // In-memory KvStore for the translator tests.
    // -------------------------------------------------------------------

    fn empty_kv() -> DetKv {
        use platform_wallet::wallet::platform_wallet::WalletId;
        use platform_wallet_storage::{KvError, KvStore};
        use std::collections::BTreeMap;
        use std::sync::Mutex;

        #[derive(Default)]
        struct InMemoryKv {
            global: Mutex<BTreeMap<String, Vec<u8>>>,
            per_wallet: Mutex<BTreeMap<(WalletId, String), Vec<u8>>>,
        }

        impl KvStore for InMemoryKv {
            fn get(
                &self,
                wallet_id: Option<&WalletId>,
                key: &str,
            ) -> Result<Option<Vec<u8>>, KvError> {
                match wallet_id {
                    None => Ok(self.global.lock().unwrap().get(key).cloned()),
                    Some(id) => Ok(self
                        .per_wallet
                        .lock()
                        .unwrap()
                        .get(&(*id, key.to_string()))
                        .cloned()),
                }
            }
            fn put(
                &self,
                wallet_id: Option<&WalletId>,
                key: &str,
                value: &[u8],
            ) -> Result<(), KvError> {
                match wallet_id {
                    None => {
                        self.global
                            .lock()
                            .unwrap()
                            .insert(key.to_string(), value.to_vec());
                    }
                    Some(id) => {
                        self.per_wallet
                            .lock()
                            .unwrap()
                            .insert((*id, key.to_string()), value.to_vec());
                    }
                }
                Ok(())
            }
            fn delete(&self, wallet_id: Option<&WalletId>, key: &str) -> Result<(), KvError> {
                match wallet_id {
                    None => {
                        self.global.lock().unwrap().remove(key);
                    }
                    Some(id) => {
                        self.per_wallet
                            .lock()
                            .unwrap()
                            .remove(&(*id, key.to_string()));
                    }
                }
                Ok(())
            }
            fn list_keys(
                &self,
                wallet_id: Option<&WalletId>,
                prefix: Option<&str>,
            ) -> Result<Vec<String>, KvError> {
                let prefix = prefix.unwrap_or("");
                let mut keys: Vec<String> = match wallet_id {
                    None => self
                        .global
                        .lock()
                        .unwrap()
                        .keys()
                        .filter(|k| k.starts_with(prefix))
                        .cloned()
                        .collect(),
                    Some(id) => self
                        .per_wallet
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|((w, k), _)| w == id && k.starts_with(prefix))
                        .map(|((_, k), _)| k.clone())
                        .collect(),
                };
                keys.sort();
                Ok(keys)
            }
        }

        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }
}
