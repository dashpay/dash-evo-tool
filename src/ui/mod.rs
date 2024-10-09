use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::add_identity_screen::AddIdentityScreen;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::identities_screen::IdentitiesScreen;
use crate::ui::key_info::KeyInfoScreen;
use crate::ui::keys_screen::KeysScreen;
use crate::ui::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::withdrawals::WithdrawalScreen;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use egui::Context;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use crate::ui::network_chooser_screen::NetworkChooserScreen;

pub mod add_identity_screen;
pub mod components;
pub mod dpns_contested_names_screen;
pub mod identities_screen;
pub mod key_info;
pub mod keys_screen;
pub mod transition_visualizer_screen;
pub mod withdrawals;
pub mod network_chooser_screen;

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum RootScreenType {
    RootScreenIdentities,
    RootScreenDPNSContestedNames,
    RootScreenTransitionVisualizerScreen,
    RootScreenNetworkChooser,
}

impl From<RootScreenType> for ScreenType {
    fn from(value: RootScreenType) -> Self {
        match value {
            RootScreenType::RootScreenIdentities => ScreenType::Identities,
            RootScreenType::RootScreenDPNSContestedNames => ScreenType::DPNSContestedNames,
            RootScreenType::RootScreenTransitionVisualizerScreen => {
                ScreenType::TransitionVisualizer
            }
            RootScreenType::RootScreenNetworkChooser => {
                ScreenType::NetworkChooser
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ScreenType {
    Identities,
    DPNSContestedNames,
    AddIdentity,
    TransitionVisualizer,
    WithdrawalScreen(QualifiedIdentity),
    KeyInfo(Identity, IdentityPublicKey, Option<Vec<u8>>),
    Keys(Identity),
    NetworkChooser,
}

impl ScreenType {
    pub fn create_screen(&self, app_context: &Arc<AppContext>) -> Screen {
        match self {
            ScreenType::Identities => Screen::IdentitiesScreen(IdentitiesScreen::new(app_context)),
            ScreenType::DPNSContestedNames => {
                Screen::DPNSContestedNamesScreen(DPNSContestedNamesScreen::new(app_context))
            }
            ScreenType::AddIdentity => {
                Screen::AddIdentityScreen(AddIdentityScreen::new(app_context))
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
                Screen::NetworkChooserScreen(NetworkChooserScreen::new(app_context))
            }
        }
    }
}

impl Default for ScreenType {
    fn default() -> Self {
        ScreenType::Identities
    }
}

pub enum Screen {
    IdentitiesScreen(IdentitiesScreen),
    DPNSContestedNamesScreen(DPNSContestedNamesScreen),
    AddIdentityScreen(AddIdentityScreen),
    KeyInfoScreen(KeyInfoScreen),
    KeysScreen(KeysScreen),
    WithdrawalScreen(WithdrawalScreen),
    TransitionVisualizerScreen(TransitionVisualizerScreen),
    NetworkChooserScreen(NetworkChooserScreen)
}

impl Screen {
    pub fn change_context(&mut self, app_context: Arc<AppContext>) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.app_context = app_context,
            Screen::DPNSContestedNamesScreen(screen) => screen.app_context = app_context,
            Screen::AddIdentityScreen(screen) => screen.app_context = app_context,
            Screen::KeyInfoScreen(screen) => screen.app_context = app_context,
            Screen::KeysScreen(screen) => screen.app_context = app_context,
            Screen::WithdrawalScreen(screen) => screen.app_context = app_context,
            Screen::TransitionVisualizerScreen(screen) => screen.app_context = app_context,
            Screen::NetworkChooserScreen(screen) => screen.app_context = app_context,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MessageType {
    Info,
    Error,
}

pub trait ScreenLike {
    fn refresh(&mut self) {}
    fn ui(&mut self, ctx: &Context) -> AppAction;

    fn display_message(&mut self, message: String, message_type: MessageType) {}
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

impl ScreenLike for Screen {
    fn refresh(&mut self) {
        match self {
            Screen::IdentitiesScreen(main_screen) => main_screen.refresh(),
            Screen::AddIdentityScreen(add_identity_screen) => add_identity_screen.refresh(),
            Screen::KeysScreen(keys_screen) => keys_screen.refresh(),
            Screen::KeyInfoScreen(key_info_screen) => key_info_screen.refresh(),
            Screen::DPNSContestedNamesScreen(contests) => contests.refresh(),
            Screen::TransitionVisualizerScreen(screen) => screen.refresh(),
            Screen::WithdrawalScreen(screen) => screen.refresh(),
            Screen::NetworkChooserScreen(screen) => screen.refresh(),
        }
    }
    fn ui(&mut self, ctx: &Context) -> AppAction {
        match self {
            Screen::IdentitiesScreen(main_screen) => main_screen.ui(ctx),
            Screen::AddIdentityScreen(add_identity_screen) => add_identity_screen.ui(ctx),
            Screen::KeysScreen(keys_screen) => keys_screen.ui(ctx),
            Screen::KeyInfoScreen(key_info_screen) => key_info_screen.ui(ctx),
            Screen::DPNSContestedNamesScreen(contests_screen) => contests_screen.ui(ctx),
            Screen::TransitionVisualizerScreen(screen) => screen.ui(ctx),
            Screen::WithdrawalScreen(screen) => screen.ui(ctx),
            Screen::NetworkChooserScreen(screen) => screen.ui(ctx),
        }
    }

    fn display_message(&mut self, message: String, message_type: MessageType) {
        match self {
            Screen::IdentitiesScreen(screen) => screen.display_message(message, message_type),
            Screen::AddIdentityScreen(screen) => screen.display_message(message, message_type),
            Screen::KeysScreen(screen) => screen.display_message(message, message_type),
            Screen::KeyInfoScreen(screen) => screen.display_message(message, message_type),
            Screen::DPNSContestedNamesScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::TransitionVisualizerScreen(screen) => {
                screen.display_message(message, message_type)
            }
            Screen::WithdrawalScreen(screen) => screen.display_message(message, message_type),
            Screen::NetworkChooserScreen(screen) => screen.display_message(message, message_type),
        }
    }
}

impl Screen {
    pub fn screen_type(&self) -> ScreenType {
        match self {
            Screen::AddIdentityScreen(_) => ScreenType::AddIdentity,
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
        }
    }
}
