use std::fmt;
use std::sync::Arc;
use dpp::identity::Identity;
use egui::Context;
use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::add_identity_screen::AddIdentityScreen;
use crate::ui::keys_screen::KeysScreen;
use crate::ui::main::MainScreen;

pub mod keys_screen;
pub mod add_identity_screen;
pub mod main;
pub mod components;

#[derive(Debug, PartialEq, Clone)]
pub enum ScreenType {
    Main,
    AddIdentity,
    Keys(Identity),
}

impl ScreenType {
    pub fn create_screen(&self, app_context: &Arc<AppContext>) -> Screen {
        match  self {
            ScreenType::Main => {
                Screen::MainScreen(MainScreen::new(app_context))
            }
            ScreenType::AddIdentity => {
                Screen::AddIdentityScreen(AddIdentityScreen::new(app_context))
            }
            ScreenType::Keys(identity) => {
                Screen::KeysScreen(KeysScreen::new(identity.clone(), app_context))
            }
        }
    }
}

impl Default for ScreenType {
    fn default() -> Self {
        ScreenType::Main
    }
}

pub enum Screen {
    MainScreen(MainScreen),
    AddIdentityScreen(AddIdentityScreen),
    KeysScreen(KeysScreen),
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
    pub fn ui(&mut self, ctx: &Context) -> AppAction {
        match self {
            Screen::MainScreen(main_screen) => { main_screen.ui(ctx) }
            Screen::AddIdentityScreen(add_identity_screen) => { add_identity_screen.ui(ctx)}
            Screen::KeysScreen(keys_screen) => { keys_screen.ui(ctx) }
        }
    }
    pub fn screen_type(&self) -> ScreenType {
        match self {
            Screen::AddIdentityScreen(_) => ScreenType::AddIdentity,
            Screen::KeysScreen(screen) => ScreenType::Keys(screen.identity.clone()),
            Screen::MainScreen(_) => ScreenType::Main,
        }
    }
}