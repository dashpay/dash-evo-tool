//! Technical details of an error: selectable monospace text with its actions.

use egui::{RichText, Ui};

use crate::ui::theme::{DashColors, Shape, Spacing, Typography};

/// Default height of the scrollable details text.
pub const DETAILS_MAX_HEIGHT: f32 = 120.0;

/// What the user did with the details this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TechnicalDetailsAction {
    /// Nothing.
    None,
    /// The details were copied to the clipboard.
    Copied,
}

/// Selectable error details in a framed scroll area, followed by an action row
/// with Copy details. Shared by every place that shows error details, so a new
/// action is added here once.
pub struct TechnicalDetails<'a> {
    id_salt: egui::IdSalt,
    details: &'a str,
    copy_text: Option<String>,
    max_height: f32,
}

impl<'a> TechnicalDetails<'a> {
    /// Details `details`; `id_salt` keeps each scroll position apart.
    pub fn new(id_salt: impl egui::AsIdSalt, details: &'a str) -> Self {
        Self {
            id_salt: egui::IdSalt::new(id_salt),
            details,
            copy_text: None,
            max_height: DETAILS_MAX_HEIGHT,
        }
    }

    /// Height of the scroll area holding the text.
    pub fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = max_height;
        self
    }

    /// Text Copy details puts on the clipboard instead of the shown details,
    /// e.g. with personal folder names removed.
    pub fn copy_text(mut self, text: String) -> Self {
        self.copy_text = Some(text);
        self
    }

    /// Show the details and their actions.
    pub fn show(&self, ui: &mut Ui) -> TechnicalDetailsAction {
        let dark_mode = ui.style().visuals.dark_mode;
        egui::Frame::new()
            .fill(DashColors::input_background(dark_mode))
            .inner_margin(egui::Margin::symmetric(Spacing::SM_I8, 6))
            .corner_radius(Shape::RADIUS_SM as f32)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(self.id_salt)
                    .max_height(self.max_height)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(self.details)
                                    .font(Typography::monospace())
                                    .color(DashColors::text_secondary(dark_mode)),
                            )
                            .wrap(),
                        );
                    });
            });
        ui.add_space(Spacing::XXS);
        // An unframed button: link styling, announced as a button.
        let copy = ui
            .add(
                egui::Button::new(
                    RichText::new("Copy details")
                        .font(Typography::body_small())
                        .color(DashColors::DASH_BLUE)
                        .underline(),
                )
                .frame(false),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if copy.clicked() {
            let text = self.copy_text.as_deref().unwrap_or(self.details);
            ui.ctx().copy_text(text.to_owned());
            return TechnicalDetailsAction::Copied;
        }
        TechnicalDetailsAction::None
    }
}
