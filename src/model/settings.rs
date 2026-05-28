//! User-facing application preferences, persisted to the upstream
//! platform-wallet-storage key/value store.
//!
//! These twelve fields were previously columns of DET's `settings` table
//! and have moved to a single bincode-encoded blob under
//! [`AppSettings::KV_KEY`] in the shared application k/v store
//! (`<data_dir>/det-app.sqlite`). The blob is global (`None` wallet
//! scope) and not network-prefixed — it spans every network, since the
//! `network` field itself is the active-network pointer.
//!
//! Selected-wallet hashes (`selected_wallet_hash`,
//! `selected_single_key_hash`) moved out in C4 and live as a
//! [`SelectedWallet`](crate::model::selected_wallet::SelectedWallet)
//! blob in the per-network wallet k/v store.

use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use dash_sdk::dpp::dashcore::Network;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

/// User experience mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserMode {
    Beginner,
    #[default]
    Advanced,
}

impl UserMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserMode::Beginner => "Beginner",
            UserMode::Advanced => "Advanced",
        }
    }

    fn from_str_or_default(s: &str) -> Self {
        match s {
            "Beginner" => UserMode::Beginner,
            _ => UserMode::Advanced,
        }
    }
}

/// Application-level user preferences.
///
/// Bincode-serialized as the single blob at [`Self::KV_KEY`] in the
/// shared app k/v store. The 12 fields are exactly the user-preference
/// columns previously held in DET's `settings` table — chain sync,
/// wallet selection, and bootstrap scaffolding are NOT part of this
/// struct.
///
/// Field rules:
/// * Wire encoding goes through the private [`AppSettingsWire`] sidecar
///   — enum/path domain types are reduced to primitives so existing
///   types do not need `serde` derives.
/// * Default values reproduce the previous database column defaults.
#[derive(Debug, Clone)]
pub struct AppSettings {
    /// The active network the app is connected to.
    pub network: Network,
    /// Which root screen the app opens to.
    pub root_screen_type: RootScreenType,
    /// Path to the Dash-Qt binary, if set. `None` means autodetect.
    pub dash_qt_path: Option<PathBuf>,
    /// Whether DET is allowed to overwrite a user's `dash.conf` file.
    pub overwrite_dash_conf: bool,
    /// User has opted out of the Dash Core ZMQ listener (requires
    /// restart to take effect).
    pub disable_zmq: bool,
    /// Light / Dark / System theme preference.
    pub theme_mode: ThemeMode,
    /// Legacy core-backend selector (0 = RPC, 1 = SPV). Chain sync is
    /// SPV-only; this is retained for compatibility with code paths that
    /// still inspect the value.
    pub core_backend_mode: u8,
    /// Whether the user has completed the initial onboarding flow.
    pub onboarding_completed: bool,
    /// Whether Evonode-related tools are shown in the UI.
    pub show_evonode_tools: bool,
    /// User experience mode (Beginner or Advanced).
    pub user_mode: UserMode,
    /// Whether DET closes Dash-Qt automatically when it exits.
    pub close_dash_qt_on_exit: bool,
    /// Whether SPV sync starts automatically when DET launches.
    pub auto_start_spv: bool,
}

impl AppSettings {
    /// Canonical k/v key under which the blob is stored. Global scope
    /// (no wallet, no network prefix) — see module-level docs.
    pub const KV_KEY: &'static str = "det:settings:v1";
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            network: Network::Mainnet,
            root_screen_type: RootScreenType::RootScreenDashpay,
            dash_qt_path: detect_dash_qt_path(),
            overwrite_dash_conf: true,
            disable_zmq: false,
            theme_mode: ThemeMode::System,
            core_backend_mode: 1, // SPV
            onboarding_completed: false,
            show_evonode_tools: false,
            user_mode: UserMode::Advanced,
            close_dash_qt_on_exit: true,
            auto_start_spv: false,
        }
    }
}

/// Wire-level mirror used for bincode encoding. Domain enums and paths
/// are flattened to strings / primitives so the existing types do not
/// need `serde` derives. Translation happens here, in one place.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppSettingsWire {
    network: String,
    root_screen_type: u32,
    dash_qt_path: Option<String>,
    overwrite_dash_conf: bool,
    disable_zmq: bool,
    theme_mode: String,
    core_backend_mode: u8,
    onboarding_completed: bool,
    show_evonode_tools: bool,
    user_mode: String,
    close_dash_qt_on_exit: bool,
    auto_start_spv: bool,
}

impl From<&AppSettings> for AppSettingsWire {
    fn from(s: &AppSettings) -> Self {
        Self {
            network: s.network.to_string(),
            root_screen_type: s.root_screen_type.to_int(),
            dash_qt_path: s
                .dash_qt_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            overwrite_dash_conf: s.overwrite_dash_conf,
            disable_zmq: s.disable_zmq,
            theme_mode: theme_mode_to_str(s.theme_mode).to_string(),
            core_backend_mode: s.core_backend_mode,
            onboarding_completed: s.onboarding_completed,
            show_evonode_tools: s.show_evonode_tools,
            user_mode: s.user_mode.as_str().to_string(),
            close_dash_qt_on_exit: s.close_dash_qt_on_exit,
            auto_start_spv: s.auto_start_spv,
        }
    }
}

impl From<AppSettingsWire> for AppSettings {
    fn from(w: AppSettingsWire) -> Self {
        let defaults = AppSettings::default();
        let network = match w.network.to_lowercase().as_str() {
            "dash" => Network::Mainnet,
            other => Network::from_str(other).unwrap_or(defaults.network),
        };
        let root_screen_type =
            RootScreenType::from_int(w.root_screen_type).unwrap_or(defaults.root_screen_type);
        let theme_mode = theme_mode_from_str(&w.theme_mode);
        let user_mode = UserMode::from_str_or_default(&w.user_mode);
        Self {
            network,
            root_screen_type,
            dash_qt_path: w.dash_qt_path.map(PathBuf::from),
            overwrite_dash_conf: w.overwrite_dash_conf,
            disable_zmq: w.disable_zmq,
            theme_mode,
            core_backend_mode: w.core_backend_mode,
            onboarding_completed: w.onboarding_completed,
            show_evonode_tools: w.show_evonode_tools,
            user_mode,
            close_dash_qt_on_exit: w.close_dash_qt_on_exit,
            auto_start_spv: w.auto_start_spv,
        }
    }
}

impl Serialize for AppSettings {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AppSettingsWire::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AppSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        AppSettingsWire::deserialize(deserializer).map(Self::from)
    }
}

fn theme_mode_to_str(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Light => "Light",
        ThemeMode::Dark => "Dark",
        ThemeMode::System => "System",
    }
}

fn theme_mode_from_str(s: &str) -> ThemeMode {
    match s {
        "Light" => ThemeMode::Light,
        "Dark" => ThemeMode::Dark,
        _ => ThemeMode::System,
    }
}

/// Detects the path to the Dash-Qt binary on the system.
fn detect_dash_qt_path() -> Option<PathBuf> {
    let path = which::which("dash-qt")
        .map(|path| path.to_string_lossy().to_string())
        .inspect_err(|e| tracing::warn!("failed to find dash-qt: {}", e))
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                PathBuf::from("/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt")
            } else if cfg!(target_os = "windows") {
                let program_files = std::env::var("PROGRAMFILES")
                    .unwrap_or_else(|_| "C:\\Program Files".to_string());
                PathBuf::from(program_files).join("DashCore\\dash-qt.exe")
            } else {
                PathBuf::from("/usr/local/bin/dash-qt")
            }
        });

    if path.is_file() {
        Some(path)
    } else {
        tracing::warn!("Dash-Qt binary not found at: {:?}", path);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S1: defaults match the previous database-column defaults so the
    /// "empty start" path (no blob in k/v yet) lands users on the same
    /// configuration they would have had with a fresh settings row.
    #[test]
    fn default_matches_previous_db_defaults() {
        let s = AppSettings::default();
        assert_eq!(s.network, Network::Mainnet);
        assert!(matches!(s.theme_mode, ThemeMode::System));
        assert!(matches!(s.user_mode, UserMode::Advanced));
        assert!(s.overwrite_dash_conf);
        assert!(!s.disable_zmq);
        assert_eq!(s.core_backend_mode, 1);
        assert!(!s.onboarding_completed);
        assert!(!s.show_evonode_tools);
        assert!(s.close_dash_qt_on_exit);
        assert!(!s.auto_start_spv);
    }

    /// S2: a settings blob round-trips through the bincode wire form
    /// with every domain field preserved.
    #[test]
    fn settings_round_trip_through_wire() {
        let s = AppSettings {
            network: Network::Testnet,
            root_screen_type: RootScreenType::RootScreenIdentities,
            dash_qt_path: Some(PathBuf::from("/tmp/dash-qt")),
            overwrite_dash_conf: false,
            disable_zmq: true,
            theme_mode: ThemeMode::Dark,
            core_backend_mode: 0,
            onboarding_completed: true,
            show_evonode_tools: true,
            user_mode: UserMode::Beginner,
            close_dash_qt_on_exit: false,
            auto_start_spv: true,
        };
        let encoded =
            bincode::serde::encode_to_vec(&s, bincode::config::standard()).expect("encode");
        let (decoded, _): (AppSettings, _) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
                .expect("decode");
        assert_eq!(decoded.network, s.network);
        assert_eq!(decoded.root_screen_type, s.root_screen_type);
        assert_eq!(decoded.dash_qt_path, s.dash_qt_path);
        assert_eq!(decoded.overwrite_dash_conf, s.overwrite_dash_conf);
        assert_eq!(decoded.disable_zmq, s.disable_zmq);
        assert_eq!(decoded.theme_mode, s.theme_mode);
        assert_eq!(decoded.core_backend_mode, s.core_backend_mode);
        assert_eq!(decoded.onboarding_completed, s.onboarding_completed);
        assert_eq!(decoded.show_evonode_tools, s.show_evonode_tools);
        assert_eq!(decoded.user_mode, s.user_mode);
        assert_eq!(decoded.close_dash_qt_on_exit, s.close_dash_qt_on_exit);
        assert_eq!(decoded.auto_start_spv, s.auto_start_spv);
    }

    /// S3: legacy "dash" network value (used by databases predating the
    /// `Network::Dash` → `Network::Mainnet` rename) decodes to Mainnet
    /// instead of failing or coercing to a different network.
    #[test]
    fn legacy_dash_network_string_decodes_to_mainnet() {
        let wire = AppSettingsWire {
            network: "dash".to_string(),
            root_screen_type: 0,
            dash_qt_path: None,
            overwrite_dash_conf: true,
            disable_zmq: false,
            theme_mode: "System".to_string(),
            core_backend_mode: 1,
            onboarding_completed: false,
            show_evonode_tools: false,
            user_mode: "Advanced".to_string(),
            close_dash_qt_on_exit: true,
            auto_start_spv: false,
        };
        let s: AppSettings = wire.into();
        assert_eq!(s.network, Network::Mainnet);
    }
}
