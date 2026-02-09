//! Identity-related Tauri IPC commands.
//!
//! Maps all 16 `IdentityTask` variants plus direct database methods to
//! Tauri commands. Long-running operations are dispatched asynchronously
//! via `task_dispatcher::dispatch_task` and results arrive as events.
//! Short reads (DB queries) return directly.

use crate::dto::common::{CreditsDto, IdentifierDto, WalletSeedHashDto};
use crate::dto::identity::{
    DpnsNameInfoDto, IdentityKeyDto, IdentityStatusDto, IdentitySummaryDto, IdentityTypeDto,
    QualifiedIdentityDto, TopUpEntryDto,
};
use crate::dto::NetworkDto;
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::identity::{
    IdentityInputToLoad, IdentityTask, RegisterDpnsNameInput,
};
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_evo_tool::model::wallet::WalletSeedHash;

use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs — serializable command parameters from the frontend
// ---------------------------------------------------------------------------

/// Input for loading an existing identity by ID or from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadIdentityInput {
    /// Identity ID as hex or base58 string.
    pub identity_id: String,
    /// Type of identity (User, Masternode, Evonode).
    pub identity_type: IdentityTypeDto,
    /// User alias to assign.
    pub alias: String,
    /// Voting private key (hex or WIF, empty string if none).
    pub voting_private_key: String,
    /// Owner private key (hex or WIF, empty string if none).
    pub owner_private_key: String,
    /// Payout address private key (hex or WIF, empty string if none).
    pub payout_address_private_key: String,
    /// Additional private keys (hex or WIF).
    pub keys: Vec<String>,
    /// Whether to try deriving keys from loaded wallets.
    pub derive_keys_from_wallets: bool,
    /// Optional wallet seed hash to associate with.
    pub selected_wallet_seed_hash: Option<WalletSeedHashDto>,
}

/// Input for searching identity by DPNS name.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchIdentityByDpnsNameInput {
    /// DPNS name without .dash suffix.
    pub name: String,
    /// Optional wallet seed hash for key derivation.
    pub wallet_seed_hash: Option<WalletSeedHashDto>,
}

/// Input for searching identities from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchIdentityFromWalletInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Identity index to search at.
    pub identity_index: u32,
}

/// Input for batch search of identities from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchIdentitiesUpToIndexInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Maximum identity index to search up to.
    pub max_identity_index: u32,
}

/// Input for registering a DPNS name.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDpnsNameCommandInput {
    /// Identity ID (hex) that will own the name.
    pub identity_id: IdentifierDto,
    /// The DPNS name to register (without .dash suffix).
    pub name: String,
}

/// Input for refreshing a single identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefreshIdentityInput {
    /// Identity ID (hex) to refresh.
    pub identity_id: IdentifierDto,
}

/// Input for withdrawing credits from an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawFromIdentityInput {
    /// Identity ID (hex) to withdraw from.
    pub identity_id: IdentifierDto,
    /// Optional destination Core address.
    pub to_address: Option<String>,
    /// Amount to withdraw in credits.
    pub credits: CreditsDto,
    /// Optional key ID to use for signing.
    pub key_id: Option<u32>,
}

/// Input for transferring credits to another identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferCreditsInput {
    /// Source identity ID (hex).
    pub from_identity_id: IdentifierDto,
    /// Destination identity ID (hex).
    pub to_identity_id: IdentifierDto,
    /// Amount to transfer in credits.
    pub credits: CreditsDto,
    /// Optional key ID to use for signing.
    pub key_id: Option<u32>,
}

/// Input for adding a key to an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddKeyToIdentityInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key type (e.g., "ECDSA_SECP256K1", "BLS12_381").
    pub key_type: String,
    /// Key purpose (e.g., "AUTHENTICATION", "VOTING", "TRANSFER").
    pub purpose: String,
    /// Security level (e.g., "HIGH", "MEDIUM").
    pub security_level: String,
    /// Private key as hex string (32 bytes).
    pub private_key_hex: String,
}

/// Input for disabling keys on an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DisableKeysInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key IDs to disable.
    pub key_ids: Vec<u32>,
}

/// Input for replacing a key on an identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceKeyInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// Key ID to replace (old key).
    pub old_key_id: u32,
    /// New key type.
    pub new_key_type: String,
    /// New key purpose.
    pub new_purpose: String,
    /// New security level.
    pub new_security_level: String,
    /// New private key as hex string (32 bytes).
    pub new_private_key_hex: String,
}

/// Input for setting identity alias.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetIdentityAliasInput {
    /// Identity ID (hex).
    pub identity_id: IdentifierDto,
    /// New alias (None to clear).
    pub alias: Option<String>,
}

/// Input for saving custom identity ordering.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveIdentityOrderInput {
    /// Ordered list of identity IDs (hex).
    pub identity_ids: Vec<IdentifierDto>,
}

/// Input for deleting a local identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteIdentityInput {
    /// Identity ID (hex) to delete.
    pub identity_id: IdentifierDto,
}

// ---------------------------------------------------------------------------
// Helper: Convert QualifiedIdentity → QualifiedIdentityDto
// ---------------------------------------------------------------------------

/// Convert a backend `QualifiedIdentity` into a serializable DTO.
///
/// Note: The `network` field is set to `Dash` by default since `QualifiedIdentity`
/// does not carry network information. Callers should override if needed.
pub fn qualified_identity_to_dto(qi: &QualifiedIdentity) -> QualifiedIdentityDto {
    let identity = &qi.identity;
    let id_hex = hex::encode(identity.id().to_vec());

    let keys: Vec<IdentityKeyDto> = identity
        .public_keys()
        .values()
        .map(|key| {
            let has_private = qi
                .private_keys
                .private_keys
                .values()
                .any(|qualified_key_pair| {
                    qualified_key_pair.0.identity_public_key.id() == key.id()
                });

            IdentityKeyDto {
                key_id: key.id(),
                key_type: format!("{:?}", key.key_type()),
                purpose: format!("{:?}", key.purpose()),
                security_level: format!("{:?}", key.security_level()),
                data: hex::encode(key.data().as_slice()),
                is_disabled: key.is_disabled(),
                disabled_at: key.disabled_at(),
                has_private_key: has_private,
            }
        })
        .collect();

    let dpns_names: Vec<DpnsNameInfoDto> = qi
        .dpns_names
        .iter()
        .map(|info| DpnsNameInfoDto {
            name: info.name.clone(),
            acquired_at: info.acquired_at,
        })
        .collect();

    let associated_wallet_hashes: Vec<WalletSeedHashDto> =
        qi.associated_wallets.keys().map(hex::encode).collect();

    let top_ups: Vec<TopUpEntryDto> = qi
        .top_ups
        .iter()
        .map(|(index, amount)| TopUpEntryDto {
            index: *index,
            amount: *amount,
        })
        .collect();

    let identity_type = match qi.identity_type {
        IdentityType::User => IdentityTypeDto::User,
        IdentityType::Masternode => IdentityTypeDto::Masternode,
        IdentityType::Evonode => IdentityTypeDto::Evonode,
    };

    QualifiedIdentityDto {
        id: id_hex,
        identity_type,
        alias: qi.alias.clone(),
        balance: identity.balance(),
        keys,
        dpns_names,
        associated_wallet_hashes,
        wallet_index: qi.wallet_index,
        top_ups,
        status: IdentityStatusDto::Active, // If loaded, it's active
        network: NetworkDto::Dash, // Network not available on QualifiedIdentity; callers override
        voter_identity_id: qi
            .associated_voter_identity
            .as_ref()
            .map(|(voter_identity, _)| hex::encode(voter_identity.id().to_vec())),
        operator_identity_id: qi
            .associated_operator_identity
            .as_ref()
            .map(|(operator_identity, _)| hex::encode(operator_identity.id().to_vec())),
    }
}

// ---------------------------------------------------------------------------
// Helper: Parse identifier from hex string
// ---------------------------------------------------------------------------

pub fn parse_identifier(hex_str: &str) -> Result<Identifier, String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid hex identifier: {e}"))?;
    Identifier::from_bytes(&bytes).map_err(|e| format!("Invalid identifier: {e}"))
}

fn parse_wallet_seed_hash(hex_str: &str) -> Result<WalletSeedHash, String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid wallet seed hash hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Wallet seed hash must be 32 bytes, got {}",
            bytes.len()
        ));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

fn parse_private_key_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid private key hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("Private key must be 32 bytes, got {}", bytes.len()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Look up a `QualifiedIdentity` from the local database by identifier.
fn lookup_identity(state: &AppState, identity_id: &str) -> Result<QualifiedIdentity, String> {
    let identifier = parse_identifier(identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_by_id(&identifier)
        .map_err(|e| format!("Database error loading identity: {e}"))?
        .ok_or_else(|| format!("Identity {} not found in local database", identity_id))
}

/// Look up a wallet Arc ref by seed hash.
fn lookup_wallet_arc_ref(
    state: &AppState,
    wallet_seed_hash_hex: &str,
) -> Result<dash_evo_tool::model::wallet::WalletArcRef, String> {
    use dash_evo_tool::model::wallet::WalletArcRef;
    let seed_hash = parse_wallet_seed_hash(wallet_seed_hash_hex)?;
    let ctx = state.current_context();
    let wallet_arc = ctx
        .wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("Wallet not found for seed hash {}", wallet_seed_hash_hex))?;
    Ok(WalletArcRef::from(wallet_arc))
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Load an existing identity by ID, optionally associating with wallets.
///
/// Dispatches `IdentityTask::LoadIdentity`. Result arrives via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn identity_load(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: LoadIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let identity_type = match input.identity_type {
        IdentityTypeDto::User => IdentityType::User,
        IdentityTypeDto::Masternode => IdentityType::Masternode,
        IdentityTypeDto::Evonode => IdentityType::Evonode,
    };

    let selected_wallet_seed_hash = input
        .selected_wallet_seed_hash
        .map(|h| parse_wallet_seed_hash(&h))
        .transpose()?;

    let load_input = IdentityInputToLoad {
        identity_id_input: input.identity_id,
        identity_type,
        alias_input: input.alias,
        voting_private_key_input: input.voting_private_key,
        owner_private_key_input: input.owner_private_key,
        payout_address_private_key_input: input.payout_address_private_key,
        keys_input: input.keys,
        derive_keys_from_wallets: input.derive_keys_from_wallets,
        selected_wallet_seed_hash,
    };

    let task = BackendTask::IdentityTask(IdentityTask::LoadIdentity(load_input));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Search for an identity by DPNS name.
///
/// Dispatches `IdentityTask::SearchIdentityByDpnsName`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_search_by_dpns_name(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchIdentityByDpnsNameInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet_seed_hash = input
        .wallet_seed_hash
        .map(|h| parse_wallet_seed_hash(&h))
        .transpose()?;

    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
        input.name,
        wallet_seed_hash,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Search for an identity from a wallet at a specific index.
///
/// Dispatches `IdentityTask::SearchIdentityFromWallet`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_search_from_wallet(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchIdentityFromWalletInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet_ref = lookup_wallet_arc_ref(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityFromWallet(
        wallet_ref,
        input.identity_index,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Batch search for identities from a wallet up to a max index.
///
/// Dispatches `IdentityTask::SearchIdentitiesUpToIndex`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_search_up_to_index(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SearchIdentitiesUpToIndexInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet_ref = lookup_wallet_arc_ref(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentitiesUpToIndex(
        wallet_ref,
        input.max_identity_index,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Register a DPNS name for an identity.
///
/// Dispatches `IdentityTask::RegisterDpnsName`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_register_dpns_name(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RegisterDpnsNameCommandInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
        qualified_identity: qi,
        name_input: input.name,
    }));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Refresh a single identity's state from Platform.
///
/// Dispatches `IdentityTask::RefreshIdentity`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_refresh(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RefreshIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let task = BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Refresh all loaded identities' DPNS names.
///
/// Dispatches `IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_refresh_dpns_names(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::IdentityTask(IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Withdraw credits from an identity to a Core address.
///
/// Dispatches `IdentityTask::WithdrawFromIdentity`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_withdraw(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: WithdrawFromIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let to_address = input
        .to_address
        .map(|addr_str| {
            addr_str
                    .parse::<dash_sdk::dpp::dashcore::Address<
                        dash_sdk::dpp::dashcore::address::NetworkUnchecked,
                    >>()
                    .map_err(|e| format!("Invalid address: {e}"))
                    .map(|a| a.assume_checked())
        })
        .transpose()?;
    let key_id: Option<KeyID> = input.key_id.map(|id| id as KeyID);

    let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
        qi,
        to_address,
        input.credits as Credits,
        key_id,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Transfer credits from one identity to another.
///
/// Dispatches `IdentityTask::Transfer`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_transfer(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: TransferCreditsInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.from_identity_id)?;
    let to_id = parse_identifier(&input.to_identity_id)?;
    let key_id: Option<KeyID> = input.key_id.map(|id| id as KeyID);

    let task = BackendTask::IdentityTask(IdentityTask::Transfer(
        qi,
        to_id,
        input.credits as Credits,
        key_id,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Add a key to an identity.
///
/// Dispatches `IdentityTask::AddKeyToIdentity`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_add_key(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: AddKeyToIdentityInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let private_key = parse_private_key_hex(&input.private_key_hex)?;

    let key_type = parse_key_type(&input.key_type)?;
    let purpose = parse_purpose(&input.purpose)?;
    let security_level = parse_security_level(&input.security_level)?;

    // Derive the public key data from the private key
    let public_key_data = derive_public_key_data(key_type, &private_key)?;

    // Construct the IdentityPublicKey with a temporary ID (backend will reassign)
    let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0, // Backend will set the correct ID
        key_type,
        purpose,
        security_level,
        contract_bounds: None,
        read_only: false,
        data: public_key_data.into(),
        disabled_at: None,
    });

    let qualified_public_key = QualifiedIdentityPublicKey {
        identity_public_key,
        in_wallet_at_derivation_path: None,
    };

    let task = BackendTask::IdentityTask(IdentityTask::AddKeyToIdentity(
        qi,
        qualified_public_key,
        private_key,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Disable one or more keys on an identity.
///
/// Dispatches `IdentityTask::DisableKeys`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_disable_keys(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: DisableKeysInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let key_ids: Vec<KeyID> = input.key_ids.iter().map(|&id| id as KeyID).collect();

    let task = BackendTask::IdentityTask(IdentityTask::DisableKeys(qi, key_ids));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Replace a key on an identity (disable old + add new atomically).
///
/// Dispatches `IdentityTask::ReplaceKey`. Result via event.
#[tauri::command]
#[specta::specta]
pub fn identity_replace_key(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: ReplaceKeyInput,
) -> Result<DispatchTaskResponse, String> {
    let qi = lookup_identity(&state, &input.identity_id)?;
    let new_private_key = parse_private_key_hex(&input.new_private_key_hex)?;

    let key_type = parse_key_type(&input.new_key_type)?;
    let purpose = parse_purpose(&input.new_purpose)?;
    let security_level = parse_security_level(&input.new_security_level)?;

    // Derive the public key data from the new private key
    let public_key_data = derive_public_key_data(key_type, &new_private_key)?;

    let identity_public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 0, // Backend will set the correct ID
        key_type,
        purpose,
        security_level,
        contract_bounds: None,
        read_only: false,
        data: public_key_data.into(),
        disabled_at: None,
    });

    let new_qualified_key = QualifiedIdentityPublicKey {
        identity_public_key,
        in_wallet_at_derivation_path: None,
    };

    let task = BackendTask::IdentityTask(IdentityTask::ReplaceKey(
        qi,
        input.old_key_id as KeyID,
        new_qualified_key,
        new_private_key,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Direct database read commands (synchronous, return immediately)
// ---------------------------------------------------------------------------

/// Load all local qualified identities from the database.
#[tauri::command]
#[specta::specta]
pub fn identity_list_local(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<QualifiedIdentityDto>, String> {
    let ctx = state.current_context();
    let identities = ctx
        .load_local_qualified_identities()
        .map_err(|e| format!("Failed to load identities: {e}"))?;

    Ok(identities.iter().map(qualified_identity_to_dto).collect())
}

/// Load all local user identities (non-masternode/evonode).
#[tauri::command]
#[specta::specta]
pub fn identity_list_user(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<QualifiedIdentityDto>, String> {
    let ctx = state.current_context();
    let identities = ctx
        .load_local_user_identities()
        .map_err(|e| format!("Failed to load user identities: {e}"))?;

    Ok(identities.iter().map(qualified_identity_to_dto).collect())
}

/// Load all local voting identities.
#[tauri::command]
#[specta::specta]
pub fn identity_list_voting(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<QualifiedIdentityDto>, String> {
    let ctx = state.current_context();
    let identities = ctx
        .load_local_voting_identities()
        .map_err(|e| format!("Failed to load voting identities: {e}"))?;

    Ok(identities.iter().map(qualified_identity_to_dto).collect())
}

/// Get a single identity by its ID.
#[tauri::command]
#[specta::specta]
pub fn identity_get_by_id(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
) -> Result<Option<QualifiedIdentityDto>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let ctx = state.current_context();
    let identity = ctx
        .get_identity_by_id(&identifier)
        .map_err(|e| format!("Failed to get identity: {e}"))?;

    Ok(identity.as_ref().map(qualified_identity_to_dto))
}

/// Set the alias for an identity.
#[tauri::command]
#[specta::specta]
pub fn identity_set_alias(
    state: tauri::State<'_, Arc<AppState>>,
    input: SetIdentityAliasInput,
) -> Result<(), String> {
    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    ctx.set_identity_alias(&identifier, input.alias.as_deref())
        .map_err(|e| format!("Failed to set identity alias: {e}"))
}

/// Get the alias for an identity.
#[tauri::command]
#[specta::specta]
pub fn identity_get_alias(
    state: tauri::State<'_, Arc<AppState>>,
    identity_id: IdentifierDto,
) -> Result<Option<String>, String> {
    let identifier = parse_identifier(&identity_id)?;
    let ctx = state.current_context();
    ctx.get_identity_alias(&identifier)
        .map_err(|e| format!("Failed to get identity alias: {e}"))
}

/// Load the custom identity ordering.
#[tauri::command]
#[specta::specta]
pub fn identity_load_order(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<IdentifierDto>, String> {
    let db = state.db();
    let ids = db
        .load_identity_order()
        .map_err(|e| format!("Failed to load identity order: {e}"))?;
    Ok(ids.iter().map(|id| hex::encode(id.to_vec())).collect())
}

/// Save the custom identity ordering.
#[tauri::command]
#[specta::specta]
pub fn identity_save_order(
    state: tauri::State<'_, Arc<AppState>>,
    input: SaveIdentityOrderInput,
) -> Result<(), String> {
    let ids: Vec<Identifier> = input
        .identity_ids
        .iter()
        .map(|hex_str| parse_identifier(hex_str))
        .collect::<Result<Vec<_>, _>>()?;
    let db = state.db();
    db.save_identity_order(ids)
        .map_err(|e| format!("Failed to save identity order: {e}"))
}

/// Delete a local identity from the database.
#[tauri::command]
#[specta::specta]
pub fn identity_delete(
    state: tauri::State<'_, Arc<AppState>>,
    input: DeleteIdentityInput,
) -> Result<(), String> {
    let identifier = parse_identifier(&input.identity_id)?;
    let ctx = state.current_context();
    let db = state.db();
    db.delete_local_qualified_identity(&identifier, ctx)
        .map_err(|e| format!("Failed to delete identity: {e}"))
}

/// Get identity summaries suitable for dropdown selectors.
#[tauri::command]
#[specta::specta]
pub fn identity_list_summaries(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<IdentitySummaryDto>, String> {
    let ctx = state.current_context();
    let identities = ctx
        .load_local_qualified_identities()
        .map_err(|e| format!("Failed to load identities: {e}"))?;

    Ok(identities
        .iter()
        .map(|qi| {
            let id_hex = hex::encode(qi.identity.id().to_vec());
            let display_name = qi.alias.clone().unwrap_or_else(|| truncate_id(&id_hex));

            IdentitySummaryDto {
                id: id_hex,
                display_name,
                identity_type: match qi.identity_type {
                    IdentityType::User => IdentityTypeDto::User,
                    IdentityType::Masternode => IdentityTypeDto::Masternode,
                    IdentityType::Evonode => IdentityTypeDto::Evonode,
                },
                balance: qi.identity.balance(),
            }
        })
        .collect())
}

/// Get all local DPNS names across all loaded identities.
#[tauri::command]
#[specta::specta]
pub fn identity_local_dpns_names(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<DpnsNameEntryDto>, String> {
    let ctx = state.current_context();
    let names = ctx
        .local_dpns_names()
        .map_err(|e| format!("Failed to load DPNS names: {e}"))?;

    Ok(names
        .iter()
        .map(|(id, info)| DpnsNameEntryDto {
            identity_id: hex::encode(id.to_vec()),
            name: info.name.clone(),
            acquired_at: info.acquired_at,
        })
        .collect())
}

/// A DPNS name entry with its owning identity.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DpnsNameEntryDto {
    /// Identity ID (hex) that owns this name.
    pub identity_id: IdentifierDto,
    /// The DPNS name (e.g., "alice.dash").
    pub name: String,
    /// Timestamp when the name was acquired.
    pub acquired_at: u64,
}

// ---------------------------------------------------------------------------
// Helper: String parsing for key types / purposes / security levels
// ---------------------------------------------------------------------------

/// Derive public key data from a private key for the given key type.
fn derive_public_key_data(key_type: KeyType, private_key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use dash_sdk::dpp::dashcore::key::Secp256k1;
    use dash_sdk::dpp::dashcore::PrivateKey;

    match key_type {
        KeyType::ECDSA_SECP256K1 => {
            let secp = Secp256k1::new();
            let secret_key = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(private_key)
                .map_err(|e| format!("Invalid private key: {e}"))?;
            let pk = PrivateKey::new(secret_key, dash_sdk::dpp::dashcore::Network::Dash);
            Ok(pk.public_key(&secp).to_bytes().to_vec())
        }
        KeyType::ECDSA_HASH160 => {
            let secp = Secp256k1::new();
            let secret_key = dash_sdk::dpp::dashcore::secp256k1::SecretKey::from_slice(private_key)
                .map_err(|e| format!("Invalid private key: {e}"))?;
            let pk = PrivateKey::new(secret_key, dash_sdk::dpp::dashcore::Network::Dash);
            Ok(pk.public_key(&secp).pubkey_hash().to_byte_array().to_vec())
        }
        _ => Err(format!(
            "Cannot derive public key for key type {:?} from raw private key bytes. \
            Only ECDSA_SECP256K1 and ECDSA_HASH160 are supported.",
            key_type
        )),
    }
}

fn parse_key_type(s: &str) -> Result<KeyType, String> {
    match s.to_uppercase().as_str() {
        "ECDSA_SECP256K1" => Ok(KeyType::ECDSA_SECP256K1),
        "BLS12_381" => Ok(KeyType::BLS12_381),
        "ECDSA_HASH160" => Ok(KeyType::ECDSA_HASH160),
        "BIP13_SCRIPT_HASH" => Ok(KeyType::BIP13_SCRIPT_HASH),
        "EDDSA_25519_HASH160" => Ok(KeyType::EDDSA_25519_HASH160),
        other => Err(format!("Unknown key type: {}", other)),
    }
}

fn parse_purpose(s: &str) -> Result<Purpose, String> {
    match s.to_uppercase().as_str() {
        "AUTHENTICATION" => Ok(Purpose::AUTHENTICATION),
        "ENCRYPTION" => Ok(Purpose::ENCRYPTION),
        "DECRYPTION" => Ok(Purpose::DECRYPTION),
        "TRANSFER" => Ok(Purpose::TRANSFER),
        "VOTING" => Ok(Purpose::VOTING),
        "OWNER" => Ok(Purpose::OWNER),
        other => Err(format!("Unknown key purpose: {}", other)),
    }
}

fn parse_security_level(s: &str) -> Result<SecurityLevel, String> {
    match s.to_uppercase().as_str() {
        "MASTER" => Ok(SecurityLevel::MASTER),
        "CRITICAL" => Ok(SecurityLevel::CRITICAL),
        "HIGH" => Ok(SecurityLevel::HIGH),
        "MEDIUM" => Ok(SecurityLevel::MEDIUM),
        other => Err(format!("Unknown security level: {}", other)),
    }
}

/// Truncate an identifier hex string for display (first 8 + last 4 chars).
fn truncate_id(hex: &str) -> String {
    if hex.len() <= 16 {
        hex.to_string()
    } else {
        format!("{}...{}", &hex[..8], &hex[hex.len() - 4..])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_identity_input_serializes_with_camel_case() {
        let input = LoadIdentityInput {
            identity_id: "abc123".into(),
            identity_type: IdentityTypeDto::User,
            alias: "Alice".into(),
            voting_private_key: String::new(),
            owner_private_key: String::new(),
            payout_address_private_key: String::new(),
            keys: vec![],
            derive_keys_from_wallets: true,
            selected_wallet_seed_hash: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"identityType\":\"user\""));
        assert!(json.contains("\"deriveKeysFromWallets\":true"));
        assert!(json.contains("\"selectedWalletSeedHash\":null"));
    }

    #[test]
    fn search_by_dpns_name_input_serializes() {
        let input = SearchIdentityByDpnsNameInput {
            name: "alice".into(),
            wallet_seed_hash: Some("deadbeef".repeat(4)),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"name\":\"alice\""));
        assert!(json.contains("\"walletSeedHash\""));
    }

    #[test]
    fn register_dpns_name_input_serializes() {
        let input = RegisterDpnsNameCommandInput {
            identity_id: "abc123".into(),
            name: "bob".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
        assert!(json.contains("\"name\":\"bob\""));
    }

    #[test]
    fn withdraw_input_serializes() {
        let input = WithdrawFromIdentityInput {
            identity_id: "abc123".into(),
            to_address: Some("XpYvN123".into()),
            credits: 50000,
            key_id: Some(2),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"toAddress\":\"XpYvN123\""));
        assert!(json.contains("\"credits\":50000"));
        assert!(json.contains("\"keyId\":2"));
    }

    #[test]
    fn transfer_input_serializes() {
        let input = TransferCreditsInput {
            from_identity_id: "aaa".into(),
            to_identity_id: "bbb".into(),
            credits: 100000,
            key_id: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"fromIdentityId\":\"aaa\""));
        assert!(json.contains("\"toIdentityId\":\"bbb\""));
        assert!(json.contains("\"keyId\":null"));
    }

    #[test]
    fn add_key_input_serializes() {
        let input = AddKeyToIdentityInput {
            identity_id: "abc".into(),
            key_type: "ECDSA_SECP256K1".into(),
            purpose: "AUTHENTICATION".into(),
            security_level: "HIGH".into(),
            private_key_hex: "aa".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyType\":\"ECDSA_SECP256K1\""));
        assert!(json.contains("\"purpose\":\"AUTHENTICATION\""));
        assert!(json.contains("\"securityLevel\":\"HIGH\""));
    }

    #[test]
    fn disable_keys_input_serializes() {
        let input = DisableKeysInput {
            identity_id: "abc".into(),
            key_ids: vec![1, 3, 5],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyIds\":[1,3,5]"));
    }

    #[test]
    fn replace_key_input_serializes() {
        let input = ReplaceKeyInput {
            identity_id: "abc".into(),
            old_key_id: 2,
            new_key_type: "BLS12_381".into(),
            new_purpose: "VOTING".into(),
            new_security_level: "MEDIUM".into(),
            new_private_key_hex: "bb".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"oldKeyId\":2"));
        assert!(json.contains("\"newKeyType\":\"BLS12_381\""));
    }

    #[test]
    fn set_alias_input_serializes() {
        let input = SetIdentityAliasInput {
            identity_id: "abc".into(),
            alias: Some("My Identity".into()),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"alias\":\"My Identity\""));
    }

    #[test]
    fn save_order_input_serializes() {
        let input = SaveIdentityOrderInput {
            identity_ids: vec!["aaa".into(), "bbb".into()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityIds\":[\"aaa\",\"bbb\"]"));
    }

    #[test]
    fn delete_identity_input_serializes() {
        let input = DeleteIdentityInput {
            identity_id: "abc".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
    }

    #[test]
    fn dpns_name_entry_serializes() {
        let entry = DpnsNameEntryDto {
            identity_id: "abc".into(),
            name: "alice.dash".into(),
            acquired_at: 1234567890,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"identityId\":\"abc\""));
        assert!(json.contains("\"name\":\"alice.dash\""));
        assert!(json.contains("\"acquiredAt\":1234567890"));
    }

    #[test]
    fn parse_identifier_valid_hex() {
        let hex_str = "a".repeat(64); // 32 bytes as hex
        let result = parse_identifier(&hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_identifier_invalid_hex() {
        let result = parse_identifier("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_wallet_seed_hash_valid() {
        let hex_str = "b".repeat(64);
        let result = parse_wallet_seed_hash(&hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_wallet_seed_hash_wrong_length() {
        let hex_str = "bb".repeat(16); // 16 bytes, not 32
        let result = parse_wallet_seed_hash(&hex_str);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("32 bytes"));
    }

    #[test]
    fn parse_private_key_hex_valid() {
        let hex_str = "cc".repeat(32);
        let result = parse_private_key_hex(&hex_str);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_key_type_variants() {
        assert!(parse_key_type("ECDSA_SECP256K1").is_ok());
        assert!(parse_key_type("BLS12_381").is_ok());
        assert!(parse_key_type("ECDSA_HASH160").is_ok());
        assert!(parse_key_type("BIP13_SCRIPT_HASH").is_ok());
        assert!(parse_key_type("EDDSA_25519_HASH160").is_ok());
        assert!(parse_key_type("ecdsa_secp256k1").is_ok()); // case insensitive
        assert!(parse_key_type("UNKNOWN").is_err());
    }

    #[test]
    fn parse_purpose_variants() {
        assert!(parse_purpose("AUTHENTICATION").is_ok());
        assert!(parse_purpose("ENCRYPTION").is_ok());
        assert!(parse_purpose("DECRYPTION").is_ok());
        assert!(parse_purpose("TRANSFER").is_ok());
        assert!(parse_purpose("VOTING").is_ok());
        assert!(parse_purpose("OWNER").is_ok());
        assert!(parse_purpose("voting").is_ok()); // case insensitive
        assert!(parse_purpose("UNKNOWN").is_err());
    }

    #[test]
    fn parse_security_level_variants() {
        assert!(parse_security_level("MASTER").is_ok());
        assert!(parse_security_level("CRITICAL").is_ok());
        assert!(parse_security_level("HIGH").is_ok());
        assert!(parse_security_level("MEDIUM").is_ok());
        assert!(parse_security_level("medium").is_ok()); // case insensitive
        assert!(parse_security_level("UNKNOWN").is_err());
    }

    #[test]
    fn truncate_id_short() {
        assert_eq!(truncate_id("abcdef"), "abcdef");
    }

    #[test]
    fn truncate_id_long() {
        let long_id = "a".repeat(64);
        let truncated = truncate_id(&long_id);
        assert_eq!(truncated, "aaaaaaaa...aaaa");
    }

    #[test]
    fn search_identity_from_wallet_input_serializes() {
        let input = SearchIdentityFromWalletInput {
            wallet_seed_hash: "aa".repeat(32),
            identity_index: 5,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
        assert!(json.contains("\"identityIndex\":5"));
    }

    #[test]
    fn search_identities_up_to_index_input_serializes() {
        let input = SearchIdentitiesUpToIndexInput {
            wallet_seed_hash: "bb".repeat(32),
            max_identity_index: 10,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"maxIdentityIndex\":10"));
    }

    #[test]
    fn refresh_identity_input_serializes() {
        let input = RefreshIdentityInput {
            identity_id: "abc123".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"identityId\":\"abc123\""));
    }

    #[test]
    fn identity_roundtrip_deserialization() {
        let json = r#"{
            "identityId": "abc123",
            "identityType": "masternode",
            "alias": "My MN",
            "votingPrivateKey": "",
            "ownerPrivateKey": "",
            "payoutAddressPrivateKey": "",
            "keys": [],
            "deriveKeysFromWallets": false,
            "selectedWalletSeedHash": null
        }"#;
        let input: LoadIdentityInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.identity_id, "abc123");
        assert_eq!(input.identity_type, IdentityTypeDto::Masternode);
        assert_eq!(input.alias, "My MN");
        assert!(!input.derive_keys_from_wallets);
    }
}
