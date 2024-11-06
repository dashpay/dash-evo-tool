use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::add_key_screen::AddKeyScreen;
use crate::ui::document_query_screen::DocumentQueryScreen;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::key_info_screen::KeyInfoScreen;
use crate::ui::keys_screen::KeysScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::transfers::TransferScreen;
use crate::ui::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::wallet::wallets_screen::WalletsBalancesScreen;
use crate::ui::withdrawals::WithdrawalScreen;
use crate::ui::withdraws_status_screen::WithdrawsStatusScreen;
use ambassador::{delegatable_trait, Delegate};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use dpns_contested_names_screen::DPNSSubscreen;
use egui::Context;
use identities::add_existing_identity_screen::AddExistingIdentityScreen;
use identities::add_new_identity_screen::AddNewIdentityScreen;
use identities::identities_screen::IdentitiesScreen;
use identities::register_dpns_name_screen::RegisterDpnsNameScreen;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use wallet::add_new_wallet_screen::AddNewWalletScreen;

mod add_key_screen;
pub mod components;
pub mod document_query_screen;
pub mod dpns_contested_names_screen;
pub(crate) mod identities;
pub mod key_info_screen;
pub mod keys_screen;
pub mod network_chooser_screen;
pub mod transfers;
pub mod transition_visualizer_screen;
pub(crate) mod wallet;
pub mod withdrawals;
pub mod withdraws_status_screen;

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum RootScreenType {
    RootScreenIdentities,
    RootScreenDPNSActiveContests,
    RootScreenDPNSPastContests,
    RootScreenDPNSOwnedNames,
    RootScreenDocumentQuery,
    RootScreenWalletsBalances,
    RootScreenTransitionVisualizerScreen,
    RootScreenWithdrawsStatus,
    RootScreenNetworkChooser,
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
            RootScreenType::RootScreenTransitionVisualizerScreen => 6,
            RootScreenType::RootScreenNetworkChooser => 7,
            RootScreenType::RootScreenWithdrawsStatus => 8,
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
            6 => Some(RootScreenType::RootScreenTransitionVisualizerScreen),
            7 => Some(RootScreenType::RootScreenNetworkChooser),
            8 => Some(RootScreenType::RootScreenWithdrawsStatus),
            _ => None,
        }
    }
}

impl From<RootScreenType> for ScreenType {
    fn from(value: RootScreenType) -> Self {
        match value {
            RootScreenType::RootScreenIdentities => ScreenType::Identities,
            RootScreenType::RootScreenDPNSActiveContests => ScreenType::DPNSActiveContests,
            RootScreenType::RootScreenDPNSPastContests => ScreenType::DPNSPastContests,
            RootScreenType::RootScreenDPNSOwnedNames => ScreenType::DPNSMyUsernames,
            RootScreenType::RootScreenTransitionVisualizerScreen => {
                ScreenType::TransitionVisualizer
            }
            RootScreenType::RootScreenDocumentQuery => ScreenType::DocumentQueryScreen,
            RootScreenType::RootScreenWithdrawsStatus => ScreenType::WithdrawsStatus,
            RootScreenType::RootScreenNetworkChooser => ScreenType::NetworkChooser,
            RootScreenType::RootScreenWalletsBalances => ScreenType::WalletsBalances,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub enum ScreenType {
    #[default]
    Identities,
    DPNSActiveContests,
    DPNSPastContests,
    DPNSMyUsernames,
    AddNewIdentity,
    WalletsBalances,
    AddNewWallet,
    AddExistingIdentity,
    TransitionVisualizer,
    WithdrawalScreen(QualifiedIdentity),
    TransferScreen(QualifiedIdentity),
    AddKeyScreen(QualifiedIdentity),
    KeyInfo(QualifiedIdentity, IdentityPublicKey, Option<[u8; 32]>),
    Keys(Identity),
    DocumentQueryScreen,
    WithdrawsStatus,
    NetworkChooser,
    RegisterDpnsName,
}

impl ScreenType {
    pub fn create_screen(&self, app_context: &Arc<AppContext>) -> Screen {
        match self {
            ScreenType::Identities => Screen::IdentitiesScreen(IdentitiesScreen::new(app_context)),
            ScreenType::DPNSActiveContests => Screen::DPNSContestedNamesScreen(
                DPNSContestedNamesScreen::new(app_context, DPNSSubscreen::Active),
            ),
            ScreenType::DPNSPastContests => Screen::DPNSContestedNamesScreen(
                DPNSContestedNamesScreen::new(app_context, DPNSSubscreen::Past),
            ),
            ScreenType::DPNSMyUsernames => Screen::DPNSContestedNamesScreen(
                DPNSContestedNamesScreen::new(app_context, DPNSSubscreen::Owned),
            ),
            ScreenType::AddNewIdentity => {
                Screen::AddNewIdentityScreen(AddNewIdentityScreen::new(app_context))
            }
            ScreenType::AddExistingIdentity => {
                Screen::AddExistingIdentityScreen(AddExistingIdentityScreen::new(app_context))
            }
            ScreenType::Keys(identity) => {
                Screen::KeysScreen(KeysScreen::new(identity.clone(), app_context))
            }
            ScreenType::KeyInfo(identity, key, private_key) => {
                Screen::KeyInfoScreen(KeyInfoScreen::new(
                    identity.clone(),
                    key.clone(),
                    private_key.clone(),
                    app_context,
                ))
            }
            ScreenType::RegisterDpnsName => {
                Screen::RegisterDpnsNameScreen(RegisterDpnsNameScreen::new(app_context))
            }
            ScreenType::TransitionVisualizer => {
                Screen::TransitionVisualizerScreen(TransitionVisualizerScreen::new(app_context))
            }
            ScreenType::WithdrawalScreen(identity) => {
                Screen::WithdrawalScreen(WithdrawalScreen::new(identity.clone(), app_context))
            }
            ScreenType::TransferScreen(identity) => {
                Screen::TransferScreen(TransferScreen::new(identity.clone(), app_context))
            }
            ScreenType::NetworkChooser => {
                unreachable!()
            }
            ScreenType::AddKeyScreen(identity) => {
                Screen::AddKeyScreen(AddKeyScreen::new(identity.clone(), app_context))
            }
            ScreenType::DocumentQueryScreen => {
                Screen::DocumentQueryScreen(DocumentQueryScreen::new(app_context))
            }
            ScreenType::WithdrawsStatus => {
                Screen::WithdrawsStatusScreen(WithdrawsStatusScreen::new(app_context))
            }
            ScreenType::AddNewWallet => {
                Screen::AddNewWalletScreen(AddNewWalletScreen::new(app_context))
            }
            ScreenType::WalletsBalances => {
                Screen::WalletsBalancesScreen(WalletsBalancesScreen::new(app_context))
            }
        }
    }
}

#[derive(Delegate)]
#[delegate(ScreenLike)]
pub enum Screen {
    IdentitiesScreen(IdentitiesScreen),
    DPNSContestedNamesScreen(DPNSContestedNamesScreen),
    DocumentQueryScreen(DocumentQueryScreen),
    AddNewWalletScreen(AddNewWalletScreen),
    AddNewIdentityScreen(AddNewIdentityScreen),
    AddExistingIdentityScreen(AddExistingIdentityScreen),
    KeyInfoScreen(KeyInfoScreen),
    KeysScreen(KeysScreen),
    RegisterDpnsNameScreen(RegisterDpnsNameScreen),
    WithdrawalScreen(WithdrawalScreen),
    TransferScreen(TransferScreen),
    AddKeyScreen(AddKeyScreen),
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    WithdrawsStatusScreen(WithdrawsStatusScreen),
    NetworkChooserScreen(NetworkChooserScreen),
    WalletsBalancesScreen(WalletsBalancesScreen),
}

impl Screen {
    pub fn change_context(&mut self, app_context: Arc<AppContext>) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.app_context = app_context,
            Screen::DPNSContestedNamesScreen(screen) => screen.app_context = app_context,
            Screen::AddExistingIdentityScreen(screen) => screen.app_context = app_context,
            Screen::KeyInfoScreen(screen) => screen.app_context = app_context,
            Screen::KeysScreen(screen) => screen.app_context = app_context,
            Screen::WithdrawalScreen(screen) => screen.app_context = app_context,
            Screen::TransitionVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::NetworkChooserScreen(screen) => screen.current_network = app_context.network,
            Screen::AddKeyScreen(screen) => screen.app_context = app_context,
            Screen::DocumentQueryScreen(screen) => screen.app_context = app_context,
            Screen::AddNewIdentityScreen(screen) => screen.app_context = app_context,
            Screen::RegisterDpnsNameScreen(screen) => screen.app_context = app_context,
            Screen::AddNewWalletScreen(screen) => screen.app_context = app_context,
            Screen::TransferScreen(screen) => screen.app_context = app_context,
            Screen::WalletsBalancesScreen(screen) => screen.app_context = app_context,
            Screen::WithdrawsStatusScreen(screen) => screen.app_context = app_context,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MessageType {
    Success,
    Info,
    Error,
}

#[delegatable_trait]
pub trait ScreenLike {
    fn refresh(&mut self) {}
    fn refresh_on_arrival(&mut self) {
        self.refresh()
    }
    fn ui(&mut self, ctx: &Context) -> AppAction;
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}
    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {
        self.display_message("Success", MessageType::Success)
    }

    fn pop_on_success(&mut self) {}
}

// Implement Debug for Screen using the ScreenType
impl fmt::Debug for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.screen_type())
    }
}

// Implement PartialEq for Screen by comparing the ScreenType
impl PartialEq for Screen {
    fn eq(&self, other: &Self) -> bool {
        self.screen_type() == other.screen_type()
    }
}

impl Screen {
    pub fn screen_type(&self) -> ScreenType {
        match self {
            Screen::AddExistingIdentityScreen(_) => ScreenType::AddExistingIdentity,
            Screen::KeysScreen(screen) => ScreenType::Keys(screen.identity.clone()),
            Screen::KeyInfoScreen(screen) => ScreenType::KeyInfo(
                screen.identity.clone(),
                screen.key.clone(),
                screen.private_key_bytes.clone(),
            ),
            Screen::IdentitiesScreen(_) => ScreenType::Identities,
            Screen::DPNSContestedNamesScreen(DPNSContestedNamesScreen {
                dpns_subscreen: DPNSSubscreen::Active,
                ..
            }) => ScreenType::DPNSActiveContests,
            Screen::DPNSContestedNamesScreen(DPNSContestedNamesScreen {
                dpns_subscreen: DPNSSubscreen::Past,
                ..
            }) => ScreenType::DPNSPastContests,
            Screen::DPNSContestedNamesScreen(DPNSContestedNamesScreen {
                dpns_subscreen: DPNSSubscreen::Owned,
                ..
            }) => ScreenType::DPNSMyUsernames,
            Screen::TransitionVisualizerScreen(_) => ScreenType::TransitionVisualizer,
            Screen::WithdrawalScreen(screen) => {
                ScreenType::WithdrawalScreen(screen.identity.clone())
            }
            Screen::NetworkChooserScreen(_) => ScreenType::NetworkChooser,
            Screen::AddKeyScreen(screen) => ScreenType::AddKeyScreen(screen.identity.clone()),
            Screen::DocumentQueryScreen(_) => ScreenType::DocumentQueryScreen,
            Screen::AddNewIdentityScreen(_) => ScreenType::AddExistingIdentity,
            Screen::RegisterDpnsNameScreen(_) => ScreenType::RegisterDpnsName,
            Screen::AddNewWalletScreen(_) => ScreenType::AddNewWallet,
            Screen::TransferScreen(screen) => ScreenType::TransferScreen(screen.identity.clone()),
            Screen::WalletsBalancesScreen(_) => ScreenType::WalletsBalances,
            Screen::WithdrawsStatusScreen(_) => ScreenType::WithdrawsStatus,
        }
    }
}
