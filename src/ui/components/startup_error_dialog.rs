//! Fatal startup failure window: what happened, where to get help, and the
//! technical details to send.

use egui::{RichText, Ui};

use crate::model::error_report::StartupFailure;
use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::ui::components::technical_details::TechnicalDetails;
use crate::ui::theme::{ComponentStyles, DashColors, Spacing, Typography};

/// Window title of the startup failure dialog.
pub const STARTUP_ERROR_TITLE: &str = "Dash Evo Tool could not start";

const MIN_WIDTH: f32 = 420.0;
const MAX_WIDTH: f32 = 640.0;
const DETAILS_HEIGHT: f32 = 220.0;

/// What the user asked for in the startup failure dialog this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupErrorDialogAction {
    /// Nothing.
    None,
    /// Quit the app (Quit or the window's close button).
    Quit,
}

/// Renders a [`StartupFailure`] with its technical details and a Quit action.
///
/// The only content of the app window after a fatal startup failure; nothing
/// else is built behind it.
pub struct StartupErrorDialog<'a> {
    failure: &'a StartupFailure,
}

impl<'a> StartupErrorDialog<'a> {
    /// Dialog for `failure`; value-constructed each frame.
    pub fn new(failure: &'a StartupFailure) -> Self {
        Self { failure }
    }

    /// Show the dialog and report the user's action.
    pub fn show(&self, ui: &mut Ui) -> StartupErrorDialogAction {
        let mut action = StartupErrorDialogAction::None;
        let chrome = modal_chrome(
            ui.ctx(),
            ModalChromeConfig {
                title: STARTUP_ERROR_TITLE.into(),
                overlay_id: egui::Id::unique("startup_error_dialog_overlay"),
                overlay_order: egui::Order::Background,
                window_order: egui::Order::Middle,
                resizable: true,
                show_close_button: true,
                blocks_input: false,
                inner_margin: Spacing::MD_I8,
            },
            |ui| {
                ui.set_min_width(MIN_WIDTH);
                ui.set_max_width(MAX_WIDTH);
                let dark_mode = ui.style().visuals.dark_mode;
                self.message(ui, dark_mode);
                ui.add_space(Spacing::MD);
                self.support(ui, dark_mode);
                ui.add_space(Spacing::MD);
                self.details(ui, dark_mode);
                ui.add_space(Spacing::MD);
                action = self.actions(ui);
            },
        );
        if chrome.closed_via_x {
            action = StartupErrorDialogAction::Quit;
        }
        action
    }

    fn message(&self, ui: &mut Ui, dark_mode: bool) {
        ui.label(
            RichText::new(self.failure.report().message())
                .color(DashColors::text_primary(dark_mode)),
        );
    }

    fn support(&self, ui: &mut Ui, dark_mode: bool) {
        ui.label(
            RichText::new(
                "If you cannot solve this yourself, report the problem at the address below and include the technical details.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::SUPPORT_URL).font(Typography::monospace()));
            ui.hyperlink_to("Open in browser", crate::SUPPORT_URL);
        });
    }

    fn details(&self, ui: &mut Ui, dark_mode: bool) {
        ui.label(RichText::new("Technical details").strong());
        let details = format!(
            "Dash Evo Tool {}\n{}",
            crate::VERSION,
            self.failure.report().details()
        );
        TechnicalDetails::new("startup_error_details", &details)
            .max_height(DETAILS_HEIGHT)
            .show(ui);
        if let Some(log_file) = self.failure.log_file() {
            ui.add_space(Spacing::SM);
            ui.label(
                RichText::new(format!(
                    "The log file is at {path}.",
                    path = log_file.display()
                ))
                .color(DashColors::text_secondary(dark_mode)),
            );
        }
    }

    fn actions(&self, ui: &mut Ui) -> StartupErrorDialogAction {
        let mut action = StartupErrorDialogAction::None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ComponentStyles::add_primary_button(ui, "Quit").clicked() {
                action = StartupErrorDialogAction::Quit;
            }
        });
        action
    }
}
