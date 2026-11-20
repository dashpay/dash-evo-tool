//! Fatal startup failure window: what happened, where to get help, and the
//! technical details to send.

use egui::{RichText, Ui};

use crate::boot::StartupFailure;
use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::ui::theme::{ComponentStyles, DashColors, Typography};

/// Window title of the startup failure dialog.
pub const STARTUP_ERROR_TITLE: &str = "Dash Evo Tool could not start";

/// Where users report a startup failure they cannot solve themselves.
pub const SUPPORT_URL: &str = "https://github.com/dashpay/dash-evo-tool/issues";

/// Renders a [`StartupFailure`] with a Copy details and a Quit action.
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

    /// Show the dialog. Returns `true` when the user asked to quit (Quit or the
    /// window's close button).
    pub fn show(&self, ui: &mut Ui) -> bool {
        let mut quit = false;
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
                inner_margin: 16,
            },
            |ui| {
                ui.set_min_width(420.0);
                ui.set_max_width(640.0);
                let dark_mode = ui.style().visuals.dark_mode;
                self.message(ui, dark_mode);
                ui.add_space(12.0);
                self.support(ui, dark_mode);
                ui.add_space(12.0);
                self.details(ui, dark_mode);
                ui.add_space(16.0);
                quit = self.actions(ui, dark_mode);
            },
        );
        quit || chrome.closed_via_x
    }

    fn message(&self, ui: &mut Ui, dark_mode: bool) {
        ui.label(RichText::new(self.failure.message()).color(DashColors::text_primary(dark_mode)));
    }

    fn support(&self, ui: &mut Ui, dark_mode: bool) {
        ui.label(
            RichText::new(
                "If you cannot solve this yourself, report the problem at the address below and include the technical details.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(SUPPORT_URL).font(Typography::monospace()));
            ui.hyperlink_to("Open in browser", SUPPORT_URL);
        });
    }

    fn details(&self, ui: &mut Ui, dark_mode: bool) {
        ui.label(RichText::new("Technical details").strong());
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.label(
                    RichText::new(self.failure.details())
                        .font(Typography::monospace())
                        .color(DashColors::text_secondary(dark_mode)),
                );
            });
        if let Some(log_file) = self.failure.log_file() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "The log file is at {path}.",
                    path = log_file.display()
                ))
                .color(DashColors::text_secondary(dark_mode)),
            );
        }
    }

    /// Returns `true` when Quit was clicked.
    fn actions(&self, ui: &mut Ui, dark_mode: bool) -> bool {
        let mut quit = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            quit = ComponentStyles::add_primary_button(ui, "Quit").clicked();
            if ComponentStyles::add_secondary_button(ui, "Copy details", dark_mode).clicked() {
                ui.ctx().copy_text(self.failure.details().to_owned());
            }
        });
        quit
    }
}
