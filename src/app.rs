use crate::context::AppContext;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::identities_screen::IdentitiesScreen;
use crate::ui::{RootScreenType, Screen, ScreenLike, ScreenType};
use dpp::prelude::Identifier;
use dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use eframe::{egui, App};
use std::collections::BTreeMap;
use std::ops::BitOrAssign;
use std::sync::Arc;
use std::vec;

pub struct AppState {
    pub main_screens: BTreeMap<RootScreenType, Screen>,
    pub selected_main_screen: RootScreenType,
    pub screen_stack: Vec<Screen>,
    pub app_context: Arc<AppContext>,
}

#[derive(Debug, PartialEq)]
pub enum DesiredAppAction {
    None,
    PopScreen,
    GoToMainScreen,
    AddScreenType(ScreenType),
}

impl DesiredAppAction {
    pub fn create_action(&self, app_context: &Arc<AppContext>) -> AppAction {
        match self {
            DesiredAppAction::None => AppAction::None,
            DesiredAppAction::PopScreen => AppAction::PopScreen,
            DesiredAppAction::GoToMainScreen => AppAction::GoToMainScreen,
            DesiredAppAction::AddScreenType(screen_type) => {
                AppAction::AddScreen(screen_type.create_screen(app_context))
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum BackendAction {
    VoteForContestant(Identifier, ResourceVoteChoice),
}

#[derive(Debug, PartialEq)]
pub enum AppAction {
    None,
    PopScreen,
    PopScreenAndRefresh,
    GoToMainScreen,
    SetMainScreen(RootScreenType),
    AddScreen(Screen),
    BackendAction(BackendAction),
}

impl BitOrAssign for AppAction {
    fn bitor_assign(&mut self, rhs: Self) {
        if matches!(rhs, AppAction::None) {
            // If rhs is None, keep the current value.
            return;
        }

        // Otherwise, assign rhs to self.
        *self = rhs;
    }
}
impl AppState {
    pub fn new() -> Self {
        let app_context = Arc::new(AppContext::new());

        let identities_screen = IdentitiesScreen::new(&app_context);
        let dpns_contested_names_screen = DPNSContestedNamesScreen::new(&app_context);

        Self {
            main_screens: [
                (
                    RootScreenType::RootScreenIdentities,
                    Screen::IdentitiesScreen(identities_screen),
                ),
                (
                    RootScreenType::RootScreenDPNSContestedNames,
                    Screen::DPNSContestedNamesScreen(dpns_contested_names_screen),
                ),
            ]
            .into(),
            selected_main_screen: RootScreenType::RootScreenIdentities,
            screen_stack: vec![],
            app_context,
        }
    }

    pub fn active_root_screen(&self) -> &Screen {
        self.main_screens
            .get(&self.selected_main_screen)
            .expect("expected to get screen")
    }

    pub fn active_root_screen_mut(&mut self) -> &mut Screen {
        self.main_screens
            .get_mut(&self.selected_main_screen)
            .expect("expected to get screen")
    }

    pub fn visible_screen(&self) -> &Screen {
        if let Some(last_screen) = self.screen_stack.last() {
            last_screen
        } else {
            self.active_root_screen()
        }
    }

    pub fn visible_screen_mut(&mut self) -> &mut Screen {
        if self.screen_stack.is_empty() {
            self.active_root_screen_mut()
        } else {
            self.screen_stack.last_mut().unwrap()
        }
    }

    pub fn visible_screen_type(&self) -> ScreenType {
        if let Some(last_screen) = self.screen_stack.last() {
            last_screen.screen_type()
        } else {
            self.selected_main_screen.into()
        }
    }
}

impl App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let action = self.visible_screen_mut().ui(ctx);

        match action {
            AppAction::AddScreen(screen) => self.screen_stack.push(screen),
            AppAction::None => {}
            AppAction::PopScreen => {
                if !self.screen_stack.is_empty() {
                    self.screen_stack.pop();
                }
            }
            AppAction::PopScreenAndRefresh => {
                if !self.screen_stack.is_empty() {
                    self.screen_stack.pop();
                }
                if let Some(screen) = self.screen_stack.last_mut() {
                    screen.refresh();
                } else {
                    self.active_root_screen_mut().refresh();
                }
            }
            AppAction::GoToMainScreen => {
                self.screen_stack = vec![];
                self.active_root_screen_mut().refresh();
            }
            AppAction::BackendAction(_) => {}
            AppAction::SetMainScreen(root_screen_type) => {
                self.selected_main_screen = root_screen_type;
                self.active_root_screen_mut().refresh();
            }
        }
    }
}
