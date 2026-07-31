use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::RootScreenType;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, FundingMethod, WalletFundedScreenStep,
};
use crate::ui::identities::funding_common::spendable_covers_minimum;
use crate::ui::theme::DashColors;
use egui::{Color32, RichText, Ui};

impl AddNewIdentityScreen {
    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            let wallet = match selected_wallet.read() {
                Ok(w) => w,
                Err(_) => {
                    ui.label("Wallet is busy. Try again in a moment.");
                    return;
                }
            };

            let seed_hash = wallet.seed_hash();
            let spendable_balance = self
                .asset_lock_balance
                .get(&seed_hash)
                .unwrap_or_else(|| self.app_context.snapshot_balance(&seed_hash).spendable());

            let dash_balance = spendable_balance as f64 * 1e-8; // Convert to DASH units

            ui.horizontal(|ui| {
                ui.label(format!("Wallet Balance: {:.8} DASH", dash_balance));
            });
        } else {
            ui.label("No wallet selected");
        }
    }

    /// If the selected wallet can't cover even the estimated identity-creation
    /// fee, render design-spec §B.1's "not enough Dash" banner with a link to
    /// the Wallets screen (design-spec calls it "Go to Receive"; this app has
    /// no separate top-level Receive screen, so the link goes to Wallets,
    /// where the user's receiving address lives) and report that the caller
    /// should stop rendering this step. Returns `None` when the balance is
    /// sufficient, or when no wallet is selected (handled earlier by the
    /// caller's own no-wallet gate).
    fn render_insufficient_wallet_balance_banner(&self, ui: &mut egui::Ui) -> Option<AppAction> {
        let selected_wallet = self.selected_wallet.as_ref()?;
        let spendable_duffs = match selected_wallet.read() {
            Ok(w) => self.asset_lock_balance.get(&w.seed_hash())?,
            Err(_) => {
                ui.label("Wallet is busy. Try again in a moment.");
                return Some(AppAction::None);
            }
        };

        let key_count = self.identity_keys.others.len() + 1; // +1 for master key
        let minimum_credits = self
            .app_context
            .fee_estimator()
            .estimate_identity_create(key_count);

        if spendable_covers_minimum(spendable_duffs, minimum_credits) {
            return None;
        }

        ui.add_space(8.0);
        ui.colored_label(
            DashColors::WARNING,
            format!(
                "Your wallet does not have enough Dash to create an identity yet. \
                 Add at least {amount} to continue.",
                amount = format_credits_as_dash(minimum_credits)
            ),
        );
        ui.add_space(8.0);
        let mut action = AppAction::None;
        if ui.button("Go to Wallets").clicked() {
            action = AppAction::SetMainScreenThenGoToMainScreen(
                RootScreenType::RootScreenWalletsBalances,
            );
        }
        ui.add_space(10.0);
        Some(action)
    }

    pub fn render_ui_by_using_unused_balance(
        &mut self,
        ui: &mut Ui,
        step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;

        ui.add_space(10.0);
        ui.heading(format!(
            "{}. How much of your wallet balance would you like to transfer?",
            step_number
        ));

        ui.add_space(10.0);
        self.show_wallet_balance(ui);
        ui.add_space(5.0);

        let seed_hash = self
            .selected_wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()));
        let Some(seed_hash) = seed_hash else {
            return action;
        };
        let failed = self.asset_lock_balance.is_failed(&seed_hash);
        let loading = self.asset_lock_quote_is_loading(&seed_hash);
        if failed || loading {
            ui.label(if failed {
                "The available amount could not be checked."
            } else {
                "Checking the available amount…"
            });
            if self.asset_lock_balance.should_offer_retry(&seed_hash)
                && ui.button("Retry available amount check").clicked()
            {
                self.asset_lock_balance.invalidate_one(&seed_hash);
            }
            return action;
        }
        if self.asset_lock_balance.should_offer_retry(&seed_hash) {
            ui.label("The amount shown is safe but may be lower than your full available amount.");
            if ui.button("Retry available amount check").clicked() {
                self.asset_lock_balance.invalidate_one(&seed_hash);
            }
        }

        if let Some(insufficient_action) = self.render_insufficient_wallet_balance_banner(ui) {
            action |= insufficient_action;
            return action;
        }

        self.render_funding_amount_input(ui);

        // Extract the step from the RwLock to minimize borrow scope
        let step = self
            .step
            .read()
            .map(|s| *s)
            .unwrap_or(WalletFundedScreenStep::ChooseFundingMethod);

        // Check if we have a valid amount before showing the button
        let has_valid_amount = self
            .funding_amount
            .as_ref()
            .map(|a| a.value() > 0)
            .unwrap_or(false);

        if !has_valid_amount {
            return action;
        }

        // Display estimated fee before action button
        let key_count = self.identity_keys.others.len() + 1; // +1 for master key
        let estimated_fee = self
            .app_context
            .fee_estimator()
            .estimate_identity_create(key_count);
        ui.add_space(10.0);
        let dark_mode = ui.style().visuals.dark_mode;
        egui::Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Estimated Fee:")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format_credits_as_dash(estimated_fee))
                            .color(DashColors::text_primary(dark_mode))
                            .strong(),
                    );
                });
            });
        ui.add_space(10.0);

        self.render_alias_input(ui, step_number + 1);

        let button = egui::Button::new(RichText::new("Create Identity").color(Color32::WHITE))
            .fill(DashColors::DASH_BLUE)
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            action = self.register_identity_clicked(FundingMethod::UseWalletBalance);
        }

        ui.add_space(20.0);

        {
            ui.vertical_centered(|ui| match step {
                WalletFundedScreenStep::WaitingForAssetLock => {
                    ui.heading(
                        "=> Waiting for Core Chain to produce proof of transfer of funds. <=",
                    );
                }
                WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                    ui.heading("=> Waiting for Platform acknowledgement <=");
                }
                WalletFundedScreenStep::Success => {
                    ui.heading("...Success...");
                }
                _ => {}
            });
        }

        ui.add_space(40.0);
        action
    }
}
