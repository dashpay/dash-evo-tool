use crate::app::AppAction;
use crate::ui::theme::{DashColors, Shape, Spacing, Typography};
use egui::{Frame, Margin, Panel, RichText, ScrollArea, Ui};

/// A single navigation entry in a left-hand subscreen chooser panel.
pub struct SubscreenNavItem {
    label: String,
    is_active: bool,
    action: AppAction,
}

impl SubscreenNavItem {
    /// Creates a navigation entry with its label, active state, and the action to dispatch on click.
    pub fn new(label: impl Into<String>, is_active: bool, action: AppAction) -> Self {
        Self {
            label: label.into(),
            is_active,
            action,
        }
    }
}

/// Builds the standard left-panel navigation button used across every subscreen chooser.
pub fn nav_button(label: &str, is_active: bool, dark_mode: bool) -> egui::Button<'static> {
    let (text_color, fill, stroke) = if is_active {
        (
            DashColors::info_color(dark_mode),
            DashColors::selected(dark_mode),
            egui::Stroke::NONE,
        )
    } else {
        (
            DashColors::text_primary(dark_mode),
            egui::Color32::TRANSPARENT,
            egui::Stroke::NONE,
        )
    };
    egui::Button::new(
        RichText::new(label)
            .color(text_color)
            .size(Typography::SCALE_SM),
    )
    .fill(fill)
    .stroke(stroke)
    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
}

/// Renders a left-hand subscreen chooser panel and returns the action of the clicked item.
///
/// The panel draws an island frame containing one [`nav_button`] per item. Set `scrollable` for
/// panels whose entries may overflow the available height.
pub fn add_subscreen_chooser_panel(
    ui: &mut Ui,
    panel_id: &str,
    resizable: bool,
    scrollable: bool,
    items: Vec<SubscreenNavItem>,
) -> AppAction {
    let dark_mode = ui.ctx().global_style().visuals.dark_mode;
    let mut action = AppAction::None;

    Panel::left(egui::Id::new(panel_id))
        .resizable(resizable)
        .default_size(200.0)
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)),
        )
        .show(ui, |ui| {
            let available_height = ui.available_height();
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::same(Spacing::MD as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .show(ui, |ui| {
                    ui.set_min_height(available_height - 2.0 - (Spacing::MD * 2.0));
                    let render = |ui: &mut Ui| {
                        ui.add_space(Spacing::SM);
                        for item in items {
                            let clicked = ui
                                .add(nav_button(&item.label, item.is_active, dark_mode))
                                .clicked();
                            ui.add_space(Spacing::SM);
                            if clicked {
                                action = item.action;
                            }
                        }
                    };
                    if scrollable {
                        ScrollArea::vertical().show(ui, render);
                    } else {
                        ui.vertical(render);
                    }
                });
        });

    action
}
