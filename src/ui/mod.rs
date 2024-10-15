use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::platform::BackendTaskSuccessResult;
use crate::ui::add_key_screen::AddKeyScreen;
use crate::ui::document_query_screen::DocumentQueryScreen;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::identities::add_new_wallet_screen::AddNewWalletScreen;
use crate::ui::key_info_screen::KeyInfoScreen;
use crate::ui::keys_screen::KeysScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::withdrawals::WithdrawalScreen;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use egui::{Context, Widget};
use enum_dispatch::enum_dispatch;
use identities::add_existing_identity_screen::AddExistingIdentityScreen;
use identities::add_new_identity_screen::AddNewIdentityScreen;
use identities::identities_screen::IdentitiesScreen;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod add_key_screen;
pub mod components;
pub mod document_query_screen;
pub mod dpns_contested_names_screen;
pub(crate) mod identities;
pub mod key_info_screen;
pub mod keys_screen;
pub mod network_chooser_screen;
pub mod transition_visualizer_screen;
pub mod withdrawals;

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum RootScreenType {
    RootScreenIdentities,
    RootScreenDPNSContestedNames,
    RootScreenDocumentQuery,
    RootScreenTransitionVisualizerScreen,
    RootScreenNetworkChooser,
}

impl RootScreenType {
    /// Convert `RootScreenType` to an integer
    pub fn to_int(self) -> u32 {
        match self {
            RootScreenType::RootScreenIdentities => 0,
            RootScreenType::RootScreenDPNSContestedNames => 1,
            RootScreenType::RootScreenDocumentQuery => 2,
            RootScreenType::RootScreenTransitionVisualizerScreen => 3,
            RootScreenType::RootScreenNetworkChooser => 4,
        }
    }

    /// Convert an integer to a `RootScreenType`
    pub fn from_int(value: u32) -> Option<Self> {
        match value {
            0 => Some(RootScreenType::RootScreenIdentities),
            1 => Some(RootScreenType::RootScreenDPNSContestedNames),
            2 => Some(RootScreenType::RootScreenTransitionVisualizerScreen),
            3 => Some(RootScreenType::RootScreenNetworkChooser),
            _ => None,
        }
    }
}

impl From<RootScreenType> for ScreenType {
    fn from(value: RootScreenType) -> Self {
        match value {
            RootScreenType::RootScreenIdentities => ScreenType::Identities,
            RootScreenType::RootScreenDPNSContestedNames => ScreenType::DPNSContestedNames,
            RootScreenType::RootScreenTransitionVisualizerScreen => {
                ScreenType::TransitionVisualizer
            }
            RootScreenType::RootScreenDocumentQuery => ScreenType::DocumentQueryScreen,
            RootScreenType::RootScreenNetworkChooser => ScreenType::NetworkChooser,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub enum ScreenType {
    #[default]
    Identities,
    DPNSContestedNames,
    AddNewIdentity,
    AddNewWallet,
    AddExistingIdentity,
    TransitionVisualizer,
    WithdrawalScreen(QualifiedIdentity),
    AddKeyScreen(QualifiedIdentity),
    KeyInfo(QualifiedIdentity, IdentityPublicKey, Option<Vec<u8>>),
    Keys(Identity),
    DocumentQueryScreen,
    NetworkChooser,
}

impl ScreenType {
    pub fn create_screen(&self, app_context: &Arc<AppContext>) -> Screen {
        match self {
            ScreenType::Identities => Screen::IdentitiesScreen(IdentitiesScreen::new(app_context)),
            ScreenType::DPNSContestedNames => {
                Screen::DPNSContestedNamesScreen(DPNSContestedNamesScreen::new(app_context))
            }
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
            ScreenType::TransitionVisualizer => {
                Screen::TransitionVisualizerScreen(TransitionVisualizerScreen::new(app_context))
            }
            ScreenType::WithdrawalScreen(identity) => {
                Screen::WithdrawalScreen(WithdrawalScreen::new(identity.clone(), app_context))
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
            ScreenType::AddNewWallet => {
                Screen::AddNewWalletScreen(AddNewWalletScreen::new(app_context))
            }
        }
    }
}

#[enum_dispatch(ScreenLike)]
pub enum Screen {
    IdentitiesScreen(IdentitiesScreen),
    DPNSContestedNamesScreen(DPNSContestedNamesScreen),
    DocumentQueryScreen(DocumentQueryScreen),
    AddNewWalletScreen(AddNewWalletScreen),
    AddNewIdentityScreen(AddNewIdentityScreen),
    AddExistingIdentityScreen(AddExistingIdentityScreen),
    KeyInfoScreen(KeyInfoScreen),
    KeysScreen(KeysScreen),
    WithdrawalScreen(WithdrawalScreen),
    AddKeyScreen(AddKeyScreen),
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    NetworkChooserScreen(NetworkChooserScreen),
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
            Screen::AddNewWalletScreen(screen) => screen.app_context = app_context,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MessageType {
    Success,
    Info,
    Error,
}

#[enum_dispatch]
pub trait ScreenLike {
    fn refresh(&mut self) {}
    fn ui(&mut self, ctx: &Context) -> AppAction;
    fn display_message(&mut self, message: &str, message_type: MessageType) {}
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {}
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
            Screen::DPNSContestedNamesScreen(_) => ScreenType::DPNSContestedNames,
            Screen::TransitionVisualizerScreen(_) => ScreenType::TransitionVisualizer,
            Screen::WithdrawalScreen(screen) => {
                ScreenType::WithdrawalScreen(screen.identity.clone())
            }
            Screen::NetworkChooserScreen(_) => ScreenType::NetworkChooser,
            Screen::AddKeyScreen(screen) => ScreenType::AddKeyScreen(screen.identity.clone()),
            Screen::DocumentQueryScreen(_) => ScreenType::DocumentQueryScreen,
            Screen::AddNewIdentityScreen(_) => ScreenType::AddExistingIdentity,
            Screen::AddNewWalletScreen(_) => ScreenType::AddNewWallet,
        }
    }
}
