use crate::app::AppAction;
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::Address;
use eframe::egui::{self, Context};
use egui::{Color32, RichText};
use std::str::FromStr;
use std::sync::Arc;

#[derive(PartialEq)]
enum Status {
    NotStarted,
    WaitingForResult,
    Complete,
}

/// Which kind of destination was parsed from the address input.
enum Destination {
    /// Shielded pool → platform address (Type 17 Unshield)
    Platform(PlatformAddress),
    /// Shielded pool → core L1 address (Type 19 ShieldedWithdrawal)
    Core(Address),
}

pub struct UnshieldCreditsScreen {
    pub app_context: Arc<AppContext>,
    pub seed_hash: WalletSeedHash,
    amount_str: String,
    address_str: String,
    max_balance: u64,
    status: Status,
    error_message: Option<String>,
    success_message: Option<String>,
}

impl UnshieldCreditsScreen {
    pub fn new(seed_hash: WalletSeedHash, app_context: &Arc<AppContext>) -> Self {
        let max_balance = {
            let states = app_context.shielded_states.lock().unwrap();
            states
                .get(&seed_hash)
                .map(|s| s.shielded_balance)
                .unwrap_or(0)
        };

        Self {
            app_context: app_context.clone(),
            seed_hash,
            amount_str: String::new(),
            address_str: String::new(),
            max_balance,
            status: Status::NotStarted,
            error_message: None,
            success_message: None,
        }
    }

    fn parse_amount_credits(&self) -> Option<u64> {
        let trimmed = self.amount_str.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains('.') {
            let dash: f64 = trimmed.parse().ok()?;
            if dash <= 0.0 {
                return None;
            }
            Some((dash * CREDITS_PER_DUFF as f64 * 1e8) as u64)
        } else {
            let credits: u64 = trimmed.parse().ok()?;
            if credits == 0 {
                return None;
            }
            Some(credits)
        }
    }

    /// Parse the address field into a Destination.
    ///
    /// Tries platform address (Bech32m tdash1.../dash1...) first, then falls
    /// back to a core address (Base58 P2PKH/P2SH).
    fn parse_destination(&self) -> Option<Destination> {
        let s = self.address_str.trim();
        if s.is_empty() {
            return None;
        }

        // Try platform address first
        if let Ok((pa, _network)) = PlatformAddress::from_bech32m_string(s) {
            return Some(Destination::Platform(pa));
        }

        // Try core address
        if let Ok(addr) = Address::from_str(s) {
            let addr = addr.require_network(self.app_context.network).ok()?;
            return Some(Destination::Core(addr));
        }

        None
    }
}

impl ScreenLike for UnshieldCreditsScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::PopScreen),
                ("Unshield Credits", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        island_central_panel(ctx, |ui| {
            ui.heading("Unshield Credits");
            ui.add_space(10.0);
            ui.label(
                "Move credits from the shielded pool to a platform address or a core DASH address.",
            );
            ui.add_space(5.0);

            let dash_balance = self.max_balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
            ui.label(format!(
                "Available shielded balance: {:.8} DASH",
                dash_balance
            ));
            ui.add_space(15.0);

            // Error/success messages
            if let Some(err) = &self.error_message {
                ui.colored_label(Color32::from_rgb(255, 100, 100), err);
                ui.add_space(5.0);
            }
            if let Some(msg) = &self.success_message {
                ui.colored_label(Color32::DARK_GREEN, msg);
                ui.add_space(10.0);
                if ui.button("Done").clicked() {
                    action = AppAction::PopScreen;
                }
                return;
            }

            // Destination address input
            ui.horizontal(|ui| {
                ui.label("To address:");
                ui.text_edit_singleline(&mut self.address_str);
            });

            // Show what was parsed
            match self.parse_destination() {
                Some(Destination::Platform(_)) => {
                    ui.colored_label(
                        Color32::DARK_GREEN,
                        "Platform address — will unshield to platform (Type 17)",
                    );
                }
                Some(Destination::Core(_)) => {
                    ui.colored_label(
                        Color32::DARK_GREEN,
                        "Core address — will withdraw to core DASH (Type 19)",
                    );
                }
                None if !self.address_str.trim().is_empty() => {
                    ui.colored_label(
                        Color32::from_rgb(255, 100, 100),
                        "Unrecognised address — enter a platform address (tdash1…/dash1…) or a core DASH address",
                    );
                }
                None => {}
            }
            ui.add_space(10.0);

            // Amount input
            ui.horizontal(|ui| {
                ui.label("Amount (DASH):");
                ui.text_edit_singleline(&mut self.amount_str);
            });
            if let Some(credits) = self.parse_amount_credits() {
                let dash = credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                ui.label(format!("= {:.8} DASH ({} credits)", dash, credits));
                if credits > self.max_balance {
                    ui.colored_label(Color32::from_rgb(255, 100, 100), "Exceeds shielded balance");
                }
            }
            ui.add_space(15.0);

            let amount_ok = self
                .parse_amount_credits()
                .is_some_and(|a| a <= self.max_balance);
            let destination = self.parse_destination();
            let can_confirm =
                self.status == Status::NotStarted && amount_ok && destination.is_some();

            if self.status == Status::WaitingForResult {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Processing...");
                });
            } else {
                ui.horizontal(|ui| {
                    let btn_label = match &destination {
                        Some(Destination::Core(_)) => "Withdraw to Core",
                        _ => "Unshield",
                    };

                    if ui
                        .add_enabled(
                            can_confirm,
                            egui::Button::new(
                                RichText::new(btn_label).color(Color32::WHITE).size(16.0),
                            )
                            .fill(crate::ui::theme::DashColors::DASH_BLUE),
                        )
                        .clicked()
                        && let Some(amount) = self.parse_amount_credits()
                    {
                        match self.parse_destination() {
                            Some(Destination::Platform(addr)) => {
                                self.status = Status::WaitingForResult;
                                self.error_message = None;
                                action = AppAction::BackendTask(BackendTask::ShieldedTask(
                                    ShieldedTask::UnshieldCredits {
                                        seed_hash: self.seed_hash,
                                        amount,
                                        to_platform_address: addr,
                                    },
                                ));
                            }
                            Some(Destination::Core(addr)) => {
                                self.status = Status::WaitingForResult;
                                self.error_message = None;
                                action = AppAction::BackendTask(BackendTask::ShieldedTask(
                                    ShieldedTask::ShieldedWithdrawal {
                                        seed_hash: self.seed_hash,
                                        amount,
                                        to_core_address: addr,
                                    },
                                ));
                            }
                            None => {}
                        }
                    }

                    ui.add_space(10.0);
                    if ui.button("Cancel").clicked() {
                        action = AppAction::PopScreen;
                    }
                });
            }
        });

        action
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::ShieldedCreditsUnshielded { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                self.success_message = Some(format!(
                    "Successfully unshielded {:.8} DASH to platform address",
                    dash
                ));
            }
            BackendTaskSuccessResult::ShieldedWithdrawalComplete { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                self.success_message = Some(format!(
                    "Successfully withdrew {:.8} DASH to core address",
                    dash
                ));
            }
            _ => {}
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Error => {
                self.status = Status::NotStarted;
                self.error_message = Some(message.to_string());
            }
            _ => {
                self.success_message = Some(message.to_string());
            }
        }
    }
}
