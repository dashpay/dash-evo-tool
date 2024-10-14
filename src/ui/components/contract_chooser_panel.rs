use crate::app::AppAction;
use crate::context::AppContext;
use egui::Context;
use std::sync::Arc;

pub fn add_contract_chooser_panel(ctx: &Context, app_context: &Arc<AppContext>) -> AppAction {
    AppAction::None
}
