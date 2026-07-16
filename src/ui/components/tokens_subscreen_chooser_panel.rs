use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::components::subscreen_chooser_panel::{
    SubscreenNavItem, add_subscreen_chooser_panel,
};
use crate::ui::tokens::tokens_screen::TokensSubscreen;
use egui::Ui;

pub fn add_tokens_subscreen_chooser_panel(ui: &mut Ui, app_context: &AppContext) -> AppAction {
    let active = match app_context.get_app_settings().root_screen_type {
        RootScreenType::RootScreenMyTokenBalances => TokensSubscreen::MyTokens,
        RootScreenType::RootScreenTokenSearch => TokensSubscreen::SearchTokens,
        RootScreenType::RootScreenTokenCreator => TokensSubscreen::TokenCreator,
        _ => TokensSubscreen::MyTokens,
    };

    let items = [
        (
            TokensSubscreen::MyTokens,
            RootScreenType::RootScreenMyTokenBalances,
        ),
        (
            TokensSubscreen::SearchTokens,
            RootScreenType::RootScreenTokenSearch,
        ),
        (
            TokensSubscreen::TokenCreator,
            RootScreenType::RootScreenTokenCreator,
        ),
    ]
    .into_iter()
    .map(|(subscreen, target)| {
        SubscreenNavItem::new(
            subscreen.display_name(),
            subscreen == active,
            AppAction::SetMainScreenThenGoToMainScreen(target),
        )
    })
    .collect();

    add_subscreen_chooser_panel(ui, "tokens_subscreen_chooser_panel", false, false, items)
}
