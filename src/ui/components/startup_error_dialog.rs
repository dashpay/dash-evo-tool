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
/// Viewport width the window frame and margins take beside the body.
const WINDOW_SIDE_ROOM: f32 = 48.0;
/// Viewport height the title bar, margins and action row take beside the body.
const WINDOW_CHROME_HEIGHT: f32 = 140.0;
/// The body never shrinks below this, even in a tiny window.
const MIN_BODY_HEIGHT: f32 = 60.0;

/// What the user asked for in the startup failure dialog this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupErrorDialogAction {
    /// Nothing.
    None,
    /// Check the profile again.
    Retry,
    /// Quit the app (Quit or the window's close button).
    Quit,
}

/// Renders a [`StartupFailure`] with its technical details and recovery actions.
///
/// The only content of the app window after a fatal startup failure; nothing
/// else is built behind it.
pub struct StartupErrorDialog<'a> {
    failure: &'a StartupFailure,
    retryable: bool,
}

impl<'a> StartupErrorDialog<'a> {
    /// Dialog for `failure`; value-constructed each frame.
    pub fn new(failure: &'a StartupFailure) -> Self {
        Self {
            failure,
            retryable: false,
        }
    }

    /// Show Try again when the owner can repeat the failed step.
    pub(crate) fn with_retry(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
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
                // Fit the viewport: the body scrolls, the action row stays visible.
                let screen = ui.ctx().content_rect();
                let width = screen.width() - WINDOW_SIDE_ROOM;
                ui.set_min_width(MIN_WIDTH.min(width));
                ui.set_max_width(MAX_WIDTH.min(width));
                let dark_mode = ui.style().visuals.dark_mode;
                egui::ScrollArea::vertical()
                    .id_salt("startup_error_body")
                    .max_height((screen.height() - WINDOW_CHROME_HEIGHT).max(MIN_BODY_HEIGHT))
                    .show(ui, |ui| {
                        self.message(ui, dark_mode);
                        ui.add_space(Spacing::MD);
                        self.support(ui, dark_mode);
                        ui.add_space(Spacing::MD);
                        self.details(ui, dark_mode);
                    });
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
                "If you cannot solve this yourself, report the problem at the address below and include the technical details. The issue page is public, and the details can include folder names from your computer, so read them before posting and remove anything you do not want to share.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.horizontal_wrapped(|ui| {
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
            if self.retryable {
                if ComponentStyles::add_primary_button(ui, "Try again").clicked() {
                    action = StartupErrorDialogAction::Retry;
                }
                if ComponentStyles::add_secondary_button(ui, "Quit", ui.visuals().dark_mode)
                    .clicked()
                {
                    action = StartupErrorDialogAction::Quit;
                }
            } else if ComponentStyles::add_primary_button(ui, "Quit").clicked() {
                action = StartupErrorDialogAction::Quit;
            }
        });
        action
    }
}
