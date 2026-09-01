//! User-facing application preferences, persisted to the upstream
//! platform-wallet-storage key/value store.
//!
//! These eleven fields were previously columns of DET's `settings` table
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

use crate::model::user_role::UserRole;
use dash_sdk::dpp::dashcore::Network;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

/// Theme mode preference persisted in [`AppSettings::theme_mode`].
///
/// Pure data enum; theme detection and rendering live in `ui::theme`, which
/// re-exports this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

/// Which root screen the app opens to, persisted in
/// [`AppSettings::root_screen_type`].
///
/// The `to_int`/`from_int` mapping is the stable on-disk encoding; the mapping
/// to the UI `ScreenType` lives in `ui` (which re-exports this type).
#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
#[allow(clippy::enum_variant_names)]
pub enum RootScreenType {
    RootScreenDPNSActiveContests,
    RootScreenDPNSPastContests,
    RootScreenDPNSOwnedNames,
    RootScreenDPNSScheduledVotes,
    RootScreenDocumentQuery,
    RootScreenWalletsBalances,
    RootScreenToolsTransitionVisualizerScreen,
    RootScreenToolsDocumentVisualizerScreen,
    RootScreenNetworkChooser,
    RootScreenToolsProofVisualizerScreen,
    RootScreenMyTokenBalances,
    RootScreenTokenSearch,
    RootScreenTokenCreator,
    RootScreenToolsContractVisualizerScreen,
    RootScreenToolsPlatformInfoScreen,
    RootScreenDashPayContacts,
    RootScreenDashPayProfile,
    RootScreenDashPayPayments,
    RootScreenDashPayProfileSearch,
    RootScreenToolsGroveSTARKScreen,
    RootScreenToolsAddressBalanceScreen,
    RootScreenDashpay,
    /// The unified Identities hub (Home · Contacts · Activity · Settings), and
    /// the single user-facing `Identities` nav entry. Distinct variant so user
    /// selection, persistence, and left-nav highlighting stay independent of the
    /// DashPay entries it coexists with.
    RootScreenIdentityHub,
    /// Masternodes section (Expert-Mode gated). Node-operator surface for
    /// loading masternode/evonode identities, DPNS-contest voting, and
    /// owner/voting/payout key management. Distinct variant so its nav gating,
    /// selection, and persistence stay independent of the everyday-user tabs.
    RootScreenMasternodes,
}

impl RootScreenType {
    /// Convert `RootScreenType` to an integer
    pub fn to_int(self) -> u32 {
        match self {
            // 0 used to be the standalone Identities screen
            RootScreenType::RootScreenDPNSActiveContests => 1,
            RootScreenType::RootScreenDPNSPastContests => 2,
            RootScreenType::RootScreenDPNSOwnedNames => 3,
            RootScreenType::RootScreenDocumentQuery => 4,
            RootScreenType::RootScreenWalletsBalances => 5,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen => 6,
            RootScreenType::RootScreenNetworkChooser => 7,
            // 8 used to be the Withdrawals Statuses screen
            // 9 used to be the Proof Log screen
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
            // 23 used to be the Masternode List Diff screen
            RootScreenType::RootScreenDashpay => 24,
            RootScreenType::RootScreenToolsGroveSTARKScreen => 25,
            RootScreenType::RootScreenToolsAddressBalanceScreen => 26,
            RootScreenType::RootScreenIdentityHub => 27,
            RootScreenType::RootScreenMasternodes => 28,
        }
    }

    /// Convert an integer to a `RootScreenType`
    pub fn from_int(value: u32) -> Option<Self> {
        match value {
            // 0 used to be the standalone Identities screen
            1 => Some(RootScreenType::RootScreenDPNSActiveContests),
            2 => Some(RootScreenType::RootScreenDPNSPastContests),
            3 => Some(RootScreenType::RootScreenDPNSOwnedNames),
            4 => Some(RootScreenType::RootScreenDocumentQuery),
            5 => Some(RootScreenType::RootScreenWalletsBalances),
            6 => Some(RootScreenType::RootScreenToolsTransitionVisualizerScreen),
            7 => Some(RootScreenType::RootScreenNetworkChooser),
            // 8 used to be the Withdrawals Statuses screen
            // 9 used to be the Proof Log screen
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
            // 23 used to be the Masternode List Diff screen
            24 => Some(RootScreenType::RootScreenDashpay),
            25 => Some(RootScreenType::RootScreenToolsGroveSTARKScreen),
            26 => Some(RootScreenType::RootScreenToolsAddressBalanceScreen),
            27 => Some(RootScreenType::RootScreenIdentityHub),
            28 => Some(RootScreenType::RootScreenMasternodes),
            _ => None,
        }
    }
}

#[cfg(test)]
mod root_screen_type_tests {
    use super::RootScreenType;

    #[test]
    fn identity_hub_round_trips() {
        let rt = RootScreenType::RootScreenIdentityHub;
        let encoded = rt.to_int();
        let decoded = RootScreenType::from_int(encoded)
            .expect("new identity hub variant must round-trip through from_int");
        assert_eq!(rt, decoded);
        // Value 27 is the canonical on-disk encoding. Keeping it stable means
        // existing user settings continue to round-trip correctly as new
        // variants are added.
        assert_eq!(encoded, 27);
    }

    #[test]
    fn masternodes_round_trips() {
        let rt = RootScreenType::RootScreenMasternodes;
        let encoded = rt.to_int();
        let decoded = RootScreenType::from_int(encoded)
            .expect("new masternodes variant must round-trip through from_int");
        assert_eq!(rt, decoded);
        // Value 28 is the canonical on-disk encoding — keep it stable so
        // persisted user settings continue to round-trip as variants are added.
        assert_eq!(encoded, 28);
    }

    #[test]
    fn from_int_returns_none_for_unknown_value() {
        assert!(RootScreenType::from_int(9999).is_none());
    }

    /// Encoding 0 belonged to the standalone Identities screen, so it decodes
    /// to `None` and the settings default decides where those users land. That
    /// default must be the Identities hub: any other destination drops someone
    /// who was last on an identities screen onto an unrelated section.
    #[test]
    fn the_retired_identities_encoding_falls_back_to_the_hub() {
        assert!(RootScreenType::from_int(0).is_none());
        assert_eq!(
            super::AppSettings::default().root_screen_type,
            RootScreenType::RootScreenIdentityHub
        );
    }
}

/// Application-level user preferences.
///
/// Bincode-serialized as the single blob at [`Self::KV_KEY`] in the
/// shared app k/v store. The 11 fields are exactly the user-preference
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
    /// Whether the user has completed the initial onboarding flow.
    pub onboarding_completed: bool,
    /// Whether Evonode-related tools are shown in the UI.
    pub show_evonode_tools: bool,
    /// The role the user has explicitly selected, or `None` when no explicit
    /// choice has ever been recorded. `None` resolves to [`UserRole::WHEN_UNSET`]
    /// at the settings-load call site (`AppContext::get_app_settings`) and is
    /// only overwritten once the user picks a role — see `UserRole::from_persisted`.
    pub user_role: Option<UserRole>,
    /// Whether DET closes Dash-Qt automatically when it exits.
    pub close_dash_qt_on_exit: bool,
    /// SPV sync starts automatically on launch. Default `true` for fresh
    /// installs (no saved blob); existing installs keep their stored value —
    /// the Network Settings checkbox is the opt-out.
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
            // Matches `app::FALLBACK_ROOT_SCREEN`: the one nav entry every
            // install has, and where an undecodable persisted value lands.
            root_screen_type: RootScreenType::RootScreenIdentityHub,
            dash_qt_path: detect_dash_qt_path(),
            overwrite_dash_conf: true,
            disable_zmq: false,
            theme_mode: ThemeMode::System,
            onboarding_completed: false,
            show_evonode_tools: false,
            // `None` = no explicit role recorded; resolved at load.
            user_role: None,
            close_dash_qt_on_exit: true,
            // Default to on so wallets sync without a manual step on fresh installs.
            // Existing users who stored false explicitly (from the old default) keep
            // their saved preference — the blob wins over this default.
            auto_start_spv: true,
        }
    }
}

/// Wire-level mirror used for bincode encoding. Domain enums and paths
/// are flattened to strings / primitives so the existing types do not
/// need `serde` derives. Translation happens here, in one place.
///
/// `bincode::config::standard()` is positional, so this struct's field
/// order and count *are* the on-disk format for every stored blob. The
/// `_reserved_core_backend_mode` byte is a retired field (the RPC/SPV
/// selector — chain sync is SPV-only now) kept solely to preserve that
/// layout: dropping it would shift every following field and corrupt
/// existing `det:settings:v1` blobs. It is written as a constant and
/// ignored on read.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppSettingsWire {
    network: String,
    root_screen_type: u32,
    dash_qt_path: Option<String>,
    overwrite_dash_conf: bool,
    disable_zmq: bool,
    theme_mode: String,
    _reserved_core_backend_mode: u8,
    onboarding_completed: bool,
    show_evonode_tools: bool,
    /// Length-prefixed slot that once held the orphaned `UserMode` string and
    /// now carries the canonical `UserRole` string (or `""` when no explicit
    /// role is recorded). Reusing the slot keeps the positional wire layout
    /// unchanged; only the stored value's meaning moved.
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
            // Retired field; constant preserves the wire layout (SPV marker).
            _reserved_core_backend_mode: 1,
            onboarding_completed: s.onboarding_completed,
            show_evonode_tools: s.show_evonode_tools,
            // `None` encodes as the empty string, which decodes back to `None`
            // (a legacy sentinel) via `UserRole::from_persisted`.
            user_mode: s.user_role.map(UserRole::as_str).unwrap_or("").to_string(),
            close_dash_qt_on_exit: s.close_dash_qt_on_exit,
            auto_start_spv: s.auto_start_spv,
        }
    }
}

impl From<AppSettingsWire> for AppSettings {
    fn from(w: AppSettingsWire) -> Self {
        let defaults = AppSettings::default();
        let network = network_from_legacy_str(&w.network).unwrap_or(defaults.network);
        let root_screen_type =
            RootScreenType::from_int(w.root_screen_type).unwrap_or(defaults.root_screen_type);
        let theme_mode = theme_mode_from_str(&w.theme_mode);
        // Decoding stays pure: `None` (legacy sentinel or empty) is resolved to
        // a concrete role once at the load site, seeded from `.env`.
        let user_role = UserRole::from_persisted(&w.user_mode);
        Self {
            network,
            root_screen_type,
            // Verbatim: `None` means "autodetect", but decoding must stay pure
            // (no filesystem IO). The autodetect fallback runs once at the
            // settings-load call site (`AppContext::get_app_settings`).
            dash_qt_path: w.dash_qt_path.map(PathBuf::from),
            overwrite_dash_conf: w.overwrite_dash_conf,
            disable_zmq: w.disable_zmq,
            theme_mode,
            onboarding_completed: w.onboarding_completed,
            show_evonode_tools: w.show_evonode_tools,
            user_role,
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

pub(crate) fn theme_mode_from_str(s: &str) -> ThemeMode {
    match s {
        "Light" => ThemeMode::Light,
        "Dark" => ThemeMode::Dark,
        _ => ThemeMode::System,
    }
}

/// Parse a network name as written by DET, accepting the pre-v29 spelling.
///
/// `data.db` (and therefore every stored settings blob) wrote mainnet as
/// `dash` until migration 29 renamed it to `mainnet`. Both spellings must
/// resolve, or an upgrading user silently lands on the default network.
/// Returns `None` for an unrecognised name so the caller can keep its own
/// fallback.
pub(crate) fn network_from_legacy_str(s: &str) -> Option<Network> {
    match s.to_lowercase().as_str() {
        "dash" => Some(Network::Mainnet),
        other => Network::from_str(other).ok(),
    }
}

/// Detects the path to the Dash-Qt binary on the system.
///
/// Filesystem IO — never call from a `Deserialize` path. Callers that need an
/// autodetect fallback for a decoded blob run this once at the settings-load
/// call site (`AppContext::get_app_settings`).
pub(crate) fn detect_dash_qt_path() -> Option<PathBuf> {
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

    /// Captured from the pre-change wire struct with live `core_backend_mode = 0`.
    const PRE_CHANGE_APP_SETTINGS_WIRE: &[u8] =
        b"\x07testnet\x03\x01\x0c/opt/dash-qt\x00\x01\x04Dark\x00\x01\x01\x08Beginner\x00\x01";

    /// S1: verify the `AppSettings` defaults for a fresh install (no blob in
    /// k/v yet). `auto_start_spv` intentionally differs from the old DB column
    /// default (0/false): new installs sync without a manual step; existing
    /// users who stored false keep their saved preference (the blob wins).
    #[test]
    fn default_matches_expected_fresh_install_values() {
        let s = AppSettings::default();
        assert_eq!(s.network, Network::Mainnet);
        assert!(matches!(s.theme_mode, ThemeMode::System));
        // No explicit role on a fresh install; the concrete role is seeded
        // from `.env` at the load site (`AppContext::get_app_settings`).
        assert_eq!(s.user_role, None);
        assert!(s.overwrite_dash_conf);
        assert!(!s.disable_zmq);
        assert!(!s.onboarding_completed);
        assert!(!s.show_evonode_tools);
        assert!(s.close_dash_qt_on_exit);
        assert!(s.auto_start_spv); // on by default for fresh installs
    }

    /// S6: `auto_start_spv` default-on semantics — fresh installs auto-connect;
    /// an existing blob with `false` keeps `false` (the struct default does NOT
    /// override a persisted value).
    #[test]
    fn auto_start_spv_default_on_and_stored_false_survives_round_trip() {
        // Fresh install: no blob → default is true.
        assert!(
            AppSettings::default().auto_start_spv,
            "fresh install must default to auto-connect"
        );

        // Existing user: blob encodes false → decodes to false regardless of
        // the current struct default.
        let wire = AppSettingsWire {
            network: "testnet".to_string(),
            root_screen_type: 0,
            dash_qt_path: None,
            overwrite_dash_conf: true,
            disable_zmq: false,
            theme_mode: "System".to_string(),
            _reserved_core_backend_mode: 1,
            onboarding_completed: true,
            show_evonode_tools: false,
            user_mode: "Advanced".to_string(),
            close_dash_qt_on_exit: true,
            auto_start_spv: false, // user had auto-connect off
        };
        let encoded =
            bincode::serde::encode_to_vec(AppSettings::from(wire), bincode::config::standard())
                .expect("encode");
        let (decoded, _): (AppSettings, _) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
                .expect("decode");
        assert!(
            !decoded.auto_start_spv,
            "a stored false must survive the round-trip — the new default must not override it"
        );
    }

    /// S2: a settings blob round-trips through the bincode wire form
    /// with every domain field preserved.
    #[test]
    fn settings_round_trip_through_wire() {
        let s = AppSettings {
            network: Network::Testnet,
            root_screen_type: RootScreenType::RootScreenIdentityHub,
            dash_qt_path: Some(PathBuf::from("/tmp/dash-qt")),
            overwrite_dash_conf: false,
            disable_zmq: true,
            theme_mode: ThemeMode::Dark,
            onboarding_completed: true,
            show_evonode_tools: true,
            user_role: Some(UserRole::Power),
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
        assert_eq!(decoded.onboarding_completed, s.onboarding_completed);
        assert_eq!(decoded.show_evonode_tools, s.show_evonode_tools);
        assert_eq!(decoded.user_role, s.user_role);
        assert_eq!(decoded.close_dash_qt_on_exit, s.close_dash_qt_on_exit);
        assert_eq!(decoded.auto_start_spv, s.auto_start_spv);
    }

    /// S7: the retired `_reserved_core_backend_mode` byte holds the wire
    /// layout stable. An existing blob whose reserved byte is `0` (the old
    /// "RPC" value) must still decode with every following field intact —
    /// proof that removing the live field did not shift the positional
    /// bincode format and corrupt already-stored `det:settings:v1` blobs.
    #[test]
    fn reserved_core_backend_mode_byte_preserves_wire_layout() {
        let (decoded, _) = bincode::serde::decode_from_slice::<AppSettings, _>(
            PRE_CHANGE_APP_SETTINGS_WIRE,
            bincode::config::standard(),
        )
        .expect("the complete legacy settings blob must decode");
        // Fields after the reserved byte must be read from the correct
        // offset — a shifted layout would scramble these.
        assert!(decoded.onboarding_completed);
        assert!(decoded.show_evonode_tools);
        // Legacy "Beginner" is a sentinel — it decodes to `None`, not a role.
        assert_eq!(decoded.user_role, None);
        assert!(!decoded.close_dash_qt_on_exit);
        assert!(decoded.auto_start_spv);
        // Fields before it, for completeness.
        assert_eq!(decoded.network, Network::Testnet);
        assert!(matches!(decoded.theme_mode, ThemeMode::Dark));
    }

    #[test]
    fn truncated_settings_blob_is_rejected() {
        let truncated = &PRE_CHANGE_APP_SETTINGS_WIRE[..PRE_CHANGE_APP_SETTINGS_WIRE.len() - 1];

        assert!(
            bincode::serde::decode_from_slice::<AppSettings, _>(
                truncated,
                bincode::config::standard(),
            )
            .is_err(),
            "boot must distinguish a corrupt saved preference from a fresh install",
        );
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
            _reserved_core_backend_mode: 1,
            onboarding_completed: false,
            show_evonode_tools: false,
            user_mode: "Advanced".to_string(),
            close_dash_qt_on_exit: true,
            auto_start_spv: false,
        };
        let s: AppSettings = wire.into();
        assert_eq!(s.network, Network::Mainnet);
    }

    /// The canonical `UserRole` strings survive the reused `user_mode` wire
    /// slot in both directions — the slot now carries the role, not the retired
    /// `UserMode`.
    #[test]
    fn user_role_canonical_strings_round_trip_through_wire() {
        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            let s = AppSettings {
                user_role: Some(role),
                ..AppSettings::default()
            };
            let encoded =
                bincode::serde::encode_to_vec(&s, bincode::config::standard()).expect("encode");
            let (decoded, _): (AppSettings, _) =
                bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
                    .expect("decode");
            assert_eq!(decoded.user_role, Some(role));
        }
    }

    /// The migration-critical case: a pre-migration blob's `user_mode` slot
    /// holds the legacy `"Advanced"` default for EVERY user. It must decode to
    /// `None` (a sentinel deferring to the `.env` seed), never `Power` — mapping
    /// it to a role would silently promote the entire user base to expert mode.
    #[test]
    fn legacy_advanced_user_mode_decodes_to_none_not_a_role() {
        let wire = AppSettingsWire {
            network: "testnet".to_string(),
            root_screen_type: 0,
            dash_qt_path: None,
            overwrite_dash_conf: true,
            disable_zmq: false,
            theme_mode: "System".to_string(),
            _reserved_core_backend_mode: 1,
            onboarding_completed: false,
            show_evonode_tools: false,
            user_mode: "Advanced".to_string(),
            close_dash_qt_on_exit: true,
            auto_start_spv: false,
        };
        let s: AppSettings = wire.into();
        assert_eq!(
            s.user_role, None,
            "legacy 'Advanced' must be a sentinel, not a role"
        );
    }

    fn wire_with_dash_qt_path(dash_qt_path: Option<String>) -> AppSettingsWire {
        AppSettingsWire {
            network: "testnet".to_string(),
            root_screen_type: 0,
            dash_qt_path,
            overwrite_dash_conf: true,
            disable_zmq: false,
            theme_mode: "System".to_string(),
            _reserved_core_backend_mode: 1,
            onboarding_completed: false,
            show_evonode_tools: false,
            user_mode: "Advanced".to_string(),
            close_dash_qt_on_exit: true,
            auto_start_spv: false,
        }
    }

    /// S4: a stored Dash-Qt path is preserved verbatim — autodetect never
    /// overrides an explicit user choice on deserialize.
    #[test]
    fn stored_dash_qt_path_is_preserved() {
        let stored = "/custom/path/to/dash-qt";
        let s: AppSettings = wire_with_dash_qt_path(Some(stored.to_string())).into();
        assert_eq!(s.dash_qt_path, Some(PathBuf::from(stored)));
    }

    /// S5: an unset (`None`) Dash-Qt path decodes to `None` verbatim — decoding
    /// stays pure (no filesystem IO). The autodetect fallback runs at the
    /// settings-load call site (`AppContext::get_app_settings`), not here.
    #[test]
    fn unset_dash_qt_path_decodes_to_none() {
        let s: AppSettings = wire_with_dash_qt_path(None).into();
        assert_eq!(s.dash_qt_path, None);
    }
}
