use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::helpers::clicked_outside_window;
use crate::ui::theme::{ComponentStyles, DashColors};
use egui;
use std::sync::{Arc, RwLock};

/// Result of showing the wallet unlock popup
#[derive(Debug, Clone, PartialEq)]
pub enum WalletUnlockResult {
    /// Popup is still open, no action taken yet
    Pending,
    /// User successfully unlocked the wallet
    Unlocked,
    /// User cancelled the unlock
    Cancelled,
}

/// A popup dialog for unlocking a wallet with password
/// Similar to InfoPopup and ConfirmationDialog but specialized for wallet unlock flow
pub struct WalletUnlockPopup {
    is_open: bool,
    password_input: PasswordInput,
    error_message: Option<String>,
}

impl Default for WalletUnlockPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl WalletUnlockPopup {
    /// Create a new wallet unlock popup
    pub fn new() -> Self {
        Self {
            is_open: false,
            password_input: PasswordInput::new().with_hint_text("Enter password"),
            error_message: None,
        }
    }

    /// Open the popup
    pub fn open(&mut self) {
        self.is_open = true;
        self.password_input.clear();
        self.error_message = None;
    }

    /// Close the popup
    pub fn close(&mut self) {
        self.is_open = false;
        self.password_input.clear();
        self.error_message = None;
    }

    /// Check if the popup is currently open
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Show the popup and handle wallet unlock
    /// Returns the result of the unlock attempt
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
    ) -> WalletUnlockResult {
        if !self.is_open {
            return WalletUnlockResult::Pending;
        }

        // Draw dark overlay behind the popup
        let screen_rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("wallet_unlock_popup_overlay"),
        ));
        painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

        let mut result = WalletUnlockResult::Pending;

        // Get wallet alias for display
        let wallet_alias = wallet
            .read()
            .ok()
            .and_then(|w| w.alias.clone())
            .unwrap_or_else(|| "Wallet".to_string());

        let mut is_open = true;

        let window_response = egui::Window::new("Unlock Wallet")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut is_open)
            .frame(egui::Frame {
                inner_margin: egui::Margin::same(20),
                outer_margin: egui::Margin::same(0),
                corner_radius: egui::CornerRadius::same(8),
                shadow: egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 16,
                    spread: 0,
                    color: DashColors::popup_shadow(),
                },
                fill: ctx.style().visuals.window_fill,
                stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
            })
            .show(ctx, |ui| {
                ui.set_min_width(350.0);
                ui.set_max_width(400.0);

                let dark_mode = ui.ctx().style().visuals.dark_mode;

                // Title/description
                ui.label(
                    egui::RichText::new(format!("Enter password to unlock \"{}\":", wallet_alias))
                        .color(DashColors::text_primary(dark_mode)),
                );

                ui.add_space(12.0);

                let mut attempt_unlock = false;

                let pw_response = self.password_input.show(ui);

                // Focus the password field when popup opens
                if pw_response.response.gained_focus() || self.password_input.is_empty() {
                    pw_response.response.request_focus();
                }

                // Check for Enter key
                if pw_response.response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    attempt_unlock = true;
                }

                // Error message
                if let Some(error) = &self.error_message {
                    ui.add_space(8.0);
                    ui.colored_label(DashColors::ERROR, error);
                }

                ui.add_space(16.0);

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Unlock button (right side)
                        if ComponentStyles::add_primary_button(ui, "Unlock").clicked() {
                            attempt_unlock = true;
                        }

                        // Cancel button (left side)
                        if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked()
                        {
                            result = WalletUnlockResult::Cancelled;
                            self.close();
                        }

                        ui.add_space(8.0);
                    });
                });

                // Attempt unlock if requested
                if attempt_unlock {
                    let mut wallet_guard = wallet.write().unwrap();
                    match wallet_guard.wallet_seed.open(self.password_input.text()) {
                        Ok(_) => {
                            // Notify app context that wallet was unlocked
                            drop(wallet_guard); // Release write lock before calling handle_wallet_unlocked
                            app_context.handle_wallet_unlocked(wallet);
                            result = WalletUnlockResult::Unlocked;
                            self.close();
                        }
                        Err(_) => {
                            // Show error with hint if available
                            if let Some(hint) = wallet_guard.password_hint() {
                                self.error_message =
                                    Some(format!("Incorrect password. Hint: {}", hint));
                            } else {
                                self.error_message = Some("Incorrect password".to_string());
                            }
                            self.password_input.clear();
                        }
                    }
                }
            });

        // Handle window being closed via X button
        if !is_open {
            result = WalletUnlockResult::Cancelled;
            self.close();
        }

        // Handle Escape key
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = WalletUnlockResult::Cancelled;
            self.close();
        }

        // Handle click outside window
        if let Some(ref wr) = window_response
            && result == WalletUnlockResult::Pending
            && clicked_outside_window(ctx, wr.response.rect)
        {
            result = WalletUnlockResult::Cancelled;
            self.close();
        }

        result
    }
}

/// Helper function to check if a wallet needs unlocking
pub fn wallet_needs_unlock(wallet: &Arc<RwLock<Wallet>>) -> bool {
    let wallet_guard = wallet.read().unwrap();
    wallet_guard.uses_password && !wallet_guard.is_open()
}

/// Helper function to try opening a wallet without password (for wallets that don't use passwords)
pub fn try_open_wallet_no_password(wallet: &Arc<RwLock<Wallet>>) -> Result<(), String> {
    let mut wallet_guard = wallet.write().unwrap();
    if !wallet_guard.uses_password {
        wallet_guard.wallet_seed.open_no_password()
    } else {
        Ok(())
    }
}
