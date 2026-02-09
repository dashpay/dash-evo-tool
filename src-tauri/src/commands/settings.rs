//! Settings and context Tauri IPC commands.
//!
//! Exposes application settings, developer mode, fee management, and
//! other configuration operations.

use crate::commands::system::ThemeModeDto;
use crate::dto::NetworkDto;
use crate::state::AppState;

use dash_evo_tool::model::settings::UserMode;
use dash_evo_tool::spv::CoreBackendMode;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// User experience mode DTO.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UserModeDto {
    Beginner,
    Advanced,
}

impl UserModeDto {
    fn to_backend(self) -> UserMode {
        match self {
            Self::Beginner => UserMode::Beginner,
            Self::Advanced => UserMode::Advanced,
        }
    }
    fn from_backend(mode: UserMode) -> Self {
        match mode {
            UserMode::Beginner => Self::Beginner,
            UserMode::Advanced => Self::Advanced,
        }
    }
}

/// Core backend mode DTO.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CoreBackendModeDto {
    Spv,
    Rpc,
}

impl CoreBackendModeDto {
    fn from_backend(mode: CoreBackendMode) -> Self {
        match mode {
            CoreBackendMode::Spv => Self::Spv,
            CoreBackendMode::Rpc => Self::Rpc,
        }
    }
}

/// Full application settings DTO.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub network: NetworkDto,
    pub theme_mode: ThemeModeDto,
    pub overwrite_dash_conf: bool,
    pub disable_zmq: bool,
    pub onboarding_completed: bool,
    pub show_evonode_tools: bool,
    pub user_mode: UserModeDto,
    pub close_dash_qt_on_exit: bool,
    pub core_backend_mode: CoreBackendModeDto,
    pub has_password: bool,
    pub dash_qt_path: Option<String>,
}

/// Input for updating Dash Core execution settings.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDashCoreSettingsInput {
    pub custom_dash_qt_path: Option<String>,
    pub overwrite_dash_conf: bool,
}

/// Input for updating the main password.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMainPasswordInput {
    pub salt_hex: String,
    pub nonce_hex: String,
    pub password_check_hex: String,
}

// ---------------------------------------------------------------------------
// Settings commands
// ---------------------------------------------------------------------------

/// Get the current application settings.
#[tauri::command]
#[specta::specta]
pub fn settings_get(state: tauri::State<'_, Arc<AppState>>) -> Result<SettingsDto, String> {
    let ctx = state.current_context();
    let settings = ctx
        .get_settings()
        .map_err(|e| format!("Failed to get settings: {e}"))?;
    match settings {
        Some(s) => Ok(SettingsDto {
            network: NetworkDto::from_network(s.network),
            theme_mode: ThemeModeDto::from_backend(s.theme_mode),
            overwrite_dash_conf: s.overwrite_dash_conf,
            disable_zmq: s.disable_zmq,
            onboarding_completed: s.onboarding_completed,
            show_evonode_tools: s.show_evonode_tools,
            user_mode: UserModeDto::from_backend(s.user_mode),
            close_dash_qt_on_exit: s.close_dash_qt_on_exit,
            core_backend_mode: CoreBackendModeDto::from_backend(s.core_backend_mode),
            has_password: s.password_info.is_some(),
            dash_qt_path: s.dash_qt_path.map(|p| p.to_string_lossy().to_string()),
        }),
        None => {
            // Return defaults
            let defaults = dash_evo_tool::model::settings::Settings::default();
            Ok(SettingsDto {
                network: NetworkDto::from_network(defaults.network),
                theme_mode: ThemeModeDto::from_backend(defaults.theme_mode),
                overwrite_dash_conf: defaults.overwrite_dash_conf,
                disable_zmq: defaults.disable_zmq,
                onboarding_completed: defaults.onboarding_completed,
                show_evonode_tools: defaults.show_evonode_tools,
                user_mode: UserModeDto::from_backend(defaults.user_mode),
                close_dash_qt_on_exit: defaults.close_dash_qt_on_exit,
                core_backend_mode: CoreBackendModeDto::from_backend(defaults.core_backend_mode),
                has_password: false,
                dash_qt_path: defaults
                    .dash_qt_path
                    .map(|p| p.to_string_lossy().to_string()),
            })
        }
    }
}

/// Update the main password.
#[tauri::command]
#[specta::specta]
pub fn settings_update_password(
    state: tauri::State<'_, Arc<AppState>>,
    input: UpdateMainPasswordInput,
) -> Result<(), String> {
    let salt = hex::decode(&input.salt_hex).map_err(|e| format!("Invalid salt hex: {e}"))?;
    let nonce = hex::decode(&input.nonce_hex).map_err(|e| format!("Invalid nonce hex: {e}"))?;
    let password_check = hex::decode(&input.password_check_hex)
        .map_err(|e| format!("Invalid password check hex: {e}"))?;
    let ctx = state.current_context();
    ctx.update_main_password(&salt, &nonce, &password_check)
        .map_err(|e| format!("Failed to update password: {e}"))
}

/// Update Dash Core execution settings.
#[tauri::command]
#[specta::specta]
pub fn settings_update_dash_core(
    state: tauri::State<'_, Arc<AppState>>,
    input: UpdateDashCoreSettingsInput,
) -> Result<(), String> {
    let path = input.custom_dash_qt_path.map(std::path::PathBuf::from);
    let ctx = state.current_context();
    ctx.update_dash_core_execution_settings(path, input.overwrite_dash_conf)
        .map_err(|e| format!("Failed to update Dash Core settings: {e}"))
}

/// Update the ZMQ disable setting.
#[tauri::command]
#[specta::specta]
pub fn settings_update_disable_zmq(
    state: tauri::State<'_, Arc<AppState>>,
    disable: bool,
) -> Result<(), String> {
    let ctx = state.current_context();
    ctx.update_disable_zmq(disable)
        .map_err(|e| format!("Failed to update ZMQ setting: {e}"))
}

/// Update onboarding completed flag.
#[tauri::command]
#[specta::specta]
pub fn settings_update_onboarding_completed(
    state: tauri::State<'_, Arc<AppState>>,
    completed: bool,
) -> Result<(), String> {
    let db = state.db();
    db.update_onboarding_completed(completed)
        .map_err(|e| format!("Failed to update onboarding: {e}"))
}

/// Update show evonode tools setting.
#[tauri::command]
#[specta::specta]
pub fn settings_update_show_evonode_tools(
    state: tauri::State<'_, Arc<AppState>>,
    show: bool,
) -> Result<(), String> {
    let db = state.db();
    db.update_show_evonode_tools(show)
        .map_err(|e| format!("Failed to update evonode tools setting: {e}"))
}

/// Update user mode (Beginner/Advanced).
#[tauri::command]
#[specta::specta]
pub fn settings_update_user_mode(
    state: tauri::State<'_, Arc<AppState>>,
    mode: UserModeDto,
) -> Result<(), String> {
    let db = state.db();
    db.update_user_mode(mode.to_backend().as_str())
        .map_err(|e| format!("Failed to update user mode: {e}"))
}

/// Update close Dash-Qt on exit setting.
#[tauri::command]
#[specta::specta]
pub fn settings_update_close_dash_qt_on_exit(
    state: tauri::State<'_, Arc<AppState>>,
    close_on_exit: bool,
) -> Result<(), String> {
    let db = state.db();
    db.update_close_dash_qt_on_exit(close_on_exit)
        .map_err(|e| format!("Failed to update close Dash-Qt setting: {e}"))
}

/// Update auto-start SPV setting.
#[tauri::command]
#[specta::specta]
pub fn settings_update_auto_start_spv(
    state: tauri::State<'_, Arc<AppState>>,
    auto_start: bool,
) -> Result<(), String> {
    let db = state.db();
    db.update_auto_start_spv(auto_start)
        .map_err(|e| format!("Failed to update auto-start SPV setting: {e}"))
}

/// Get auto-start SPV setting.
#[tauri::command]
#[specta::specta]
pub fn settings_get_auto_start_spv(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, String> {
    let db = state.db();
    db.get_auto_start_spv()
        .map_err(|e| format!("Failed to get auto-start SPV setting: {e}"))
}

// ---------------------------------------------------------------------------
// Context commands
// ---------------------------------------------------------------------------

/// Check if developer mode is enabled.
#[tauri::command]
#[specta::specta]
pub fn context_is_developer_mode(state: tauri::State<'_, Arc<AppState>>) -> bool {
    let ctx = state.current_context();
    ctx.is_developer_mode()
}

/// Enable or disable developer mode.
#[tauri::command]
#[specta::specta]
pub fn context_enable_developer_mode(state: tauri::State<'_, Arc<AppState>>, enable: bool) {
    let ctx = state.current_context();
    ctx.enable_developer_mode(enable);
}

/// Get the current fee multiplier (in permille: 1000 = 1x, 2000 = 2x).
#[tauri::command]
#[specta::specta]
pub fn context_get_fee_multiplier(state: tauri::State<'_, Arc<AppState>>) -> u64 {
    let ctx = state.current_context();
    ctx.fee_multiplier_permille()
}

/// Set the fee multiplier (in permille: 1000 = 1x, 2000 = 2x).
#[tauri::command]
#[specta::specta]
pub fn context_set_fee_multiplier(state: tauri::State<'_, Arc<AppState>>, multiplier: u64) {
    let ctx = state.current_context();
    ctx.set_fee_multiplier_permille(multiplier);
}

/// Get the current network as a string (for display/routing).
#[tauri::command]
#[specta::specta]
pub fn context_get_network(state: tauri::State<'_, Arc<AppState>>) -> NetworkDto {
    let ctx = state.current_context();
    NetworkDto::from_network(ctx.network())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_dto_serializes() {
        let dto = SettingsDto {
            network: NetworkDto::Testnet,
            theme_mode: ThemeModeDto::Dark,
            overwrite_dash_conf: true,
            disable_zmq: false,
            onboarding_completed: true,
            show_evonode_tools: false,
            user_mode: UserModeDto::Advanced,
            close_dash_qt_on_exit: true,
            core_backend_mode: CoreBackendModeDto::Spv,
            has_password: false,
            dash_qt_path: Some("/usr/bin/dash-qt".into()),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"themeMode\":\"dark\""));
        assert!(json.contains("\"overwriteDashConf\":true"));
        assert!(json.contains("\"disableZmq\":false"));
        assert!(json.contains("\"onboardingCompleted\":true"));
        assert!(json.contains("\"userMode\":\"advanced\""));
        assert!(json.contains("\"closeDashQtOnExit\":true"));
        assert!(json.contains("\"coreBackendMode\":\"spv\""));
        assert!(json.contains("\"hasPassword\":false"));
        assert!(json.contains("\"dashQtPath\":\"/usr/bin/dash-qt\""));
    }

    #[test]
    fn user_mode_dto_roundtrip() {
        let original = UserModeDto::Beginner;
        let backend = original.to_backend();
        let recovered = UserModeDto::from_backend(backend);
        let json_orig = serde_json::to_string(&original).unwrap();
        let json_recovered = serde_json::to_string(&recovered).unwrap();
        assert_eq!(json_orig, json_recovered);
    }

    #[test]
    fn update_dash_core_settings_input_serializes() {
        let input = UpdateDashCoreSettingsInput {
            custom_dash_qt_path: Some("/usr/bin/dash-qt".into()),
            overwrite_dash_conf: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"customDashQtPath\":\"/usr/bin/dash-qt\""));
        assert!(json.contains("\"overwriteDashConf\":true"));
    }

    #[test]
    fn update_password_input_serializes() {
        let input = UpdateMainPasswordInput {
            salt_hex: "aabb".into(),
            nonce_hex: "ccdd".into(),
            password_check_hex: "eeff".into(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"saltHex\":\"aabb\""));
        assert!(json.contains("\"nonceHex\":\"ccdd\""));
        assert!(json.contains("\"passwordCheckHex\":\"eeff\""));
    }

    #[test]
    fn core_backend_mode_dto_serializes() {
        let spv = CoreBackendModeDto::Spv;
        let json = serde_json::to_string(&spv).unwrap();
        assert!(json.contains("\"spv\""));

        let rpc = CoreBackendModeDto::Rpc;
        let json = serde_json::to_string(&rpc).unwrap();
        assert!(json.contains("\"rpc\""));
    }
}
