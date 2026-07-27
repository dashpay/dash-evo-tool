use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::RootScreenType;
use crate::ui::identities::funding_common::{FundingMethod, spendable_covers_minimum};
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use crate::ui::theme::DashColors;
use egui::{Color32, Frame, Margin, RichText, Ui};

impl TopUpIdentityScreen {
    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.wallet {
            let wallet = match selected_wallet.read() {
                Ok(w) => w,
                Err(_) => {
                    ui.label("Wallet is busy. Try again in a moment.");
                    return;
                }
            };

            let spendable_balance: u64 = self
                .app_context
                .snapshot_balance(&wallet.seed_hash())
                .spendable();

            let dash_balance = spendable_balance as f64 * 1e-8; // Convert to DASH units

            ui.horizontal(|ui| {
                ui.label(format!("Wallet Balance: {:.8} DASH", dash_balance));
            });
        } else {
            ui.label("No wallet selected");
        }
    }

    /// If the selected wallet can't cover even the estimated top-up fee,
    /// render an equivalent of the Create-Identity "not enough Dash" banner
    /// with a link to the Wallets screen, and report that the caller should
    /// stop rendering this step. Returns `None` when the balance is
    /// sufficient, or when no wallet is selected (handled earlier by the
    /// caller's own no-wallet gate).
    fn render_insufficient_wallet_balance_banner(&self, ui: &mut egui::Ui) -> Option<AppAction> {
        let selected_wallet = self.wallet.as_ref()?;
        let spendable_duffs = match selected_wallet.read() {
            Ok(w) => self.asset_lock_balance.get(&w.seed_hash())?,
            Err(_) => {
                ui.label("Wallet is busy. Try again in a moment.");
                return Some(AppAction::None);
            }
        };

        let minimum_credits = self.app_context.fee_estimator().estimate_identity_topup();

        if spendable_covers_minimum(spendable_duffs, minimum_credits) {
            return None;
        }

        ui.add_space(8.0);
        ui.colored_label(
            DashColors::WARNING,
            format!(
                "Your wallet does not have enough Dash to top up this identity yet. \
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

        ui.heading(format!(
            "{}. How much of your wallet balance would you like to transfer?",
            step_number
        ));

        ui.add_space(10.0);
        self.show_wallet_balance(ui);
        ui.add_space(5.0);

        let seed_hash = self
            .wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()));
        let Some(seed_hash) = seed_hash else {
            return action;
        };
        let snapshot_generation = self.app_context.snapshot_generation(&seed_hash);
        if let Some(task) = self
            .asset_lock_balance
            .ensure_requested(seed_hash, snapshot_generation)
        {
            action |= AppAction::BackendTask(task);
        }
        if self.asset_lock_balance.is_failed(&seed_hash) {
            ui.label("The available amount could not be checked.");
            if ui.button("Retry available amount check").clicked() {
                self.asset_lock_balance.invalidate_one(&seed_hash);
            }
            return action;
        }
        if self.asset_lock_balance.get(&seed_hash).is_none() {
            ui.label("Checking the available amount…");
            return action;
        }

        if let Some(insufficient_action) = self.render_insufficient_wallet_balance_banner(ui) {
            action |= insufficient_action;
            return action;
        }

        self.top_up_funding_amount_input(ui);

        // Extract the step from the RwLock to minimize borrow scope
        let step = self.current_step();

        // Only show the fee estimate and Top Up button once a positive amount
        // is entered — otherwise clicking Top Up would silently no-op.
        let has_valid_amount = self.funding_amount_exact.is_some_and(|d| d > 0);
        if !has_valid_amount {
            return action;
        }

        // Fee estimation display
        let fee_estimator = self.app_context.fee_estimator();
        let estimated_fee = fee_estimator.estimate_identity_topup();

        let dark_mode = ui.style().visuals.dark_mode;
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Estimated fee:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.label(
                        RichText::new(format_credits_as_dash(estimated_fee))
                            .color(DashColors::text_primary(dark_mode))
                            .size(14.0),
                    );
                });
            });

        ui.add_space(10.0);

        // Top up button
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);
        let button = egui::Button::new(RichText::new("Top Up Identity").color(Color32::WHITE))
            .fill(DashColors::DASH_BLUE)
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            action = self.top_up_identity_clicked(FundingMethod::UseWalletBalance);
        }

        ui.add_space(20.0);

        ui.vertical_centered(|ui| {
            match step {
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
            };
        });

        ui.add_space(40.0);
        action
    }
}
