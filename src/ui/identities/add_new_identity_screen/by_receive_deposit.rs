use crate::app::AppAction;
use crate::model::amount::Amount;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::identities::funding_common::{
    FundingMethod, WalletFundedScreenStep, generate_qr_code_image, round_up_dash_4dp,
};
use crate::ui::theme::DashColors;
use crate::wallet_backend::poison::RwLockRecover;
use egui::{Color32, RichText, Ui, Vec2};
use std::time::Duration;

impl AddNewIdentityScreen {
    /// The minimum credits needed to create the identity with the current key
    /// set — shown as the amount to deposit.
    fn deposit_minimum_credits(&self) -> u64 {
        let key_count = self.identity_keys.others.len() + 1; // +1 for master key
        self.app_context
            .fee_estimator()
            .estimate_identity_create(key_count)
    }

    /// The wallet's cumulative spendable balance, in duffs, or `0` when no
    /// wallet is selected or its lock is momentarily busy.
    fn selected_wallet_spendable_duffs(&self) -> u64 {
        self.selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok())
            .map(|w| {
                self.app_context
                    .snapshot_balance(&w.seed_hash())
                    .spendable()
            })
            .unwrap_or(0)
    }

    /// Queue a deposit-address derivation unless one is already shown, in
    /// flight, or a prior derivation failed. Idempotent, so it is safe to call
    /// every frame from the QR view.
    fn queue_funding_address_request(&mut self) {
        if self.funding_address.is_some()
            || self.pending_funding_address_request.is_some()
            || self.funding_address_request_failed
        {
            return;
        }
        if let Some(wallet) = &self.selected_wallet
            && let Ok(seed_hash) = wallet.read().map(|w| w.seed_hash())
        {
            self.pending_funding_address_request = Some(seed_hash);
        }
    }

    fn render_deposit_qr(&mut self, ui: &mut Ui) {
        let Some(address) = self.funding_address.clone() else {
            if self.funding_address_request_failed {
                ui.label("Could not prepare a deposit address.");
                if ui.button("Try again").clicked() {
                    self.funding_address_request_failed = false;
                }
            } else {
                self.queue_funding_address_request();
                ui.label("Generating a deposit address…");
            }
            return;
        };

        // The QR URI encodes the amount at 4 decimals; show that same rounded-up
        // figure in the hint so the two never disagree or understate the minimum.
        let minimum_credits = self.deposit_minimum_credits();
        let minimum_dash = round_up_dash_4dp(Amount::dash_from_credits(minimum_credits).to_f64());
        let minimum_amount = format!("{minimum_dash:.4} DASH");
        let dash_uri = format!("dash:{address}?amount={minimum_dash:.4}");

        if let Ok(qr_image) = generate_qr_code_image(&dash_uri) {
            let texture =
                ui.ctx()
                    .load_texture("deposit_qr_code", qr_image, egui::TextureOptions::LINEAR);
            ui.image((texture.id(), Vec2::new(200.0, 200.0)));
        } else {
            ui.label("Could not create the deposit QR code. Copy the address below instead.");
        }

        ui.add_space(10.0);
        ui.label(format!(
            "Send at least {minimum_amount} to this address to fund your identity."
        ));
        ui.add_space(5.0);

        let address_text = address.to_string();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&address_text).font(egui::FontId::monospace(12.0)));
            if ui.button("Copy address").clicked() {
                ui.ctx().copy_text(address_text.clone());
                MessageBanner::set_global(
                    ui.ctx(),
                    "Address copied to clipboard.",
                    MessageType::Success,
                );
            }
        });

        ui.add_space(8.0);
        let spendable_duffs = self.selected_wallet_spendable_duffs();
        if spendable_duffs > 0 {
            ui.label(format!(
                "Received {received} so far. Waiting for at least {minimum_amount}.",
                received = Amount::dash_from_duffs(spendable_duffs),
            ));
        } else {
            ui.label(
                "Waiting for your deposit to arrive. You can leave this open — your funds are \
                 safe once they reach this address.",
            );
        }
    }

    /// Render the "Receive a new deposit" funding method: a scannable deposit
    /// address while waiting, then an editable amount and Create button once the
    /// deposit arrives. A "Choose a different funding method" affordance is
    /// present throughout so the user is never trapped.
    pub fn render_ui_by_receive_deposit(&mut self, ui: &mut Ui, step_number: u32) -> AppAction {
        let mut action = AppAction::None;
        let step = *self.step.read_recover();

        if step == WalletFundedScreenStep::WaitingOnFunds {
            ui.heading(format!(
                "{step_number}. Send a deposit to fund your identity."
            ));
            ui.add_space(10.0);
            self.render_deposit_qr(ui);
            ui.add_space(10.0);
            if ui.button("Choose a different funding method").clicked() {
                self.reset_to_choose_funding();
            }
            // Poll for the incoming deposit; there is no timeout and no error state.
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            return action;
        }

        ui.heading(format!(
            "{step_number}. Deposit received. Choose how much to use, then continue."
        ));
        ui.add_space(10.0);
        self.render_funding_amount_input(ui);

        let has_valid_amount = self
            .funding_amount
            .as_ref()
            .map(|a| a.value() > 0)
            .unwrap_or(false);

        if step == WalletFundedScreenStep::FundsReceived {
            if has_valid_amount {
                self.render_alias_input(ui, step_number + 1);
                let button =
                    egui::Button::new(RichText::new("Create Identity").color(Color32::WHITE))
                        .fill(DashColors::DASH_BLUE)
                        .frame(true)
                        .corner_radius(3.0);
                if ui.add(button).clicked() {
                    action = self.register_identity_clicked(FundingMethod::ReceiveDeposit);
                }
                ui.add_space(10.0);
            }
            if ui.button("Choose a different funding method").clicked() {
                self.reset_to_choose_funding();
            }
        }

        ui.add_space(20.0);
        ui.vertical_centered(|ui| match step {
            WalletFundedScreenStep::WaitingForAssetLock => {
                ui.heading("=> Waiting for Core Chain to produce proof of transfer of funds. <=");
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                ui.heading("=> Waiting for Platform acknowledgement <=");
            }
            _ => {}
        });
        ui.add_space(20.0);

        action
    }
}
