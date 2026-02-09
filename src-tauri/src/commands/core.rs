//! Core-related Tauri IPC commands.
//!
//! Maps all 10 `CoreTask` variants to Tauri commands. Long-running operations
//! are dispatched asynchronously via `task_dispatcher::dispatch_task` and
//! results arrive as events. This module also exposes wallet payment
//! operations (send from HD and single-key wallets), asset lock creation,
//! and Dash Core (DashQT) management.

use crate::dto::common::{CreditsDto, SingleKeyHashDto, WalletSeedHashDto};
use crate::dto::wallet::PaymentRecipientDto;
use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::wallet::PlatformSyncMode;
use dash_evo_tool::backend_task::BackendTask;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs — serializable command parameters from the frontend
// ---------------------------------------------------------------------------

/// Platform sync mode DTO for the IPC boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlatformSyncModeDto {
    Auto,
    ForceFull,
    TerminalOnly,
}

impl PlatformSyncModeDto {
    pub fn to_backend(self) -> PlatformSyncMode {
        match self {
            Self::Auto => PlatformSyncMode::Auto,
            Self::ForceFull => PlatformSyncMode::ForceFull,
            Self::TerminalOnly => PlatformSyncMode::TerminalOnly,
        }
    }
}

/// Input for refreshing an HD wallet from Core.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWalletInfoInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Optional platform sync mode. None = skip platform sync.
    pub platform_sync_mode: Option<PlatformSyncModeDto>,
}

/// Input for refreshing a single-key wallet from Core.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSingleKeyWalletInfoInput {
    /// Single-key wallet hash (hex).
    pub key_hash: SingleKeyHashDto,
}

/// Input for starting DashQT.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartDashQtInput {
    /// Path to the custom Dash-Qt binary.
    pub dash_qt_path: String,
    /// Whether to overwrite dash.conf.
    pub overwrite_dash_conf: bool,
}

/// Input for creating a registration asset lock.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateRegistrationAssetLockInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Amount in credits to lock.
    pub amount_credits: CreditsDto,
    /// Identity index for the registration.
    pub identity_index: u32,
}

/// Input for creating a top-up asset lock.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateTopUpAssetLockInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Amount in credits to lock.
    pub amount_credits: CreditsDto,
    /// Identity index.
    pub identity_index: u32,
    /// Top-up index within the identity.
    pub top_up_index: u32,
}

/// Input for sending a wallet payment (HD wallet).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendWalletPaymentInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
    /// Payment recipients.
    pub recipients: Vec<PaymentRecipientDto>,
    /// Whether to subtract the fee from the send amount.
    pub subtract_fee_from_amount: bool,
    /// Optional memo.
    pub memo: Option<String>,
    /// Optional override fee in duffs (for retry after min relay fee error).
    pub override_fee: Option<u64>,
}

/// Input for sending a payment from a single-key wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendSingleKeyWalletPaymentInput {
    /// Single-key wallet hash (hex).
    pub key_hash: SingleKeyHashDto,
    /// Payment recipients.
    pub recipients: Vec<PaymentRecipientDto>,
    /// Whether to subtract the fee from the send amount.
    pub subtract_fee_from_amount: bool,
    /// Optional memo.
    pub memo: Option<String>,
    /// Optional override fee in duffs.
    pub override_fee: Option<u64>,
}

/// Input for recovering asset locks from a wallet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoverAssetLocksInput {
    /// Wallet seed hash (hex).
    pub wallet_seed_hash: WalletSeedHashDto,
}

// ---------------------------------------------------------------------------
// Helper: parse wallet identifiers
// ---------------------------------------------------------------------------

fn parse_wallet_seed_hash(hex_str: &str) -> Result<[u8; 32], String> {
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

fn parse_single_key_hash(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes =
        hex::decode(hex_str).map_err(|e| format!("Invalid single-key wallet hash hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Single-key hash must be 32 bytes, got {}",
            bytes.len()
        ));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

fn lookup_hd_wallet(
    state: &AppState,
    seed_hash_hex: &str,
) -> Result<std::sync::Arc<std::sync::RwLock<dash_evo_tool::model::wallet::Wallet>>, String> {
    let seed_hash = parse_wallet_seed_hash(seed_hash_hex)?;
    let ctx = state.current_context();
    ctx.wallet_by_seed_hash(&seed_hash)
        .ok_or_else(|| format!("HD wallet not found for seed hash {}", seed_hash_hex))
}

fn lookup_single_key_wallet(
    state: &AppState,
    key_hash_hex: &str,
) -> Result<
    std::sync::Arc<std::sync::RwLock<dash_evo_tool::model::wallet::single_key::SingleKeyWallet>>,
    String,
> {
    let key_hash = parse_single_key_hash(key_hash_hex)?;
    let ctx = state.current_context();
    ctx.single_key_wallet_by_hash(&key_hash)
        .ok_or_else(|| format!("Single-key wallet not found for hash {}", key_hash_hex))
}

fn build_payment_request(
    recipients: Vec<PaymentRecipientDto>,
    subtract_fee_from_amount: bool,
    memo: Option<String>,
    override_fee: Option<u64>,
) -> Result<WalletPaymentRequest, String> {
    if recipients.is_empty() {
        return Err("At least one recipient is required".to_string());
    }
    let payment_recipients: Vec<PaymentRecipient> = recipients
        .into_iter()
        .map(|r| PaymentRecipient {
            address: r.address,
            amount_duffs: r.amount,
        })
        .collect();
    Ok(WalletPaymentRequest {
        recipients: payment_recipients,
        subtract_fee_from_amount,
        memo,
        override_fee,
    })
}

// ---------------------------------------------------------------------------
// Async dispatch commands (BackendTask-based, returns task ID)
// ---------------------------------------------------------------------------

/// Get the best chain lock for the active network.
///
/// Dispatches `CoreTask::GetBestChainLock`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_get_best_chain_lock(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::CoreTask(CoreTask::GetBestChainLock);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Get the best chain locks for all configured networks.
///
/// Dispatches `CoreTask::GetBestChainLocks`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_get_best_chain_locks(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::CoreTask(CoreTask::GetBestChainLocks);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Refresh an HD wallet's info from Core, optionally syncing Platform balances.
///
/// Dispatches `CoreTask::RefreshWalletInfo`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_refresh_wallet_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RefreshWalletInfoInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_hd_wallet(&state, &input.wallet_seed_hash)?;
    let sync_mode = input.platform_sync_mode.map(|m| m.to_backend());
    let task = BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet, sync_mode));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Refresh a single-key wallet's info from Core.
///
/// Dispatches `CoreTask::RefreshSingleKeyWalletInfo`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_refresh_single_key_wallet_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RefreshSingleKeyWalletInfoInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_single_key_wallet(&state, &input.key_hash)?;
    let task = BackendTask::CoreTask(CoreTask::RefreshSingleKeyWalletInfo(wallet));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Start DashQT on the active network.
///
/// Dispatches `CoreTask::StartDashQT`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_start_dash_qt(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: StartDashQtInput,
) -> DispatchTaskResponse {
    let network = state.active_network();
    let path = std::path::PathBuf::from(input.dash_qt_path);
    let task = BackendTask::CoreTask(CoreTask::StartDashQT(
        network,
        path,
        input.overwrite_dash_conf,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Create an asset lock for identity registration.
///
/// Dispatches `CoreTask::CreateRegistrationAssetLock`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_create_registration_asset_lock(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: CreateRegistrationAssetLockInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_hd_wallet(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::CoreTask(CoreTask::CreateRegistrationAssetLock(
        wallet,
        input.amount_credits,
        input.identity_index,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Create an asset lock for identity top-up.
///
/// Dispatches `CoreTask::CreateTopUpAssetLock`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_create_top_up_asset_lock(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: CreateTopUpAssetLockInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_hd_wallet(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::CoreTask(CoreTask::CreateTopUpAssetLock(
        wallet,
        input.amount_credits,
        input.identity_index,
        input.top_up_index,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Send a payment from an HD wallet.
///
/// Dispatches `CoreTask::SendWalletPayment`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_send_wallet_payment(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SendWalletPaymentInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_hd_wallet(&state, &input.wallet_seed_hash)?;
    let request = build_payment_request(
        input.recipients,
        input.subtract_fee_from_amount,
        input.memo,
        input.override_fee,
    )?;
    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment { wallet, request });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Send a payment from a single-key wallet.
///
/// Dispatches `CoreTask::SendSingleKeyWalletPayment`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_send_single_key_wallet_payment(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: SendSingleKeyWalletPaymentInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_single_key_wallet(&state, &input.key_hash)?;
    let request = build_payment_request(
        input.recipients,
        input.subtract_fee_from_amount,
        input.memo,
        input.override_fee,
    )?;
    let task = BackendTask::CoreTask(CoreTask::SendSingleKeyWalletPayment { wallet, request });
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

/// Recover asset locks from an HD wallet.
///
/// Dispatches `CoreTask::RecoverAssetLocks`. Result via `TaskResultEvent`.
#[tauri::command]
#[specta::specta]
pub fn core_recover_asset_locks(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: RecoverAssetLocksInput,
) -> Result<DispatchTaskResponse, String> {
    let wallet = lookup_hd_wallet(&state, &input.wallet_seed_hash)?;
    let task = BackendTask::CoreTask(CoreTask::RecoverAssetLocks(wallet));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    Ok(DispatchTaskResponse { task_id })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_sync_mode_dto_serializes() {
        let mode = PlatformSyncModeDto::Auto;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"auto\"");

        let mode = PlatformSyncModeDto::ForceFull;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"forceFull\"");

        let mode = PlatformSyncModeDto::TerminalOnly;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"terminalOnly\"");
    }

    #[test]
    fn platform_sync_mode_dto_roundtrip() {
        let modes = vec![
            PlatformSyncModeDto::Auto,
            PlatformSyncModeDto::ForceFull,
            PlatformSyncModeDto::TerminalOnly,
        ];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: PlatformSyncModeDto = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{:?}", mode), format!("{:?}", deserialized));
        }
    }

    #[test]
    fn refresh_wallet_info_input_serializes_with_camel_case() {
        let input = RefreshWalletInfoInput {
            wallet_seed_hash: "aa".repeat(32),
            platform_sync_mode: Some(PlatformSyncModeDto::Auto),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
        assert!(json.contains("\"platformSyncMode\":\"auto\""));
    }

    #[test]
    fn refresh_wallet_info_input_no_sync() {
        let input = RefreshWalletInfoInput {
            wallet_seed_hash: "bb".repeat(32),
            platform_sync_mode: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"platformSyncMode\":null"));
    }

    #[test]
    fn refresh_single_key_input_serializes() {
        let input = RefreshSingleKeyWalletInfoInput {
            key_hash: "cc".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"keyHash\""));
    }

    #[test]
    fn start_dash_qt_input_serializes() {
        let input = StartDashQtInput {
            dash_qt_path: "/usr/bin/dash-qt".into(),
            overwrite_dash_conf: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"dashQtPath\":\"/usr/bin/dash-qt\""));
        assert!(json.contains("\"overwriteDashConf\":true"));
    }

    #[test]
    fn create_registration_asset_lock_input_serializes() {
        let input = CreateRegistrationAssetLockInput {
            wallet_seed_hash: "aa".repeat(32),
            amount_credits: 100000,
            identity_index: 0,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"amountCredits\":100000"));
        assert!(json.contains("\"identityIndex\":0"));
    }

    #[test]
    fn create_top_up_asset_lock_input_serializes() {
        let input = CreateTopUpAssetLockInput {
            wallet_seed_hash: "aa".repeat(32),
            amount_credits: 50000,
            identity_index: 1,
            top_up_index: 3,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"topUpIndex\":3"));
    }

    #[test]
    fn send_wallet_payment_input_serializes() {
        let input = SendWalletPaymentInput {
            wallet_seed_hash: "aa".repeat(32),
            recipients: vec![PaymentRecipientDto {
                address: "XpYvN123".into(),
                amount: 100000,
            }],
            subtract_fee_from_amount: true,
            memo: Some("Test payment".into()),
            override_fee: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"subtractFeeFromAmount\":true"));
        assert!(json.contains("\"memo\":\"Test payment\""));
        assert!(json.contains("\"overrideFee\":null"));
        assert!(json.contains("\"recipients\""));
    }

    #[test]
    fn send_single_key_payment_input_serializes() {
        let input = SendSingleKeyWalletPaymentInput {
            key_hash: "bb".repeat(32),
            recipients: vec![
                PaymentRecipientDto {
                    address: "addr1".into(),
                    amount: 50000,
                },
                PaymentRecipientDto {
                    address: "addr2".into(),
                    amount: 75000,
                },
            ],
            subtract_fee_from_amount: false,
            memo: None,
            override_fee: Some(1000),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"overrideFee\":1000"));
        // Verify multiple recipients
        let deserialized: SendSingleKeyWalletPaymentInput = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.recipients.len(), 2);
    }

    #[test]
    fn recover_asset_locks_input_serializes() {
        let input = RecoverAssetLocksInput {
            wallet_seed_hash: "dd".repeat(32),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"walletSeedHash\""));
    }

    #[test]
    fn build_payment_request_empty_recipients_errors() {
        let result = build_payment_request(vec![], false, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("At least one recipient"));
    }

    #[test]
    fn build_payment_request_success() {
        let result = build_payment_request(
            vec![PaymentRecipientDto {
                address: "XpYvN123".into(),
                amount: 100000,
            }],
            true,
            Some("Test".into()),
            Some(500),
        );
        assert!(result.is_ok());
        let req = result.unwrap();
        assert_eq!(req.recipients.len(), 1);
        assert_eq!(req.recipients[0].address, "XpYvN123");
        assert_eq!(req.recipients[0].amount_duffs, 100000);
        assert!(req.subtract_fee_from_amount);
        assert_eq!(req.memo.unwrap(), "Test");
        assert_eq!(req.override_fee.unwrap(), 500);
    }

    #[test]
    fn parse_wallet_seed_hash_valid() {
        let hex = "aa".repeat(32);
        let result = parse_wallet_seed_hash(&hex);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_wallet_seed_hash_wrong_length() {
        let hex = "aa".repeat(16);
        let result = parse_wallet_seed_hash(&hex);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("32 bytes"));
    }

    #[test]
    fn parse_single_key_hash_valid() {
        let hex = "bb".repeat(32);
        let result = parse_single_key_hash(&hex);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_single_key_hash_invalid_hex() {
        let result = parse_single_key_hash("not-valid-hex");
        assert!(result.is_err());
    }

    #[test]
    fn send_wallet_payment_input_roundtrip() {
        let json = r#"{
            "walletSeedHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "recipients": [
                {"address": "XpYvN123", "amount": 100000},
                {"address": "XqRtM456", "amount": 200000}
            ],
            "subtractFeeFromAmount": false,
            "memo": null,
            "overrideFee": null
        }"#;
        let input: SendWalletPaymentInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.recipients.len(), 2);
        assert_eq!(input.recipients[0].address, "XpYvN123");
        assert_eq!(input.recipients[1].amount, 200000);
        assert!(!input.subtract_fee_from_amount);
    }
}
