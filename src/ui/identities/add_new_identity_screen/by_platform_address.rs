use crate::app::AppAction;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, FundingMethod, WalletFundedScreenStep,
};
use dash_sdk::dpp::address_funds::PlatformAddress;
use egui::{Color32, ComboBox, RichText, Ui};

/// Constants for credit/DASH conversion
const CREDITS_PER_DUFF: u64 = 1000;

impl AddNewIdentityScreen {
    fn show_platform_address_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            let wallet = selected_wallet.read().unwrap();

            let total_platform_balance: u64 = wallet
                .platform_address_info
                .values()
                .map(|info| info.balance)
                .sum();

            let dash_balance = total_platform_balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;

            ui.horizontal(|ui| {
                ui.label(format!(
                    "Total Platform Address Balance: {:.8} DASH",
                    dash_balance
                ));
            });
        } else {
            ui.label("No wallet selected");
        }
    }

    pub fn render_ui_by_platform_address(&mut self, ui: &mut Ui, step_number: u32) -> AppAction {
        let mut action = AppAction::None;

        ui.add_space(10.0);
        ui.heading(format!(
            "{}. Select a Platform address to fund your new identity",
            step_number
        ));

        ui.add_space(10.0);
        self.show_platform_address_balance(ui);
        ui.add_space(10.0);

        // Get Platform addresses from the wallet
        let platform_addresses: Vec<(String, PlatformAddress, u64)> =
            if let Some(wallet_arc) = &self.selected_wallet {
                let wallet = wallet_arc.read().unwrap();
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
                        (core_addr.to_string(), platform_addr, balance)
                    })
                    .filter(|(_, _, balance)| *balance > 0)
                    .collect()
            } else {
                vec![]
            };

        if platform_addresses.is_empty() {
            ui.colored_label(
                Color32::GRAY,
                "No Platform addresses with balance found. Fund a Platform address first.",
            );
            return action;
        }

        // Platform address selector
        let selected_addr_display = self
            .selected_platform_address_for_funding
            .as_ref()
            .map(|(addr, _)| {
                let display = match addr {
                    PlatformAddress::P2pkh(hash) => {
                        let hex = hex::encode(hash);
                        format!("{}...{}", &hex[..8], &hex[hex.len() - 8..])
                    }
                    PlatformAddress::P2sh(hash) => {
                        let hex = hex::encode(hash);
                        format!("{}...{}", &hex[..8], &hex[hex.len() - 8..])
                    }
                };
                display
            })
            .unwrap_or_else(|| "Select a Platform address".to_string());

        ComboBox::from_label("Platform Address")
            .selected_text(selected_addr_display)
            .show_ui(ui, |ui| {
                for (core_addr_str, platform_addr, balance) in &platform_addresses {
                    let dash_balance = *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                    let label = format!(
                        "{}... ({:.4} DASH)",
                        &core_addr_str[..12.min(core_addr_str.len())],
                        dash_balance
                    );
                    let is_selected = self
                        .selected_platform_address_for_funding
                        .as_ref()
                        .map(|(addr, _)| addr == platform_addr)
                        .unwrap_or(false);

                    if ui.selectable_label(is_selected, label).clicked() {
                        // Parse the amount to credits
                        let amount_credits = self
                            .platform_funding_amount_input
                            .parse::<f64>()
                            .map(|dash| (dash * 1e8 * CREDITS_PER_DUFF as f64) as u64)
                            .unwrap_or(0);
                        self.selected_platform_address_for_funding =
                            Some((*platform_addr, amount_credits.min(*balance)));
                    }
                }
            });

        ui.add_space(10.0);

        // Amount input
        ui.horizontal(|ui| {
            ui.label("Amount (DASH):");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.platform_funding_amount_input)
                    .hint_text("0.5")
                    .desired_width(100.0),
            );

            // Update selected amount when text changes
            if response.changed() {
                if let Some((platform_addr, _)) = self.selected_platform_address_for_funding.clone()
                {
                    let max_balance = platform_addresses
                        .iter()
                        .find(|(_, addr, _)| *addr == platform_addr)
                        .map(|(_, _, balance)| *balance)
                        .unwrap_or(0);

                    let amount_credits = self
                        .platform_funding_amount_input
                        .parse::<f64>()
                        .map(|dash| (dash * 1e8 * CREDITS_PER_DUFF as f64) as u64)
                        .unwrap_or(0);

                    self.selected_platform_address_for_funding =
                        Some((platform_addr, amount_credits.min(max_balance)));
                }
            }

            // Max button
            if let Some((platform_addr, _)) = &self.selected_platform_address_for_funding {
                if let Some((_, _, balance)) = platform_addresses
                    .iter()
                    .find(|(_, addr, _)| addr == platform_addr)
                {
                    if ui.small_button("Max").clicked() {
                        let max_dash = *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                        self.platform_funding_amount_input = format!("{:.8}", max_dash);
                        self.selected_platform_address_for_funding =
                            Some((*platform_addr, *balance));
                    }
                }
            }
        });

        // Show selected amount info
        if let Some((_, amount)) = &self.selected_platform_address_for_funding {
            let dash_amount = *amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
            ui.label(format!("Will use: {:.8} DASH", dash_amount));
        }

        ui.add_space(20.0);

        // Extract the step from the RwLock to minimize borrow scope
        let step = *self.step.read().unwrap();

        // Create Identity button
        let can_create = self.selected_platform_address_for_funding.is_some()
            && self
                .selected_platform_address_for_funding
                .as_ref()
                .map(|(_, amount)| *amount > 0)
                .unwrap_or(false);

        let button = egui::Button::new(RichText::new("Create Identity").color(Color32::WHITE))
            .fill(if can_create {
                Color32::from_rgb(0, 128, 255)
            } else {
                Color32::from_rgb(100, 100, 100)
            })
            .frame(true)
            .corner_radius(3.0);

        if ui.add_enabled(can_create, button).clicked() {
            self.error_message = None;
            action = self.register_identity_clicked(FundingMethod::UsePlatformAddress);
        }

        if let Some(error_message) = self.error_message.as_ref() {
            ui.colored_label(Color32::DARK_RED, error_message);
            ui.add_space(20.0);
        }

        ui.vertical_centered(|ui| match step {
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                ui.heading("=> Waiting for Platform acknowledgement <=");
            }
            WalletFundedScreenStep::Success => {
                ui.heading("...Success...");
            }
            _ => {}
        });

        ui.add_space(40.0);
        action
    }
}
