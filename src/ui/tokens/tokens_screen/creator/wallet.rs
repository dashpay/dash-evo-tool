use super::*;
use crate::ui::{MessageType, components::MessageBanner};

impl TokensScreen {
    pub(super) fn update_selected_wallet(&mut self) {
        if let (Some(qid), Some(key)) = (&self.selected_identity, &self.selected_key) {
            let new_wallet = crate::ui::identity::get_selected_wallet(qid, None, Some(key))
                .unwrap_or_else(|e| {
                    MessageBanner::set_global(self.app_context.egui_ctx(), &e, MessageType::Error);
                    None
                });
            let wallet_changed = match (&self.selected_wallet, &new_wallet) {
                (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if wallet_changed {
                self.wallet_open_attempted = false;
            }
            self.selected_wallet = new_wallet;
        }
    }
    pub(super) fn ensure_wallet_unlocked(&mut self, ui: &mut Ui) -> bool {
        if let Some(wallet) = &self.selected_wallet {
            use crate::ui::components::wallet_unlock_popup::{
                try_open_wallet_no_password, wallet_needs_unlock,
            };

            if !self.wallet_open_attempted {
                if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                    MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                        .disable_auto_dismiss();
                }
                self.wallet_open_attempted = true;
            }

            if wallet_needs_unlock(wallet) {
                ui.add_space(10.0);
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 50),
                    "Wallet is locked. Please unlock to continue.",
                );
                ui.add_space(8.0);
                if ui.button("Unlock Wallet").clicked() {
                    self.wallet_unlock_popup.open();
                }
                return false;
            }
        }

        true
    }
}
