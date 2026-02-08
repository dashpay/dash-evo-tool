use crate::model::password_info::PasswordInfo;
use crate::spv::CoreBackendMode;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;

/// Theme mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
#[allow(clippy::enum_variant_names)]
pub enum RootScreenType {
    RootScreenIdentities,
    RootScreenDPNSActiveContests,
    RootScreenDPNSPastContests,
    RootScreenDPNSOwnedNames,
    RootScreenDPNSScheduledVotes,
    RootScreenDocumentQuery,
    RootScreenWalletsBalances,
    RootScreenToolsProofLogScreen,
    RootScreenToolsTransitionVisualizerScreen,
    RootScreenToolsDocumentVisualizerScreen,
    RootScreenNetworkChooser,
    RootScreenToolsProofVisualizerScreen,
    RootScreenMyTokenBalances,
    RootScreenTokenSearch,
    RootScreenTokenCreator,
    RootScreenToolsMasternodeListDiffScreen,
    RootScreenToolsContractVisualizerScreen,
    RootScreenToolsPlatformInfoScreen,
    RootScreenDashPayContacts,
    RootScreenDashPayProfile,
    RootScreenDashPayPayments,
    RootScreenDashPayProfileSearch,
    RootScreenToolsGroveSTARKScreen,
    RootScreenToolsAddressBalanceScreen,
    RootScreenDashpay,
}

impl RootScreenType {
    /// Convert `RootScreenType` to an integer
    pub fn to_int(self) -> u32 {
        match self {
            RootScreenType::RootScreenIdentities => 0,
            RootScreenType::RootScreenDPNSActiveContests => 1,
            RootScreenType::RootScreenDPNSPastContests => 2,
            RootScreenType::RootScreenDPNSOwnedNames => 3,
            RootScreenType::RootScreenDocumentQuery => 4,
            RootScreenType::RootScreenWalletsBalances => 5,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen => 6,
            RootScreenType::RootScreenNetworkChooser => 7,
            // 8 used to be the Withdrawals Statuses screen
            RootScreenType::RootScreenToolsProofLogScreen => 9,
            RootScreenType::RootScreenDPNSScheduledVotes => 10,
            RootScreenType::RootScreenToolsProofVisualizerScreen => 11,
            RootScreenType::RootScreenMyTokenBalances => 12,
            RootScreenType::RootScreenTokenSearch => 13,
            RootScreenType::RootScreenTokenCreator => 14,
            RootScreenType::RootScreenToolsDocumentVisualizerScreen => 15,
            RootScreenType::RootScreenToolsContractVisualizerScreen => 16,
            RootScreenType::RootScreenToolsPlatformInfoScreen => 17,
            RootScreenType::RootScreenDashPayContacts => 18,
            // 19 used to be RootScreenDashPayRequests (now consolidated into Contacts)
            RootScreenType::RootScreenDashPayProfile => 20,
            RootScreenType::RootScreenDashPayPayments => 21,
            RootScreenType::RootScreenDashPayProfileSearch => 22,
            RootScreenType::RootScreenToolsMasternodeListDiffScreen => 23,
            RootScreenType::RootScreenDashpay => 24,
            RootScreenType::RootScreenToolsGroveSTARKScreen => 25,
            RootScreenType::RootScreenToolsAddressBalanceScreen => 26,
        }
    }

    /// Convert an integer to a `RootScreenType`
    pub fn from_int(value: u32) -> Option<Self> {
        match value {
            0 => Some(RootScreenType::RootScreenIdentities),
            1 => Some(RootScreenType::RootScreenDPNSActiveContests),
            2 => Some(RootScreenType::RootScreenDPNSPastContests),
            3 => Some(RootScreenType::RootScreenDPNSOwnedNames),
            4 => Some(RootScreenType::RootScreenDocumentQuery),
            5 => Some(RootScreenType::RootScreenWalletsBalances),
            6 => Some(RootScreenType::RootScreenToolsTransitionVisualizerScreen),
            7 => Some(RootScreenType::RootScreenNetworkChooser),
            // 8 used to be the Withdrawals Statuses screen
            9 => Some(RootScreenType::RootScreenToolsProofLogScreen),
            10 => Some(RootScreenType::RootScreenDPNSScheduledVotes),
            11 => Some(RootScreenType::RootScreenToolsProofVisualizerScreen),
            12 => Some(RootScreenType::RootScreenMyTokenBalances),
            13 => Some(RootScreenType::RootScreenTokenSearch),
            14 => Some(RootScreenType::RootScreenTokenCreator),
            15 => Some(RootScreenType::RootScreenToolsDocumentVisualizerScreen),
            16 => Some(RootScreenType::RootScreenToolsContractVisualizerScreen),
            17 => Some(RootScreenType::RootScreenToolsPlatformInfoScreen),
            18 => Some(RootScreenType::RootScreenDashPayContacts),
            // 19 used to be RootScreenDashPayRequests (now consolidated into Contacts)
            20 => Some(RootScreenType::RootScreenDashPayProfile),
            21 => Some(RootScreenType::RootScreenDashPayPayments),
            22 => Some(RootScreenType::RootScreenDashPayProfileSearch),
            23 => Some(RootScreenType::RootScreenToolsMasternodeListDiffScreen),
            24 => Some(RootScreenType::RootScreenDashpay),
            25 => Some(RootScreenType::RootScreenToolsGroveSTARKScreen),
            26 => Some(RootScreenType::RootScreenToolsAddressBalanceScreen),
            _ => None,
        }
    }
}

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
    pub disable_zmq: bool,
    pub theme_mode: ThemeMode,
    pub core_backend_mode: CoreBackendMode,
    /// Whether the user has completed the initial onboarding
    pub onboarding_completed: bool,
    /// Whether to show Evonode-related tools
    pub show_evonode_tools: bool,
    /// User experience mode (Beginner or Advanced)
    pub user_mode: UserMode,
    /// Whether to automatically close Dash-Qt when DET exits
    pub close_dash_qt_on_exit: bool,
}

impl
    From<(
        Network,
        RootScreenType,
        Option<PasswordInfo>,
        Option<PathBuf>,
        bool,
        bool,
        ThemeMode,
        u8,
        bool,     // onboarding_completed
        bool,     // show_evonode_tools
        UserMode, // user_mode
        bool,     // close_dash_qt_on_exit
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
            bool,
            ThemeMode,
            u8,
            bool,
            bool,
            UserMode,
            bool,
        ),
    ) -> Self {
        Self::new(
            tuple.0,
            tuple.1,
            tuple.2,
            tuple.3,
            tuple.4,
            tuple.5,
            tuple.6,
            CoreBackendMode::from(tuple.7),
            tuple.8,
            tuple.9,
            tuple.10,
            tuple.11,
        )
    }
}

impl Default for Settings {
    /// Default settings for the application
    fn default() -> Self {
        Self::new(
            Network::Dash,
            RootScreenType::RootScreenDashpay,
            None,
            None, // autodetect
            true,
            false,
            ThemeMode::System,
            CoreBackendMode::Spv, // Default to SPV mode
            false,                // onboarding not completed
            false,                // don't show evonode tools by default
            UserMode::Advanced,   // default to advanced mode
            true,                 // close Dash-Qt on exit by default
        )
    }
}

impl Settings {
    /// Creates a new Settings instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        network: Network,
        root_screen_type: RootScreenType,
        password_info: Option<PasswordInfo>,
        dash_qt_path: Option<PathBuf>,
        overwrite_dash_conf: bool,
        disable_zmq: bool,
        theme_mode: ThemeMode,
        core_backend_mode: CoreBackendMode,
        onboarding_completed: bool,
        show_evonode_tools: bool,
        user_mode: UserMode,
        close_dash_qt_on_exit: bool,
    ) -> Self {
        Self {
            network,
            root_screen_type,
            password_info,
            dash_qt_path: dash_qt_path.or_else(detect_dash_qt_path),
            overwrite_dash_conf,
            disable_zmq,
            theme_mode,
            core_backend_mode,
            onboarding_completed,
            show_evonode_tools,
            user_mode,
            close_dash_qt_on_exit,
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
