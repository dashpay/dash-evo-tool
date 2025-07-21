use crate::model::password_info::PasswordInfo;
use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;

/// Connection mode for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMode {
    /// Connect through Dash Core RPC
    Core,
    /// Use SPV (Simplified Payment Verification) client
    Spv,
}

impl Default for ConnectionMode {
    fn default() -> Self {
        ConnectionMode::Core
    }
}

/// Application settings structure
#[derive(Debug, Clone)]
pub struct Settings {
    pub network: Network,
    pub root_screen_type: RootScreenType,
    pub password_info: Option<PasswordInfo>,
    /// Path to the Dash-Qt binary, if set. None means autodetect.
    /// Empty value (`""`) means path deliberately not set, autodetect will not be performed.
    pub dash_qt_path: Option<PathBuf>,
    pub overwrite_dash_conf: bool,
    pub theme_mode: ThemeMode,
    pub connection_mode: ConnectionMode,
}

impl
    From<(
        Network,
        RootScreenType,
        Option<PasswordInfo>,
        Option<PathBuf>,
        bool,
        ThemeMode,
        ConnectionMode,
    )> for Settings
{
    /// Converts a tuple into a Settings instance
    ///
    /// Used mainly for database operations where settings are retrieved as a tuple.
    fn from(
        tuple: (
            Network,
            RootScreenType,
            Option<PasswordInfo>,
            Option<PathBuf>,
            bool,
            ThemeMode,
            ConnectionMode,
        ),
    ) -> Self {
        Self::new(tuple.0, tuple.1, tuple.2, tuple.3, tuple.4, tuple.5, tuple.6)
    }
}

impl Default for Settings {
    /// Default settings for the application
    fn default() -> Self {
        Self::new(
            Network::Dash,
            RootScreenType::RootScreenIdentities,
            None,
            None, // autodetect
            true,
            ThemeMode::System,
            ConnectionMode::default(),
        )
    }
}

impl Settings {
    /// Creates a new Settings instance
    pub fn new(
        network: Network,
        root_screen_type: RootScreenType,
        password_info: Option<PasswordInfo>,
        dash_qt_path: Option<PathBuf>,
        overwrite_dash_conf: bool,
        theme_mode: ThemeMode,
        connection_mode: ConnectionMode,
    ) -> Self {
        Self {
            network,
            root_screen_type,
            password_info,
            dash_qt_path: dash_qt_path.or_else(detect_dash_qt_path),
            overwrite_dash_conf,
            theme_mode,
            connection_mode,
        }
    }
}

/// Detects the path to the Dash-Qt binary on the system
fn detect_dash_qt_path() -> Option<PathBuf> {
    let path = which::which("dash-qt")
        .map(|path| path.to_string_lossy().to_string())
        .inspect_err(|e| tracing::warn!("failed to find dash-qt: {}", e))
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Fallback to default paths based on the operating system
            if cfg!(target_os = "macos") {
                PathBuf::from("/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt")
            } else if cfg!(target_os = "windows") {
                // Retrieve the PROGRAMFILES environment variable or default to "C:\\Program Files"
                let program_files = std::env::var("PROGRAMFILES")
                    .unwrap_or_else(|_| "C:\\Program Files".to_string());
                PathBuf::from(program_files).join("DashCore\\dash-qt.exe")
            } else {
                PathBuf::from("/usr/local/bin/dash-qt") // Default Linux path
            }
        });

    if path.is_file() {
        Some(path)
    } else {
        tracing::warn!("Dash-Qt binary not found at: {:?}", path);
        None
    }
}
