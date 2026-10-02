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
            let items: Vec<_> = items.into_iter().collect();
            let width = items.iter().fold(150.0_f32, |width, (label, ..)| {
                let galley = egui::WidgetText::from(*label).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::FontSelection::Style(egui::TextStyle::Button),
                );
                width.max((galley.size().x + 2.0 * ui.spacing().button_padding.x).ceil())
            });
            ui.set_width(width);
            for (label, desired_action, enabled, tooltip) in items {
                let mut response = ui
                    .add_enabled_ui(enabled, |ui| {
                        ComponentStyles::add_button(
                            ui,
                            egui::Button::new(label).min_size(egui::vec2(width, 0.0)),
                        )
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
