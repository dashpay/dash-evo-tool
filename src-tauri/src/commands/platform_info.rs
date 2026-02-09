//! Platform info Tauri IPC commands.
//!
//! Maps all 8 `PlatformInfoTaskRequestType` variants to Tauri commands.

use crate::state::AppState;
use crate::task_dispatcher;
use crate::DispatchTaskResponse;

use dash_evo_tool::backend_task::platform_info::PlatformInfoTaskRequestType;
use dash_evo_tool::backend_task::BackendTask;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::AppHandle;

// ---------------------------------------------------------------------------
// Input DTOs
// ---------------------------------------------------------------------------

/// Input for fetching an address balance.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FetchAddressBalanceInput {
    pub address: String,
}

// ---------------------------------------------------------------------------
// Async dispatch commands
// ---------------------------------------------------------------------------

/// Fetch current epoch info from Platform.
#[tauri::command]
#[specta::specta]
pub fn platform_current_epoch_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::CurrentEpochInfo);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch total credits on Platform.
#[tauri::command]
#[specta::specta]
pub fn platform_total_credits(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::TotalCreditsOnPlatform);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch current version voting state.
#[tauri::command]
#[specta::specta]
pub fn platform_version_voting_state(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::CurrentVersionVotingState);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch current validator set info.
#[tauri::command]
#[specta::specta]
pub fn platform_validator_set_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::CurrentValidatorSetInfo);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch current withdrawals in queue.
#[tauri::command]
#[specta::specta]
pub fn platform_withdrawals_in_queue(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::CurrentWithdrawalsInQueue);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch recently completed withdrawals.
#[tauri::command]
#[specta::specta]
pub fn platform_recently_completed_withdrawals(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::RecentlyCompletedWithdrawals);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch basic platform info (version, chain lock height, network).
#[tauri::command]
#[specta::specta]
pub fn platform_basic_info(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::BasicPlatformInfo);
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

/// Fetch balance for a Platform address.
#[tauri::command]
#[specta::specta]
pub fn platform_fetch_address_balance(
    app_handle: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    input: FetchAddressBalanceInput,
) -> DispatchTaskResponse {
    let task = BackendTask::PlatformInfo(PlatformInfoTaskRequestType::FetchAddressBalance(
        input.address,
    ));
    let task_id = task_dispatcher::dispatch_task(&app_handle, &state, task);
    DispatchTaskResponse { task_id }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_address_balance_input_serializes() {
        let input = FetchAddressBalanceInput {
            address: "yXmqz...".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"address\":\"yXmqz...\""));
    }
}
