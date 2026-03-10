use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::password_input::PasswordInput;
use egui::Ui;
use std::sync::{Arc, RwLock};

pub trait ScreenWithWalletUnlock {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>>;
    fn password_input(&mut self) -> &mut PasswordInput;
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

                let pw_response = self.password_input().show(ui);

                let enter_pressed = pw_response.response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));

                ui.add_space(5.0);

                let unlock_clicked = ui.button("Unlock").clicked();

                if enter_pressed || unlock_clicked {
                    let unlock_result = wallet.wallet_seed.open(self.password_input().text());

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

                    self.password_input().clear();
                }
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
