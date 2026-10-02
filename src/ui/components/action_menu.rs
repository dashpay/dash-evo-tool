use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use egui::{Popup, Ui};
use std::sync::Arc;

/// Render a themed action popup and return the selected item's action.
pub(crate) fn show_action_menu<'a>(
    ui: &Ui,
    app_context: &Arc<AppContext>,
    popup: Popup<'_>,
    items: impl IntoIterator<Item = (&'a str, &'a DesiredAppAction, bool, &'a str)>,
) -> AppAction {
    let mut action = AppAction::None;
    popup
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(egui::Frame::popup(ui.style()).fill(DashColors::popup_fill(ui.visuals().dark_mode)))
        .show(|ui| {
            ui.set_min_width(150.0);
            for (label, desired_action, enabled, tooltip) in items {
                let mut response = ui
                    .add_enabled_ui(enabled, |ui| {
                        ComponentStyles::add_button(ui, egui::Button::new(label))
                    })
                    .inner;
                if !tooltip.is_empty() {
                    response = response
                        .clickable_tooltip(tooltip)
                        .disabled_tooltip(tooltip);
                }
                if response.clicked() {
                    action = desired_action.create_action(app_context);
                    ui.close();
                }
            }
        });
    action
}
