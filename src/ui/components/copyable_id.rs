//! A shortened ID with a copy button: the full ID shows on hover and goes to
//! the clipboard on click, with a success banner. Every string is supplied
//! whole by the caller, so each stays one translation unit.

use crate::model::identity_name::shorten_id;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use egui::{Response, RichText, Ui};

/// Copy `text` and confirm it with a success banner saying `copied_message`.
pub fn copy_with_confirmation(ctx: &egui::Context, text: &str, copied_message: &str) {
    ctx.copy_text(text.to_owned());
    MessageBanner::replace_global(ctx, copied_message, copied_message, MessageType::Success);
}

/// A shortened ID followed by a copy button.
pub struct CopyableId<'a> {
    full: &'a str,
    button_label: &'a str,
    tooltip: &'a str,
    copied_message: &'a str,
    button_size: egui::Vec2,
}

impl<'a> CopyableId<'a> {
    /// `button_label` names the copy button for screen readers, `tooltip`
    /// explains it on hover and `copied_message` confirms the copy.
    pub fn new(
        full: &'a str,
        button_label: &'a str,
        tooltip: &'a str,
        copied_message: &'a str,
    ) -> Self {
        Self {
            full,
            button_label,
            tooltip,
            copied_message,
            button_size: egui::Vec2::ZERO,
        }
    }

    /// The copy button's minimum size, to line it up with neighbouring buttons.
    pub fn button_size(mut self, size: egui::Vec2) -> Self {
        self.button_size = size;
        self
    }

    /// Paint the ID and the button; returns the button's response.
    pub fn show(self, ui: &mut Ui) -> Response {
        let dark = ui.visuals().dark_mode;
        ui.label(
            RichText::new(shorten_id(self.full))
                .monospace()
                .color(DashColors::text_secondary(dark)),
        )
        .info_tooltip(self.full);
        let copy = ui
            .add(ComponentStyles::secondary_button("📋", dark).min_size(self.button_size))
            .clickable_tooltip(self.tooltip);
        let label = self.button_label;
        copy.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if copy.clicked() {
            copy_with_confirmation(ui.ctx(), self.full, self.copied_message);
        }
        copy
    }
}
