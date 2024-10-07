use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::add_identity_screen::AddIdentityScreen;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::identities_screen::IdentitiesScreen;
use crate::ui::key_info::KeyInfoScreen;
use crate::ui::keys_screen::KeysScreen;
use crate::ui::transition_visualizer_screen::TransitionVisualizerScreen;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use egui::Context;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub mod add_identity_screen;
pub mod components;
pub mod dpns_contested_names_screen;
pub mod identities_screen;
pub mod key_info;
pub mod keys_screen;
pub mod transition_visualizer_screen;

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum RootScreenType {
    RootScreenIdentities,
    RootScreenDPNSContestedNames,
    RootScreenTransitionVisualizerScreen,
}

impl From<RootScreenType> for ScreenType {
    fn from(value: RootScreenType) -> Self {
        match value {
            RootScreenType::RootScreenIdentities => ScreenType::Identities,
            RootScreenType::RootScreenDPNSContestedNames => ScreenType::DPNSContestedNames,
            RootScreenType::RootScreenTransitionVisualizerScreen => {
                ScreenType::TransitionVisualizerScreen
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ScreenType {
    Identities,
    DPNSContestedNames,
    AddIdentity,
    TransitionVisualizerScreen,
    KeyInfo(Identity, IdentityPublicKey, Option<Vec<u8>>),
    Keys(Identity),
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
            ScreenType::TransitionVisualizerScreen => {
                Screen::TransitionVisualizerScreen(TransitionVisualizerScreen::new(app_context))
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
    TransitionVisualizerScreen(TransitionVisualizerScreen),
}

pub trait ScreenLike {
    fn refresh(&mut self);
    fn ui(&mut self, ctx: &Context) -> AppAction;
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
            Screen::TransitionVisualizerScreen(_) => ScreenType::TransitionVisualizerScreen,
        }
    }
}
