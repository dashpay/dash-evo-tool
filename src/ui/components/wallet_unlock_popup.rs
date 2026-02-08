use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::wallet::Wallet;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::components::styled::StyledCheckbox;
use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use egui;
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

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
    password: String,
    show_password: bool,
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
            password: String::new(),
            show_password: false,
            error_message: None,
        }
    }

    /// Open the popup
    pub fn open(&mut self) {
        self.is_open = true;
        self.password.clear();
        self.error_message = None;
    }

    /// Close the popup
    pub fn close(&mut self) {
        self.is_open = false;
        self.password.zeroize();
        self.error_message = None;
    }

    /// Check if the popup is currently open
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Show the popup and handle HD wallet unlock
    /// Returns the result of the unlock attempt
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
    ) -> WalletUnlockResult {
        let wallet_alias = wallet
            .read()
            .ok()
            .and_then(|w| w.alias.clone())
            .unwrap_or_else(|| "Wallet".to_string());

        self.show_inner(ctx, &wallet_alias, |password| {
            let mut wallet_guard = wallet.write_or_recover();
            match wallet_guard.wallet_seed.open(password) {
                Ok(_) => {
                    // Notify app context that wallet was unlocked
                    drop(wallet_guard); // Release write lock before calling handle_wallet_unlocked
                    app_context.handle_wallet_unlocked(wallet);
                    Ok(())
                }
                Err(_) => {
                    // Return error with hint if available
                    if let Some(hint) = wallet_guard.password_hint() {
                        Err(format!("Incorrect password. Hint: {}", hint))
                    } else {
                        Err("Incorrect password".to_string())
                    }
                }
            }
        })
    }

    /// Show the popup and handle single-key wallet unlock
    /// Returns the result of the unlock attempt
    pub fn show_single_key(
        &mut self,
        ctx: &egui::Context,
        wallet: &Arc<RwLock<SingleKeyWallet>>,
    ) -> WalletUnlockResult {
        let wallet_alias = wallet
            .read()
            .ok()
            .and_then(|w| w.alias.clone())
            .unwrap_or_else(|| "Wallet".to_string());

        self.show_inner(ctx, &wallet_alias, |password| {
            let mut wallet_guard = wallet.write_or_recover();
            wallet_guard
                .open(password)
                .map_err(|e| format!("Failed to unlock: {}", e))
        })
    }

    /// Shared popup UI for both HD and single-key wallets.
    /// The `try_unlock` closure attempts to unlock the wallet with the given password,
    /// returning `Ok(())` on success or `Err(message)` on failure.
    fn show_inner(
        &mut self,
        ctx: &egui::Context,
        wallet_alias: &str,
        try_unlock: impl FnOnce(&str) -> Result<(), String>,
    ) -> WalletUnlockResult {
        if !self.is_open {
            return WalletUnlockResult::Pending;
        }

        // Draw dark overlay behind the popup
        super::modal_overlay::draw_modal_overlay(ctx, "wallet_unlock_popup_overlay");

        let mut result = WalletUnlockResult::Pending;

        let mut is_open = true;

        egui::Window::new("Unlock Wallet")
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

                // Password input
                let mut attempt_unlock = false;

                let password_response = ui.add(
                    egui::TextEdit::singleline(&mut self.password)
                        .password(!self.show_password)
                        .hint_text("Enter password")
                        .desired_width(f32::INFINITY)
                        .text_color(DashColors::text_primary(dark_mode))
                        .background_color(DashColors::input_background(dark_mode)),
                );

                // Focus the password field when popup opens
                if password_response.gained_focus() || self.password.is_empty() {
                    password_response.request_focus();
                }

                // Check for Enter key
                if password_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    attempt_unlock = true;
                }

                ui.add_space(8.0);

                // Show password checkbox
                ui.horizontal(|ui| {
                    StyledCheckbox::new(&mut self.show_password, "Show password").show(ui);
                });

                // Error message
                if let Some(error) = &self.error_message {
                    ui.add_space(8.0);
                    ui.colored_label(DashColors::ERROR, error);
                }

                ui.add_space(16.0);

                // Buttons
                ui.horizontal(|ui| {
                    // Cancel button
                    let cancel_button = egui::Button::new(
                        egui::RichText::new("Cancel").color(DashColors::text_primary(dark_mode)),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(
                        1.0,
                        DashColors::text_secondary(dark_mode),
                    ))
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                    .min_size(egui::Vec2::new(80.0, 32.0));

                    if ui
                        .add(cancel_button)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        result = WalletUnlockResult::Cancelled;
                        self.close();
                    }

                    ui.add_space(8.0);

                    // Unlock button
                    let unlock_button = egui::Button::new(
                        egui::RichText::new("Unlock").color(ComponentStyles::primary_button_text()),
                    )
                    .fill(ComponentStyles::primary_button_fill())
                    .stroke(ComponentStyles::primary_button_stroke())
                    .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                    .min_size(egui::Vec2::new(80.0, 32.0));

                    if ui
                        .add(unlock_button)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        attempt_unlock = true;
                    }
                });

                // Attempt unlock if requested
                if attempt_unlock {
                    match try_unlock(&self.password) {
                        Ok(()) => {
                            result = WalletUnlockResult::Unlocked;
                            self.close();
                        }
                        Err(msg) => {
                            self.error_message = Some(msg);
                            self.password.zeroize();
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

        result
    }
}

impl Drop for WalletUnlockPopup {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

/// Helper function to check if a wallet needs unlocking
pub fn wallet_needs_unlock(wallet: &Arc<RwLock<Wallet>>) -> bool {
    let wallet_guard = wallet.read_or_recover();
    wallet_guard.uses_password && !wallet_guard.is_open()
}

/// Helper function to try opening a wallet without password (for wallets that don't use passwords)
pub fn try_open_wallet_no_password(wallet: &Arc<RwLock<Wallet>>) -> Result<(), String> {
    let mut wallet_guard = wallet.write_or_recover();
    if !wallet_guard.uses_password {
        wallet_guard.wallet_seed.open_no_password()
    } else {
        Ok(())
    }
}

/// Helper function to check if a single-key wallet needs unlocking
pub fn single_key_wallet_needs_unlock(wallet: &Arc<RwLock<SingleKeyWallet>>) -> bool {
    let wallet_guard = wallet.read_or_recover();
    wallet_guard.uses_password && !wallet_guard.is_open()
}

/// Helper function to try opening a single-key wallet without password
pub fn try_open_single_key_wallet_no_password(
    wallet: &Arc<RwLock<SingleKeyWallet>>,
) -> Result<(), String> {
    let mut wallet_guard = wallet.write_or_recover();
    if !wallet_guard.uses_password {
        wallet_guard.open_no_password()
    } else {
        Ok(())
    }
}
