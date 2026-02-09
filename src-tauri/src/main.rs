// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod commands;
pub mod dto;
pub mod events;
pub mod state;
pub mod task_dispatcher;

use dto::NetworkDto;
use events::{
    ScheduledVoteExecutedEvent, SpvStatusEvent, TaskErrorEvent, TaskResultEvent,
    WalletUpdatedEvent, ZmqChainLockedBlockEvent, ZmqConnectionStatusEvent,
    ZmqIsLockedTransactionEvent,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use specta_typescript::Typescript;
use state::AppState;
use std::sync::Arc;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

/// A sample DTO demonstrating tauri-specta type generation.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GreetResponse {
    pub message: String,
    pub timestamp_ms: u64,
}

/// Network availability info returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfo {
    /// Currently active network.
    pub active_network: NetworkDto,
    /// All networks that have valid configurations.
    pub available_networks: Vec<NetworkDto>,
}

/// Response from dispatching a backend task.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DispatchTaskResponse {
    /// Unique task ID. Listen for `TaskResultEvent` or `TaskErrorEvent` with
    /// this ID to receive the result.
    pub task_id: String,
}

#[tauri::command]
#[specta::specta]
fn greet(name: String) -> GreetResponse {
    GreetResponse {
        message: format!("Hello, {}! Welcome to Dash Evo Tool.", name),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    }
}

#[tauri::command]
#[specta::specta]
async fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get the current network state: active network and available networks.
#[tauri::command]
#[specta::specta]
fn get_network_info(state: tauri::State<'_, Arc<AppState>>) -> NetworkInfo {
    NetworkInfo {
        active_network: NetworkDto::from_network(state.active_network()),
        available_networks: state
            .available_networks()
            .into_iter()
            .map(NetworkDto::from_network)
            .collect(),
    }
}

/// Switch the active network. Returns `Ok(())` on success or an error if
/// the requested network has no valid configuration.
#[tauri::command]
#[specta::specta]
fn switch_network(
    network: NetworkDto,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let net = network.to_network();
    if state.switch_network(net) {
        Ok(())
    } else {
        Err(format!(
            "Network {:?} is not available — check your .env configuration.",
            network
        ))
    }
}

/// Get the SPV status for all available networks.
#[tauri::command]
#[specta::specta]
fn get_spv_status(state: tauri::State<'_, Arc<AppState>>) -> Vec<events::SpvStatusEvent> {
    let mut statuses = Vec::new();
    for network in state.available_networks() {
        let ctx = state.context_for_network(network);
        let network_dto = NetworkDto::from_network(network);
        let spv_manager = ctx.spv_manager();
        let snapshot = spv_manager.status();
        let status = match snapshot.status {
            dash_evo_tool::spv::SpvStatus::Idle => events::SpvStatusDto::Idle,
            dash_evo_tool::spv::SpvStatus::Starting => events::SpvStatusDto::Starting,
            dash_evo_tool::spv::SpvStatus::Syncing => events::SpvStatusDto::Syncing,
            dash_evo_tool::spv::SpvStatus::Running => events::SpvStatusDto::Running,
            dash_evo_tool::spv::SpvStatus::Stopping => events::SpvStatusDto::Stopping,
            dash_evo_tool::spv::SpvStatus::Stopped => events::SpvStatusDto::Stopped,
            dash_evo_tool::spv::SpvStatus::Error => events::SpvStatusDto::Error,
        };
        let sync_pct = None::<f64>; // Not computable without a target height
        let header_height = snapshot.sync_progress.as_ref().map(|p| p.header_height);
        statuses.push(events::SpvStatusEvent {
            network: network_dto,
            status,
            sync_progress_pct: sync_pct,
            header_height,
            connected_peers: snapshot.connected_peers as u32,
            error: snapshot.last_error,
        });
    }
    statuses
}

/// Create the tauri-specta builder with all commands and events registered.
///
/// This is extracted so both `main()` and tests can use the same builder
/// to generate TypeScript bindings.
fn create_specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            greet,
            get_app_version,
            get_network_info,
            switch_network,
            get_spv_status,
            // Identity commands
            commands::identity::identity_load,
            commands::identity::identity_search_by_dpns_name,
            commands::identity::identity_search_from_wallet,
            commands::identity::identity_search_up_to_index,
            commands::identity::identity_register_dpns_name,
            commands::identity::identity_refresh,
            commands::identity::identity_refresh_dpns_names,
            commands::identity::identity_withdraw,
            commands::identity::identity_transfer,
            commands::identity::identity_add_key,
            commands::identity::identity_disable_keys,
            commands::identity::identity_replace_key,
            commands::identity::identity_register,
            commands::identity::identity_top_up,
            commands::identity::identity_top_up_from_platform_addresses,
            commands::identity::identity_transfer_to_addresses,
            commands::identity::identity_list_local,
            commands::identity::identity_list_user,
            commands::identity::identity_list_voting,
            commands::identity::identity_get_by_id,
            commands::identity::identity_set_alias,
            commands::identity::identity_get_alias,
            commands::identity::identity_load_order,
            commands::identity::identity_save_order,
            commands::identity::identity_delete,
            commands::identity::identity_list_summaries,
            commands::identity::identity_local_dpns_names,
            // Core commands
            commands::core::core_get_best_chain_lock,
            commands::core::core_get_best_chain_locks,
            commands::core::core_refresh_wallet_info,
            commands::core::core_refresh_single_key_wallet_info,
            commands::core::core_start_dash_qt,
            commands::core::core_create_registration_asset_lock,
            commands::core::core_create_top_up_asset_lock,
            commands::core::core_send_wallet_payment,
            commands::core::core_send_single_key_wallet_payment,
            commands::core::core_recover_asset_locks,
            // Wallet commands
            commands::wallet::wallet_generate_receive_address,
            commands::wallet::wallet_fetch_platform_address_balances,
            commands::wallet::wallet_transfer_platform_credits,
            commands::wallet::wallet_withdraw_from_platform_address,
            commands::wallet::wallet_fund_platform_address_from_utxos,
            commands::wallet::wallet_fund_platform_from_asset_lock,
            commands::wallet::wallet_list_all,
            commands::wallet::wallet_get_hd,
            commands::wallet::wallet_get_single_key,
            commands::wallet::wallet_select,
            commands::wallet::wallet_set_alias,
            commands::wallet::wallet_set_single_key_alias,
            commands::wallet::wallet_remove,
            commands::wallet::wallet_remove_single_key,
            commands::wallet::wallet_start_spv,
            commands::wallet::wallet_stop_spv,
            commands::wallet::wallet_clear_spv_data,
            commands::wallet::wallet_notify_unlocked,
            commands::wallet::wallet_notify_locked,
            // Contract commands
            commands::contract::contract_fetch,
            commands::contract::contract_fetch_with_descriptions,
            commands::contract::contract_fetch_active_group_actions,
            commands::contract::contract_register,
            commands::contract::contract_update,
            commands::contract::contract_save,
            commands::contract::contract_remove,
            commands::contract::contract_list_local,
            commands::contract::contract_get_by_id,
            commands::contract::contract_set_alias,
            // Document commands
            commands::document::document_broadcast,
            commands::document::document_delete,
            commands::document::document_replace,
            commands::document::document_transfer,
            commands::document::document_purchase,
            commands::document::document_set_price,
            commands::document::document_fetch,
            commands::document::document_fetch_page,
            // Token commands
            commands::token::token_query_my_balances,
            commands::token::token_query_identity_balance,
            commands::token::token_query_frozen_identities,
            commands::token::token_query_descriptions_by_keyword,
            commands::token::token_fetch_by_contract_id,
            commands::token::token_fetch_by_token_id,
            commands::token::token_save_locally,
            commands::token::token_query_pricing,
            commands::token::token_mint,
            commands::token::token_transfer,
            commands::token::token_burn,
            commands::token::token_destroy_frozen_funds,
            commands::token::token_freeze,
            commands::token::token_unfreeze,
            commands::token::token_pause,
            commands::token::token_resume,
            commands::token::token_claim,
            commands::token::token_estimate_perpetual_rewards,
            commands::token::token_update_config,
            commands::token::token_purchase,
            commands::token::token_set_direct_purchase_price,
            commands::token::token_register_contract,
            commands::token::token_remove,
            commands::token::token_load_order,
            commands::token::token_save_order,
            // DashPay commands
            commands::dashpay::dashpay_load_profile,
            commands::dashpay::dashpay_update_profile,
            commands::dashpay::dashpay_load_contacts,
            commands::dashpay::dashpay_load_contact_requests,
            commands::dashpay::dashpay_fetch_contact_profile,
            commands::dashpay::dashpay_search_profiles,
            commands::dashpay::dashpay_send_contact_request,
            commands::dashpay::dashpay_send_contact_request_with_proof,
            commands::dashpay::dashpay_accept_contact_request,
            commands::dashpay::dashpay_reject_contact_request,
            commands::dashpay::dashpay_load_payment_history,
            commands::dashpay::dashpay_send_payment_to_contact,
            commands::dashpay::dashpay_update_contact_info,
            commands::dashpay::dashpay_register_addresses,
            commands::dashpay::dashpay_db_load_profile,
            commands::dashpay::dashpay_db_save_profile,
            commands::dashpay::dashpay_db_load_contacts,
            commands::dashpay::dashpay_db_load_pending_requests,
            commands::dashpay::dashpay_db_load_payments,
            commands::dashpay::dashpay_db_load_contact_private_info,
            commands::dashpay::dashpay_db_save_contact_private_info,
            commands::dashpay::dashpay_db_set_contact_hidden,
            commands::dashpay::dashpay_db_save_avatar_bytes,
            // Contested resource / DPNS commands
            commands::contested::contested_query_dpns_contests,
            commands::contested::contested_vote_on_dpns_names,
            commands::contested::contested_schedule_dpns_votes,
            commands::contested::contested_cast_scheduled_vote,
            commands::contested::contested_clear_all_scheduled_votes,
            commands::contested::contested_clear_executed_scheduled_votes,
            commands::contested::contested_delete_scheduled_vote,
            commands::contested::contested_get_scheduled_votes,
            // Platform info commands
            commands::platform_info::platform_current_epoch_info,
            commands::platform_info::platform_total_credits,
            commands::platform_info::platform_version_voting_state,
            commands::platform_info::platform_validator_set_info,
            commands::platform_info::platform_withdrawals_in_queue,
            commands::platform_info::platform_recently_completed_withdrawals,
            commands::platform_info::platform_basic_info,
            commands::platform_info::platform_fetch_address_balance,
            // System commands
            commands::system::system_wipe_platform_data,
            commands::system::system_update_theme,
            // MnList commands
            commands::system::mnlist_fetch_diff,
            commands::system::mnlist_fetch_qr_info,
            commands::system::mnlist_fetch_qr_info_with_dmls,
            commands::system::mnlist_fetch_chain_locks,
            commands::system::mnlist_fetch_diffs_chain,
            // GroveSTARK commands
            commands::system::grovestark_generate_proof,
            commands::system::grovestark_verify_proof,
            // Broadcast state transition
            commands::system::broadcast_state_transition,
            // Settings commands
            commands::settings::settings_get,
            commands::settings::settings_update_password,
            commands::settings::settings_update_dash_core,
            commands::settings::settings_update_disable_zmq,
            commands::settings::settings_update_onboarding_completed,
            commands::settings::settings_update_show_evonode_tools,
            commands::settings::settings_update_user_mode,
            commands::settings::settings_update_close_dash_qt_on_exit,
            commands::settings::settings_update_auto_start_spv,
            commands::settings::settings_get_auto_start_spv,
            // Context commands
            commands::settings::context_is_developer_mode,
            commands::settings::context_enable_developer_mode,
            commands::settings::context_get_fee_multiplier,
            commands::settings::context_set_fee_multiplier,
            commands::settings::context_get_network,
            commands::settings::context_get_core_backend_mode,
            commands::settings::context_set_core_backend_mode,
        ])
        .events(collect_events![
            TaskResultEvent,
            TaskErrorEvent,
            ZmqIsLockedTransactionEvent,
            ZmqChainLockedBlockEvent,
            ZmqConnectionStatusEvent,
            SpvStatusEvent,
            WalletUpdatedEvent,
            ScheduledVoteExecutedEvent,
        ])
}

fn main() {
    let builder = create_specta_builder();

    #[cfg(debug_assertions)]
    {
        let bindings_path = std::path::Path::new("../src/frontend/bindings.ts");
        builder
            .export(
                Typescript::default()
                    .bigint(specta_typescript::BigIntExportBehavior::Number)
                    .header("/* eslint-disable */\n// This file is auto-generated by tauri-specta. Do not edit."),
                bindings_path,
            )
            .expect("Failed to export TypeScript bindings");

        // Strip trailing whitespace generated by tauri-specta
        if let Ok(content) = std::fs::read_to_string(bindings_path) {
            let cleaned: String = content
                .lines()
                .map(|line| line.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            let cleaned = if content.ends_with('\n') {
                cleaned + "\n"
            } else {
                cleaned
            };
            let _ = std::fs::write(bindings_path, cleaned);
        }
    }

    tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            // Initialize the full application state (database, AppContexts, etc.)
            match AppState::init() {
                Ok(app_state) => {
                    let app_state = Arc::new(app_state);

                    // Start background event forwarding loops.
                    // These hold Arc<AppState> clones for the lifetime of the app.
                    let handle = app.handle().clone();
                    task_dispatcher::start_scheduled_vote_polling(
                        handle.clone(),
                        app_state.clone(),
                    );
                    task_dispatcher::start_spv_status_polling(handle, app_state.clone());

                    // Manage the Arc<AppState> directly — Tauri commands receive
                    // tauri::State<'_, Arc<AppState>> and can clone the Arc as needed.
                    app.manage(app_state);

                    tracing::info!("Tauri app setup complete — AppState initialized.");
                }
                Err(e) => {
                    tracing::error!("Failed to initialize AppState: {e}");
                    eprintln!("FATAL: Failed to initialize AppState: {e}");
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Dash Evo Tool");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_returns_expected_response() {
        let response = greet("Alice".into());
        assert_eq!(response.message, "Hello, Alice! Welcome to Dash Evo Tool.");
        assert!(response.timestamp_ms > 0);
    }

    #[test]
    fn greet_handles_empty_name() {
        let response = greet(String::new());
        assert_eq!(response.message, "Hello, ! Welcome to Dash Evo Tool.");
    }

    #[test]
    fn greet_response_serializes() {
        let response = GreetResponse {
            message: "test".into(),
            timestamp_ms: 12345,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"message\":\"test\""));
        assert!(json.contains("\"timestamp_ms\":12345"));
    }

    #[test]
    fn greet_response_deserializes() {
        let json = r#"{"message":"test","timestamp_ms":12345}"#;
        let response: GreetResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.message, "test");
        assert_eq!(response.timestamp_ms, 12345);
    }

    #[test]
    fn network_info_serializes_with_camel_case() {
        let info = NetworkInfo {
            active_network: NetworkDto::Dash,
            available_networks: vec![NetworkDto::Dash, NetworkDto::Testnet],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"activeNetwork\""));
        assert!(json.contains("\"availableNetworks\""));
    }

    #[test]
    fn dispatch_task_response_serializes() {
        let resp = DispatchTaskResponse {
            task_id: "task-42".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"taskId\":\"task-42\""));
    }

    #[test]
    fn event_types_serialize_correctly() {
        // TaskResultEvent
        let event = TaskResultEvent {
            task_id: "task-1".into(),
            result_type: "Identity".into(),
            payload: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"taskId\""));
        assert!(json.contains("\"resultType\""));

        // TaskErrorEvent
        let event = TaskErrorEvent {
            task_id: "task-2".into(),
            message: "Something failed".into(),
            details: "Full error trace".into(),
            recoverable: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"recoverable\":true"));

        // SpvStatusEvent
        let event = SpvStatusEvent {
            network: NetworkDto::Testnet,
            status: events::SpvStatusDto::Syncing,
            sync_progress_pct: Some(42.5),
            header_height: Some(100000),
            connected_peers: 8,
            error: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"syncProgressPct\":42.5"));
        assert!(json.contains("\"connectedPeers\":8"));
    }

    #[test]
    fn zmq_event_types_serialize() {
        let event = ZmqIsLockedTransactionEvent {
            network: NetworkDto::Dash,
            txid: "abc123".into(),
            raw_tx: "deadbeef".into(),
            affected_utxo_count: 3,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"affectedUtxoCount\":3"));

        let event = ZmqChainLockedBlockEvent {
            network: NetworkDto::Dash,
            block_height: 500000,
            block_hash: "def456".into(),
            tx_count: 12,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"blockHeight\":500000"));

        let event = ZmqConnectionStatusEvent {
            network: NetworkDto::Testnet,
            connected: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"connected\":true"));
    }

    #[test]
    fn scheduled_vote_event_serializes() {
        let event = ScheduledVoteExecutedEvent {
            contested_name: "alice".into(),
            voter_id: "abc123".into(),
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"contestedName\":\"alice\""));
        assert!(json.contains("\"voterId\":\"abc123\""));
        assert!(json.contains("\"success\":true"));
    }

    #[test]
    fn wallet_updated_event_serializes() {
        let event = WalletUpdatedEvent {
            wallet_seed_hash: "deadbeef".into(),
            network: NetworkDto::Dash,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"walletSeedHash\":\"deadbeef\""));
    }

    #[test]
    fn export_typescript_bindings() {
        let builder = create_specta_builder();
        let path = std::path::Path::new("../src/frontend/bindings.ts");
        builder
            .export(
                Typescript::default()
                    .bigint(specta_typescript::BigIntExportBehavior::Number)
                    .header(
                        "/* eslint-disable */\n// This file is auto-generated by tauri-specta. Do not edit.",
                    ),
                path,
            )
            .expect("Failed to export TypeScript bindings");

        // tauri-specta generates trailing whitespace on some lines.
        // Strip it to satisfy git pre-commit hooks.
        let content = std::fs::read_to_string(path).expect("Failed to read bindings");
        let cleaned: String = content
            .lines()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        // Preserve trailing newline
        let cleaned = if content.ends_with('\n') {
            cleaned + "\n"
        } else {
            cleaned
        };
        std::fs::write(path, cleaned).expect("Failed to write cleaned bindings");
    }
}
