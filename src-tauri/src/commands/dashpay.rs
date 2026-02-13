//! DashPay-related Tauri IPC commands.
//!
//! Maps all 14 `DashPayTask` variants plus direct database methods to
//! Tauri commands. Long-running operations are dispatched asynchronously
//! via `task_dispatcher::dispatch_task` and results arrive as events.

use crate::commands::identity::parse_identifier;
use crate::dto::common::IdentifierDto;
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::dashpay::DashPayTask;
use dash_evo_tool::backend_task::BackendTask;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs
// ---------------------------------------------------------------------------

/// Input for loading a DashPay profile.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadProfileInput {
    pub identity_id: IdentifierDto,
}

/// Input for updating a DashPay profile.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileInput {
    pub identity_id: IdentifierDto,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
}

/// Input for loading contacts.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadContactsInput {
    pub identity_id: IdentifierDto,
}

/// Input for loading contact requests.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadContactRequestsInput {
    pub identity_id: IdentifierDto,
}

/// Input for fetching a contact's profile.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchContactProfileInput {
    pub identity_id: IdentifierDto,
    pub contact_id: IdentifierDto,
}

/// Input for searching profiles by keyword.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchProfilesInput {
    pub search_query: String,
}

/// Input for sending a contact request.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendContactRequestInput {
    pub identity_id: IdentifierDto,
    /// Key ID to use for signing.
    pub signing_key_id: u32,
    /// Recipient username (with or without .dash).
    pub to_username: String,
    pub account_label: Option<String>,
}

/// Input for sending a contact request with QR auto-accept proof.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendContactRequestWithProofInput {
    pub identity_id: IdentifierDto,
    pub signing_key_id: u32,
    pub to_identity_id: IdentifierDto,
    pub account_label: Option<String>,
    /// Auto-accept proof data from QR scan.
    pub proof_identity_id: IdentifierDto,
    pub proof_key_hex: String,
    pub proof_account_reference: u32,
    pub proof_expires_at: u64,
}

/// Input for accepting a contact request.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AcceptContactRequestInput {
    pub identity_id: IdentifierDto,
    pub request_id: IdentifierDto,
}

/// Input for rejecting a contact request.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RejectContactRequestInput {
    pub identity_id: IdentifierDto,
    pub request_id: IdentifierDto,
}

/// Input for loading payment history.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadPaymentHistoryInput {
    pub identity_id: IdentifierDto,
}

/// Input for sending a payment to a contact.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendPaymentToContactInput {
    pub identity_id: IdentifierDto,
    pub contact_id: IdentifierDto,
    pub amount_dash: f64,
    pub memo: Option<String>,
}

/// Input for updating contact info.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateContactInfoInput {
    pub identity_id: IdentifierDto,
    pub contact_id: IdentifierDto,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub is_hidden: bool,
    pub accepted_accounts: Vec<u32>,
}

/// Input for registering DashPay addresses.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDashPayAddressesInput {
    pub identity_id: IdentifierDto,
}

/// Input for generating a QR auto-accept proof.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenerateAutoAcceptProofInput {
    pub identity_id: IdentifierDto,
    pub account_index: u32,
    pub validity_hours: u32,
}

/// Result of QR auto-accept proof generation.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AutoAcceptProofResult {
    pub qr_string: String,
    pub identity_id: String,
    pub account_reference: u32,
    pub expires_at: u64,
}

/// Input for parsing a QR code string.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParseAutoAcceptProofInput {
    pub qr_data: String,
}

/// Parsed QR auto-accept proof data.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ParsedAutoAcceptProofResult {
    pub identity_id: String,
    pub proof_key_hex: String,
    pub account_reference: u32,
    pub expires_at: u64,
}

// ---------------------------------------------------------------------------
// DB-direct DTOs
// ---------------------------------------------------------------------------

/// Stored DashPay profile from local database.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StoredProfileDto {
    pub identity_id: IdentifierDto,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub public_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Stored DashPay contact from local database.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StoredContactDto {
    pub owner_identity_id: IdentifierDto,
    pub contact_identity_id: IdentifierDto,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub public_message: Option<String>,
    pub contact_status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen: Option<i64>,
}

/// Stored contact request from local database.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StoredContactRequestDto {
    pub id: i64,
    pub from_identity_id: IdentifierDto,
    pub to_identity_id: IdentifierDto,
    pub to_username: Option<String>,
    /// Resolved display name of the sender (for incoming requests).
    pub from_username: Option<String>,
    pub account_label: Option<String>,
    pub request_type: String,
    pub status: String,
    pub created_at: i64,
    pub responded_at: Option<i64>,
    pub expires_at: Option<i64>,
    /// The Platform document ID for this contact request (hex-encoded).
    /// Needed for accept/reject operations. May be null for old DB rows.
    pub platform_document_id: Option<IdentifierDto>,
}

/// Stored payment record from local database.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StoredPaymentDto {
    pub id: i64,
    pub tx_id: String,
    pub from_identity_id: IdentifierDto,
    pub to_identity_id: IdentifierDto,
    pub amount: i64,
    pub memo: Option<String>,
    pub payment_type: String,
    pub status: String,
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
}

/// Contact private info (nickname, notes, hidden).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContactPrivateInfoDto {
    pub nickname: String,
    pub notes: String,
    pub is_hidden: bool,
}

// ---------------------------------------------------------------------------
// Helper: Look up QualifiedIdentity and extract a signing key by ID
// ---------------------------------------------------------------------------

fn lookup_identity(
    state: &AppState,
    identity_id: &str,
) -> Result<dash_evo_tool::model::qualified_identity::QualifiedIdentity, String> {
    let identifier = parse_identifier(identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_by_id(&identifier)
        .map_err(|e| format!("Database error loading identity: {e}"))?
        .ok_or_else(|| format!("Identity {} not found in local database", identity_id))
}

fn lookup_signing_key(
    qi: &dash_evo_tool::model::qualified_identity::QualifiedIdentity,
    key_id: u32,
) -> Result<dash_sdk::platform::IdentityPublicKey, String> {
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    qi.identity
        .get_public_key_by_id(key_id as dash_sdk::dpp::identity::KeyID)
        .cloned()
        .ok_or_else(|| format!("Key ID {} not found on identity", key_id))
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Load a DashPay profile for an identity.
#[tauri::command]
#[specta::specta]
pub fn dashpay_load_profile(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: LoadProfileInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile { identity: qi }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Update a DashPay profile.
#[tauri::command]
#[specta::specta]
pub fn dashpay_update_profile(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: UpdateProfileInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateProfile {
        identity: qi,
        display_name: input.display_name,
        bio: input.bio,
        avatar_url: input.avatar_url,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Load contacts for an identity.
#[tauri::command]
#[specta::specta]
pub fn dashpay_load_contacts(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: LoadContactsInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts { identity: qi }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Load contact requests for an identity.
#[tauri::command]
#[specta::specta]
pub fn dashpay_load_contact_requests(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: LoadContactRequestsInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task =
        BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests { identity: qi }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Fetch a contact's public profile.
#[tauri::command]
#[specta::specta]
pub fn dashpay_fetch_contact_profile(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchContactProfileInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let contact_id = parse_identifier(&input.contact_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::FetchContactProfile {
        identity: qi,
        contact_id,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Search for DashPay profiles by keyword.
#[tauri::command]
#[specta::specta]
pub fn dashpay_search_profiles(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchProfilesInput,
) -> Result<DispatchTaskResponse, String> {
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles {
        search_query: input.search_query,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Send a contact request to a username.
#[tauri::command]
#[specta::specta]
pub fn dashpay_send_contact_request(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SendContactRequestInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = lookup_signing_key(&qi, input.signing_key_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
        identity: qi,
        signing_key,
        to_username: input.to_username,
        account_label: input.account_label,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Send a contact request with QR auto-accept proof.
#[tauri::command]
#[specta::specta]
pub fn dashpay_send_contact_request_with_proof(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SendContactRequestWithProofInput,
) -> Result<DispatchTaskResponse, String> {
    use dash_evo_tool::backend_task::dashpay::auto_accept_proof::AutoAcceptProofData;

    let qi = lookup_identity(&state, &input.identity_id)?;
    let signing_key = lookup_signing_key(&qi, input.signing_key_id)?;
    let to_identity_id = parse_identifier(&input.to_identity_id)?;
    let proof_identity_id = parse_identifier(&input.proof_identity_id)?;
    let proof_key_bytes =
        hex::decode(&input.proof_key_hex).map_err(|e| format!("Invalid proof key hex: {e}"))?;
    if proof_key_bytes.len() != 32 {
        return Err(format!(
            "Proof key must be 32 bytes, got {}",
            proof_key_bytes.len()
        ));
    }
    let mut proof_key = [0u8; 32];
    proof_key.copy_from_slice(&proof_key_bytes);

    let qr_auto_accept = AutoAcceptProofData {
        identity_id: proof_identity_id,
        proof_key,
        account_reference: input.proof_account_reference,
        expires_at: input.proof_expires_at,
    };

    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequestWithProof {
        identity: qi,
        signing_key,
        to_identity_id,
        account_label: input.account_label,
        qr_auto_accept,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Accept a contact request.
#[tauri::command]
#[specta::specta]
pub fn dashpay_accept_contact_request(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: AcceptContactRequestInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let request_id = parse_identifier(&input.request_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
        identity: qi,
        request_id,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Reject a contact request.
#[tauri::command]
#[specta::specta]
pub fn dashpay_reject_contact_request(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RejectContactRequestInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let request_id = parse_identifier(&input.request_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest {
        identity: qi,
        request_id,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Load payment history for an identity.
#[tauri::command]
#[specta::specta]
pub fn dashpay_load_payment_history(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: LoadPaymentHistoryInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory { identity: qi }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Send a payment to a contact.
#[tauri::command]
#[specta::specta]
pub fn dashpay_send_payment_to_contact(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SendPaymentToContactInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let contact_id = parse_identifier(&input.contact_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendPaymentToContact {
        identity: qi,
        contact_id,
        amount_dash: input.amount_dash,
        memo: input.memo,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Update contact info (nickname, note, hidden status).
#[tauri::command]
#[specta::specta]
pub fn dashpay_update_contact_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: UpdateContactInfoInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let contact_id = parse_identifier(&input.contact_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo {
        identity: qi,
        contact_id,
        nickname: input.nickname,
        note: input.note,
        is_hidden: input.is_hidden,
        accepted_accounts: input.accepted_accounts,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Register DashPay receiving addresses for incoming payment detection.
#[tauri::command]
#[specta::specta]
pub fn dashpay_register_addresses(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RegisterDashPayAddressesInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::DashPayTask(Box::new(DashPayTask::RegisterDashPayAddresses {
        identity: qi,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Direct database commands
// ---------------------------------------------------------------------------

/// Load a locally stored DashPay profile.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_load_profile(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
) -> Result<Option<StoredProfileDto>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let db = state.db();
    let ctx = state.current_context();
    let network_str = format!("{:?}", ctx.network());
    let profile = db
        .load_dashpay_profile(&identifier, &network_str)
        .map_err(|e| format!("Failed to load profile: {e}"))?;
    Ok(profile.map(|p| StoredProfileDto {
        identity_id: hex::encode(&p.identity_id),
        display_name: p.display_name,
        bio: p.bio,
        avatar_url: p.avatar_url,
        public_message: p.public_message,
        created_at: p.created_at,
        updated_at: p.updated_at,
    }))
}

/// Save a DashPay profile locally.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_save_profile(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
    display_name: Option<String>,
    bio: Option<String>,
    avatar_url: Option<String>,
    public_message: Option<String>,
) -> Result<(), String> {
    let identifier = parse_identifier(&identity_id)?;
    let db = state.db();
    let ctx = state.current_context();
    let network_str = format!("{:?}", ctx.network());
    db.save_dashpay_profile(
        &identifier,
        &network_str,
        display_name.as_deref(),
        bio.as_deref(),
        avatar_url.as_deref(),
        public_message.as_deref(),
    )
    .map_err(|e| format!("Failed to save profile: {e}"))
}

/// Load all locally stored contacts for an identity.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_load_contacts(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
) -> Result<Vec<StoredContactDto>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let db = state.db();
    let ctx = state.current_context();
    let network_str = format!("{:?}", ctx.network());
    let contacts = db
        .load_dashpay_contacts(&identifier, &network_str)
        .map_err(|e| format!("Failed to load contacts: {e}"))?;
    Ok(contacts
        .into_iter()
        .map(|c| StoredContactDto {
            owner_identity_id: hex::encode(&c.owner_identity_id),
            contact_identity_id: hex::encode(&c.contact_identity_id),
            username: c.username,
            display_name: c.display_name,
            avatar_url: c.avatar_url,
            public_message: c.public_message,
            contact_status: c.contact_status,
            created_at: c.created_at,
            updated_at: c.updated_at,
            last_seen: c.last_seen,
        })
        .collect())
}

/// Load pending contact requests for an identity.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_load_pending_requests(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
    request_type: String,
) -> Result<Vec<StoredContactRequestDto>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let db = state.db();
    let ctx = state.current_context();
    let network_str = format!("{:?}", ctx.network());
    let requests = db
        .load_pending_contact_requests(&identifier, &network_str, &request_type)
        .map_err(|e| format!("Failed to load pending requests: {e}"))?;
    Ok(requests
        .into_iter()
        .map(|r| StoredContactRequestDto {
            id: r.id,
            from_identity_id: hex::encode(&r.from_identity_id),
            to_identity_id: hex::encode(&r.to_identity_id),
            to_username: r.to_username,
            from_username: r.from_username,
            account_label: r.account_label,
            request_type: r.request_type,
            status: r.status,
            created_at: r.created_at,
            responded_at: r.responded_at,
            expires_at: r.expires_at,
            platform_document_id: r.platform_document_id.map(|bytes| hex::encode(&bytes)),
        })
        .collect())
}

/// Load payment history from local database.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_load_payments(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
    limit: u32,
) -> Result<Vec<StoredPaymentDto>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let db = state.db();
    let payments = db
        .load_payment_history(&identifier, limit)
        .map_err(|e| format!("Failed to load payment history: {e}"))?;
    Ok(payments
        .into_iter()
        .map(|p| StoredPaymentDto {
            id: p.id,
            tx_id: p.tx_id,
            from_identity_id: hex::encode(&p.from_identity_id),
            to_identity_id: hex::encode(&p.to_identity_id),
            amount: p.amount,
            memo: p.memo,
            payment_type: p.payment_type,
            status: p.status,
            created_at: p.created_at,
            confirmed_at: p.confirmed_at,
        })
        .collect())
}

/// Load private contact info (nickname, notes, hidden).
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_load_contact_private_info(
    state: tauri::State<'_, Arc<AppState>>,
    owner_identity_id: IdentifierDto,
    contact_identity_id: IdentifierDto,
) -> Result<ContactPrivateInfoDto, String> {
    let owner = parse_identifier(&owner_identity_id)?;
    let contact = parse_identifier(&contact_identity_id)?;
    let db = state.db();
    let (nickname, notes, is_hidden) = db
        .load_contact_private_info(&owner, &contact)
        .map_err(|e| format!("Failed to load contact private info: {e}"))?;
    Ok(ContactPrivateInfoDto {
        nickname,
        notes,
        is_hidden,
    })
}

/// Save private contact info.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_save_contact_private_info(
    state: tauri::State<'_, Arc<AppState>>,
    owner_identity_id: IdentifierDto,
    contact_identity_id: IdentifierDto,
    nickname: String,
    notes: String,
    is_hidden: bool,
) -> Result<(), String> {
    let owner = parse_identifier(&owner_identity_id)?;
    let contact = parse_identifier(&contact_identity_id)?;
    let db = state.db();
    db.save_contact_private_info(&owner, &contact, &nickname, &notes, is_hidden)
        .map_err(|e| format!("Failed to save contact private info: {e}"))
}

/// Set contact hidden status.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_set_contact_hidden(
    state: tauri::State<'_, Arc<AppState>>,
    owner_identity_id: IdentifierDto,
    contact_identity_id: IdentifierDto,
    is_hidden: bool,
) -> Result<(), String> {
    let owner = parse_identifier(&owner_identity_id)?;
    let contact = parse_identifier(&contact_identity_id)?;
    let db = state.db();
    db.set_contact_hidden(&owner, &contact, is_hidden)
        .map_err(|e| format!("Failed to set contact hidden: {e}"))
}

/// Save a DashPay profile's avatar bytes.
#[tauri::command]
#[specta::specta]
pub fn dashpay_db_save_avatar_bytes(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
    avatar_bytes: Option<Vec<u8>>,
) -> Result<(), String> {
    let identifier = parse_identifier(&identity_id)?;
    let db = state.db();
    let ctx = state.current_context();
    let network_str = format!("{:?}", ctx.network());
    db.save_dashpay_profile_avatar_bytes(&identifier, &network_str, avatar_bytes.as_deref())
        .map_err(|e| format!("Failed to save avatar bytes: {e}"))
}

// ---------------------------------------------------------------------------
// QR Code commands
// ---------------------------------------------------------------------------

/// Generate a QR auto-accept proof for sharing.
#[tauri::command]
#[specta::specta]
pub fn dashpay_generate_auto_accept_proof(
    state: tauri::State<'_, Arc<AppState>>,
    input: GenerateAutoAcceptProofInput,
) -> Result<AutoAcceptProofResult, String> {
    use dash_evo_tool::backend_task::dashpay::auto_accept_proof::generate_auto_accept_proof;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;

    if input.validity_hours == 0 || input.validity_hours > 720 {
        return Err("Validity hours must be between 1 and 720".into());
    }

    let qi = lookup_identity(&state, &input.identity_id)?;
    let proof_data = generate_auto_accept_proof(&qi, input.account_index, input.validity_hours)?;

    Ok(AutoAcceptProofResult {
        qr_string: proof_data.to_qr_string(),
        identity_id: proof_data.identity_id.to_string(Encoding::Base58),
        account_reference: proof_data.account_reference,
        expires_at: proof_data.expires_at,
    })
}

/// Parse a QR code string into auto-accept proof data.
#[tauri::command]
#[specta::specta]
pub fn dashpay_parse_auto_accept_proof(
    input: ParseAutoAcceptProofInput,
) -> Result<ParsedAutoAcceptProofResult, String> {
    use dash_evo_tool::backend_task::dashpay::auto_accept_proof::AutoAcceptProofData;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;

    let parsed = AutoAcceptProofData::from_qr_string(&input.qr_data)?;

    Ok(ParsedAutoAcceptProofResult {
        identity_id: parsed.identity_id.to_string(Encoding::Base58),
        proof_key_hex: hex::encode(parsed.proof_key),
        account_reference: parsed.account_reference,
        expires_at: parsed.expires_at,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_profile_input_serializes() {
        let input = LoadProfileInput {
            identity_id: "abc123".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
    }

    #[test]
    fn update_profile_input_serializes() {
        let input = UpdateProfileInput {
            identity_id: "abc123".into(),
            display_name: Some("Alice".into()),
            bio: Some("Hello".into()),
            avatar_url: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"displayName\":\"Alice\""));
        assert!(json.contains("\"bio\":\"Hello\""));
        assert!(json.contains("\"avatarUrl\":null"));
    }

    #[test]
    fn send_contact_request_input_serializes() {
        let input = SendContactRequestInput {
            identity_id: "abc".into(),
            signing_key_id: 1,
            to_username: "bob.dash".into(),
            account_label: Some("Main".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"signingKeyId\":1"));
        assert!(json.contains("\"toUsername\":\"bob.dash\""));
        assert!(json.contains("\"accountLabel\":\"Main\""));
    }

    #[test]
    fn send_contact_request_with_proof_input_serializes() {
        let input = SendContactRequestWithProofInput {
            identity_id: "abc".into(),
            signing_key_id: 2,
            to_identity_id: "def".into(),
            account_label: None,
            proof_identity_id: "ghi".into(),
            proof_key_hex: "aa".repeat(32),
            proof_account_reference: 42,
            proof_expires_at: 1234567890,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"proofAccountReference\":42"));
        assert!(json.contains("\"proofExpiresAt\":1234567890"));
    }

    #[test]
    fn send_payment_input_serializes() {
        let input = SendPaymentToContactInput {
            identity_id: "abc".into(),
            contact_id: "def".into(),
            amount_dash: 1.5,
            memo: Some("Lunch".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"amountDash\":1.5"));
        assert!(json.contains("\"memo\":\"Lunch\""));
    }

    #[test]
    fn update_contact_info_input_serializes() {
        let input = UpdateContactInfoInput {
            identity_id: "abc".into(),
            contact_id: "def".into(),
            nickname: Some("Bobby".into()),
            note: Some("Friend".into()),
            is_hidden: false,
            accepted_accounts: vec![0, 1],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"nickname\":\"Bobby\""));
        assert!(json.contains("\"isHidden\":false"));
        assert!(json.contains("\"acceptedAccounts\":[0,1]"));
    }

    #[test]
    fn stored_profile_dto_serializes() {
        let dto = StoredProfileDto {
            identity_id: "abc".into(),
            display_name: Some("Alice".into()),
            bio: None,
            avatar_url: None,
            public_message: None,
            created_at: 1000,
            updated_at: 2000,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"createdAt\":1000"));
        assert!(json.contains("\"updatedAt\":2000"));
    }

    #[test]
    fn stored_contact_dto_serializes() {
        let dto = StoredContactDto {
            owner_identity_id: "aaa".into(),
            contact_identity_id: "bbb".into(),
            username: Some("bob".into()),
            display_name: None,
            avatar_url: None,
            public_message: None,
            contact_status: "accepted".into(),
            created_at: 100,
            updated_at: 200,
            last_seen: Some(150),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"contactStatus\":\"accepted\""));
        assert!(json.contains("\"lastSeen\":150"));
    }

    #[test]
    fn stored_payment_dto_serializes() {
        let dto = StoredPaymentDto {
            id: 1,
            tx_id: "txhash".into(),
            from_identity_id: "aaa".into(),
            to_identity_id: "bbb".into(),
            amount: 100000,
            memo: Some("Test".into()),
            payment_type: "sent".into(),
            status: "confirmed".into(),
            created_at: 1000,
            confirmed_at: Some(2000),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"txId\":\"txhash\""));
        assert!(json.contains("\"paymentType\":\"sent\""));
        assert!(json.contains("\"confirmedAt\":2000"));
    }

    #[test]
    fn contact_private_info_dto_serializes() {
        let dto = ContactPrivateInfoDto {
            nickname: "Bobby".into(),
            notes: "Good friend".into(),
            is_hidden: false,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"nickname\":\"Bobby\""));
        assert!(json.contains("\"isHidden\":false"));
    }
}
