//! Reusable fee confirmation dialog component.
//!
//! Shown when the network requires a higher fee than initially estimated.
//! The user can confirm sending with the higher fee or cancel.

use crate::ui::theme::DashColors;
use eframe::egui::{self, Context, RichText};
use egui::{Color32, Frame, Margin};

/// User's response from the fee confirmation dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum FeeConfirmationResponse {
    /// User confirmed — proceed with the required (higher) fee.
    Confirmed { override_fee: u64 },
    /// User canceled — abort the send.
    Canceled,
}

/// State for the fee confirmation dialog shown when min relay fee is higher
/// than estimated.
#[derive(Debug, Clone, Default)]
pub struct FeeConfirmationDialog {
    is_open: bool,
    estimated_fee: u64,
    required_fee: u64,
}

impl FeeConfirmationDialog {
    /// Open the dialog with the given estimated and required fee values.
    pub fn open(&mut self, estimated_fee: u64, required_fee: u64) {
        self.is_open = true;
        self.estimated_fee = estimated_fee;
        self.required_fee = required_fee;
    }

    /// Whether the dialog is currently visible.
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Close the dialog without returning a response.
    pub fn close(&mut self) {
        self.is_open = false;
    }

    /// Render the dialog. Returns `Some(response)` when the user clicks
    /// Confirm or Cancel, `None` while the dialog is still open.
    pub fn show(&mut self, ctx: &Context) -> Option<FeeConfirmationResponse> {
        if !self.is_open {
            return None;
        }

        let dark_mode = ctx.style().visuals.dark_mode;
        let mut result = None;

        egui::Window::new("Fee Confirmation Required")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(10.0);

                ui.label(
                    RichText::new("The network requires a higher fee than estimated.")
                        .color(DashColors::text_primary(dark_mode))
                        .size(14.0),
                );

                ui.add_space(15.0);

                Frame::group(ui.style())
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(12, 10))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated fee:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} duffs ({:.8} DASH)",
                                    self.estimated_fee,
                                    self.estimated_fee as f64 * 1e-8
                                ))
                                .color(DashColors::text_primary(dark_mode)),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Required fee:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} duffs ({:.8} DASH)",
                                    self.required_fee,
                                    self.required_fee as f64 * 1e-8
                                ))
                                .color(DashColors::WARNING)
                                .strong(),
                            );
                        });

                        let fee_diff = self.required_fee.saturating_sub(self.estimated_fee);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Additional cost:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "+{} duffs ({:.8} DASH)",
                                    fee_diff,
                                    fee_diff as f64 * 1e-8
                                ))
                                .color(DashColors::text_primary(dark_mode)),
                            );
                        });
                    });

                ui.add_space(15.0);

                ui.label(
                    RichText::new("Would you like to proceed with the higher fee?")
                        .color(DashColors::text_primary(dark_mode)),
                );

                ui.add_space(15.0);

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        result = Some(FeeConfirmationResponse::Canceled);
                    }

                    ui.add_space(20.0);

                    let confirm_button = egui::Button::new(
                        RichText::new("Confirm & Send")
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(DashColors::DASH_BLUE);

                    if ui.add(confirm_button).clicked() {
                        result = Some(FeeConfirmationResponse::Confirmed {
                            override_fee: self.required_fee,
                        });
                    }
                });

                ui.add_space(10.0);
            });

        if result.is_some() {
            self.is_open = false;
        }

        result
    }
}

/// Parse the required fee from a "min relay fee not met" error message.
///
/// Returns `Some(required_fee)` if the error matches the pattern
/// `"min relay fee not met, X < Y"`, extracting `Y`.
pub fn parse_min_relay_fee_error(error: &str) -> Option<u64> {
    if (error.contains("min relay fee not met") || error.contains("min relay fee"))
        && let Some(pos) = error.find('<')
    {
        let after_lt = &error[pos + 1..];
        let num_str: String = after_lt
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(required_fee) = num_str.parse::<u64>() {
            return Some(required_fee);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_min_relay_fee_error_basic() {
        let error = "min relay fee not met, 226 < 1000";
        assert_eq!(parse_min_relay_fee_error(error), Some(1000));
    }

    #[test]
    fn test_parse_min_relay_fee_error_no_match() {
        assert_eq!(parse_min_relay_fee_error("insufficient funds"), None);
    }

    #[test]
    fn test_parse_min_relay_fee_error_partial() {
        let error = "min relay fee not met";
        assert_eq!(parse_min_relay_fee_error(error), None);
    }

    #[test]
    fn test_fee_confirmation_dialog_default_closed() {
        let dialog = FeeConfirmationDialog::default();
        assert!(!dialog.is_open());
    }
}
