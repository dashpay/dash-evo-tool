use crate::model::wallet::Wallet;
use crate::ui::components::styled::StyledCheckbox;
use eframe::epaint::Color32;
use egui::Ui;
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

pub trait ScreenWithWalletUnlock {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>>;
    // Allow dead_code: This method provides read-only access to wallet passwords,
    // useful for password validation and UI state management
    #[allow(dead_code)]
    fn wallet_password_ref(&self) -> &String;
    fn wallet_password_mut(&mut self) -> &mut String;
    fn show_password(&self) -> bool;
    fn show_password_mut(&mut self) -> &mut bool;
    fn set_error_message(&mut self, error_message: Option<String>);

    fn error_message(&self) -> Option<&String>;

    fn should_ask_for_password(&mut self) -> bool {
        if let Some(wallet_guard) = self.selected_wallet_ref().clone() {
            let mut wallet = wallet_guard.write().unwrap();
            if !wallet.uses_password {
                if let Err(e) = wallet.wallet_seed.open_no_password() {
                    self.set_error_message(Some(e));
                }
                false
            } else {
                !wallet.is_open()
            }
        } else {
            true
        }
    }

    fn render_wallet_unlock_if_needed(&mut self, ui: &mut Ui) -> (bool, bool) {
        if self.should_ask_for_password() {
            (true, self.render_wallet_unlock(ui))
        } else {
            (false, false)
        }
    }

    fn render_wallet_unlock(&mut self, ui: &mut Ui) -> bool {
        if let Some(wallet_guard) = self.selected_wallet_ref().clone() {
            let mut wallet = wallet_guard.write().unwrap();

            // Only render the unlock prompt if the wallet requires a password and is locked
            if wallet.uses_password && !wallet.is_open() {
                if let Some(alias) = &wallet.alias {
                    ui.label(format!(
                        "This wallet ({}) is locked. Please enter the password to unlock it:",
                        alias
                    ));
                } else {
                    ui.label("This wallet is locked. Please enter the password to unlock it:");
                }

                ui.add_space(5.0);

                let mut unlocked = false;

                // Capture necessary values before the closure
                let show_password = self.show_password();
                let mut local_show_password = show_password; // Local copy of show_password
                let mut local_error_message = self.error_message().cloned(); // Local variable for error message
                let wallet_password_mut = self.wallet_password_mut(); // Mutable reference to the password

                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    let password_input = ui.add(
                        egui::TextEdit::singleline(wallet_password_mut)
                            .password(!local_show_password)
                            .hint_text("Enter password")
                            .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                            .background_color(crate::ui::theme::DashColors::input_background(
                                dark_mode,
                            )),
                    );

                    // Checkbox to toggle password visibility
                    StyledCheckbox::new(&mut local_show_password, "Show Password").show(ui);

                    if password_input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        // Use the password from wallet_password_mut
                        let wallet_password_ref = &*wallet_password_mut;

                        let unlock_result = wallet.wallet_seed.open(wallet_password_ref);

                        match unlock_result {
                            Ok(_) => {
                                local_error_message = None;
                                unlocked = true;
                            }
                            Err(_) => {
                                if let Some(hint) = wallet.password_hint() {
                                    local_error_message = Some(format!(
                                        "Incorrect Password, password hint is {}",
                                        hint
                                    ));
                                } else {
                                    local_error_message = Some("Incorrect Password".to_string());
                                }
                            }
                        }
                        // Clear the password field after submission
                        wallet_password_mut.zeroize();
                    }
                });

                // Update `show_password` after the closure
                *self.show_password_mut() = local_show_password;

                // Update the error message
                self.set_error_message(local_error_message);

                // Display error message if the password was incorrect
                if let Some(error_message) = self.error_message() {
                    ui.add_space(5.0);
                    ui.colored_label(Color32::RED, error_message);
                }

                return unlocked;
            }
        }
        false
    }
}
