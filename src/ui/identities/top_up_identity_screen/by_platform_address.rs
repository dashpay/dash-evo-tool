use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::model::amount::Amount;
use crate::model::wallet::WalletSeedHash;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::Address;
use egui::{Frame, Margin, RichText, Ui};
use std::collections::BTreeMap;

use super::TopUpIdentityScreen;

impl TopUpIdentityScreen {
    /// Render the UI for topping up identity from Platform addresses
    pub(super) fn render_ui_by_platform_address(
        &mut self,
        ui: &mut Ui,
        step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.heading(format!(
            "{}. Select a Platform address to use for top-up.",
            step_number
        ));
        ui.add_space(10.0);

        // Get Platform addresses from the wallet
        let platform_addresses = self.get_platform_addresses_with_balance();

        if platform_addresses.is_empty() {
            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("No Platform addresses with balance found.")
                            .color(DashColors::text_secondary(dark_mode))
                            .italics(),
                    );
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Fund a Platform address first to use it for top-up.")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                });
            return action;
        }

        // Show list of Platform addresses (using DIP-18 Bech32m format)
        let network = self.app_context.network;
        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                for (core_addr, platform_addr, balance) in &platform_addresses {
                    let is_selected = self
                        .selected_platform_address
                        .as_ref()
                        .map(|(_, p, _)| p == platform_addr)
                        .unwrap_or(false);

                    // Display address in Bech32m format
                    let addr_display = platform_addr.to_bech32m_string(network);
                    let response = ui.selectable_label(
                        is_selected,
                        format!("{} - {}", addr_display, Self::format_credits(*balance)),
                    );

                    if response.clicked() {
                        self.selected_platform_address =
                            Some((core_addr.clone(), *platform_addr, *balance));
                    }
                }
            });

        ui.add_space(15.0);

        // Amount input
        ui.heading(format!("{}. Enter the amount to top up.", step_number + 1));
        ui.add_space(10.0);

        // Get max balance for the selected platform address
        let max_balance_credits = self
            .selected_platform_address
            .as_ref()
            .map(|(_, _, balance)| *balance);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Amount input using AmountInput component
                    let amount_input = self.platform_top_up_amount_input.get_or_insert_with(|| {
                        AmountInput::new(Amount::new_dash(0.0))
                            .with_label("Amount (DASH):")
                            .with_hint_text("Enter amount (e.g., 0.01)")
                            .with_max_button(true)
                            .with_desired_width(150.0)
                    });

                    // Update max amount dynamically based on selected platform address
                    amount_input.set_max_amount(max_balance_credits);

                    let response = amount_input.show(ui);
                    response.inner.update(&mut self.platform_top_up_amount);

                    if let Some((_, _, balance)) = &self.selected_platform_address {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("Available: {}", Self::format_credits(*balance)))
                                .color(DashColors::text_secondary(dark_mode))
                                .size(12.0),
                        );
                    }
                });
            });

        ui.add_space(20.0);

        // Top Up button
        let has_valid_amount = self
            .platform_top_up_amount
            .as_ref()
            .map(|a| a.value() > 0)
            .unwrap_or(false);
        let can_top_up =
            self.selected_platform_address.is_some() && has_valid_amount && self.wallet.is_some();

        let step = { *self.step.read().unwrap() };

        ui.horizontal(|ui| {
            let button_text = match step {
                WalletFundedScreenStep::WaitingForPlatformAcceptance => "Topping Up...",
                _ => "Top Up Identity",
            };

            let button = egui::Button::new(
                RichText::new(button_text)
                    .color(egui::Color32::WHITE)
                    .strong(),
            )
            .fill(if can_top_up {
                DashColors::DASH_BLUE
            } else {
                DashColors::DASH_BLUE.gamma_multiply(0.5)
            })
            .min_size(egui::vec2(120.0, 36.0));

            if ui.add_enabled(can_top_up, button).clicked() {
                match self.validate_and_top_up_from_platform() {
                    Ok(top_up_action) => {
                        action = top_up_action;
                    }
                    Err(e) => {
                        self.error_message = Some(e);
                    }
                }
            }
        });

        action
    }

    /// Get Platform addresses with balance from the selected wallet
    fn get_platform_addresses_with_balance(&self) -> Vec<(Address, PlatformAddress, Credits)> {
        let Some(wallet_arc) = &self.wallet else {
            return vec![];
        };
        let Ok(wallet) = wallet_arc.read() else {
            return vec![];
        };

        let network = self.app_context.network;
        wallet
            .platform_addresses(network)
            .into_iter()
            .map(|(core_addr, platform_addr)| {
                let balance = wallet
                    .platform_address_info
                    .get(&core_addr)
                    .map(|info| info.balance)
                    .unwrap_or(0);
                (core_addr, platform_addr, balance)
            })
            .filter(|(_, _, balance)| *balance > 0)
            .collect()
    }

    /// Format credits as DASH equivalent
    fn format_credits(credits: Credits) -> String {
        let dash_equivalent = credits as f64 / 1000.0 / 100_000_000.0;
        format!("{:.8} DASH", dash_equivalent)
    }

    /// Validate and create the top-up task
    fn validate_and_top_up_from_platform(&mut self) -> Result<AppAction, String> {
        let (_, platform_addr, available_balance) = self
            .selected_platform_address
            .clone()
            .ok_or_else(|| "Please select a Platform address".to_string())?;

        let amount = self
            .platform_top_up_amount
            .as_ref()
            .map(|a| a.value())
            .ok_or_else(|| "Amount is required".to_string())?;

        if amount == 0 {
            return Err("Amount must be positive".to_string());
        }

        if amount > available_balance {
            return Err(format!(
                "Insufficient balance. Available: {}, Requested: {}",
                Self::format_credits(available_balance),
                Self::format_credits(amount)
            ));
        }

        // Get wallet seed hash
        let wallet_seed_hash: WalletSeedHash = {
            let wallet = self
                .wallet
                .as_ref()
                .ok_or_else(|| "No wallet selected".to_string())?;
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            wallet_guard.seed_hash()
        };

        // Build inputs
        let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        inputs.insert(platform_addr, amount);

        // Update step
        {
            let mut step = self.step.write().unwrap();
            *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
        }

        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TopUpIdentityFromPlatformAddresses {
                identity: self.identity.clone(),
                inputs,
                wallet_seed_hash,
            },
        )))
    }
}
