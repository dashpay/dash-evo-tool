use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::styled::StyledCheckbox;
use crate::ui::theme::DashColors;
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

    fn app_context(&self) -> Arc<AppContext>;

    fn should_ask_for_password(&mut self) -> bool {
        if let Some(wallet_guard) = self.selected_wallet_ref().clone() {
            let mut wallet = wallet_guard.write().unwrap();
            if !wallet.uses_password {
                if let Err(e) = wallet.wallet_seed.open_no_password() {
                    MessageBanner::set_global(
                        self.app_context().egui_ctx(),
                        &e,
                        MessageType::Error,
                    );
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
        let mut unlocked_wallet: Option<Arc<RwLock<Wallet>>> = None;

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

                // Capture necessary values before the closure
                let show_password = self.show_password();
                let mut local_show_password = show_password;
                let wallet_password_mut = self.wallet_password_mut();

                let mut attempt_unlock = false;

                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    let password_input = ui.add(
                        egui::TextEdit::singleline(wallet_password_mut)
                            .password(!local_show_password)
                            .hint_text("Enter password")
                            .text_color(DashColors::text_primary(dark_mode))
                            .background_color(DashColors::input_background(dark_mode)),
                    );

                    if password_input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        attempt_unlock = true;
                    }

                    ui.add_space(5.0);

                    // Checkbox to toggle password visibility
                    StyledCheckbox::new(&mut local_show_password, "Show Password").show(ui);
                });

                ui.add_space(5.0);

                if ui.button("Unlock").clicked() {
                    attempt_unlock = true;
                }

                if attempt_unlock {
                    // Use the password from wallet_password_mut
                    let wallet_password_ref = &*wallet_password_mut;

                    let unlock_result = wallet.wallet_seed.open(wallet_password_ref);

                    match unlock_result {
                        Ok(_) => {
                            unlocked_wallet = Some(wallet_guard.clone());
                        }
                        Err(_) => {
                            let error_msg = if let Some(hint) = wallet.password_hint() {
                                format!("Incorrect Password, password hint is {}", hint)
                            } else {
                                "Incorrect Password".to_string()
                            };
                            MessageBanner::set_global(ui.ctx(), &error_msg, MessageType::Error)
                                .with_auto_dismiss(std::time::Duration::from_secs(10));
                        }
                    }
                    // Clear the password field after submission
                    wallet_password_mut.zeroize();
                }

                // Update `show_password` after the closure
                *self.show_password_mut() = local_show_password;
                // Error display is handled by the global MessageBanner
            }
        }

        if let Some(wallet_arc) = unlocked_wallet {
            let app_context = self.app_context();
            app_context.handle_wallet_unlocked(&wallet_arc);
            return true;
        }

        false
    }
}
