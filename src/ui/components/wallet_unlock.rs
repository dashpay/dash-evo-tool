use std::sync::{Arc, RwLock};
use eframe::epaint::Color32;
use egui::Ui;
use zeroize::Zeroize;
use crate::model::wallet::Wallet;

pub trait ScreenWithWalletUnlock {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>>;
    fn wallet_password_ref(&self) -> &String;
    fn wallet_password_mut(&mut self) -> &mut String;
    fn show_password(&self) -> bool;
    fn show_password_mut(&mut self) -> &mut bool;
    fn set_error_message(&mut self, error_message: Option<String>);

    fn error_message(&self) -> Option<&String>;
    fn render_wallet_unlock(&mut self, ui: &mut Ui) -> bool {
        if let Some(wallet_guard) = self.selected_wallet_ref().as_ref() {
            let mut wallet = wallet_guard.write().unwrap();

            // Only render the unlock prompt if the wallet requires a password and is locked
            if wallet.uses_password && !wallet.is_open() {
                ui.add_space(10.0);
                ui.label("This wallet is locked. Please enter the password to unlock it:");

                let mut unlocked = false;
                ui.horizontal(|ui| {
                    let password_input = ui.add(
                        egui::TextEdit::singleline(self.wallet_password_mut())
                            .password(!self.show_password())
                            .hint_text("Enter password"),
                    );

                    ui.checkbox(self.show_password_mut(), "Show Password");

                    unlocked = if password_input.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        let unlocked = match wallet.wallet_seed.open(&self.wallet_password_ref()) {
                            Ok(_) => {
                                self.set_error_message(None); // Clear any previous error
                                true
                            }
                            Err(_) => {
                                if let Some(hint) = wallet.password_hint() {
                                    self.set_error_message(Some(format!(
                                        "Incorrect Password, password hint is {}",
                                        hint
                                    )));
                                } else {
                                    self.set_error_message(Some("Incorrect Password".to_string()));
                                }
                                false
                            }
                        };
                        // Clear the password field after submission
                        self.wallet_password_mut().zeroize();
                        unlocked
                    } else {
                        false
                    };
                });

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