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
use eframe::egui::{self, Context};
use egui::{Color32, RichText};
use std::sync::Arc;

#[derive(PartialEq)]
enum Status {
    NotStarted,
    WaitingForResult,
    Complete,
}

pub struct UnshieldCreditsScreen {
    pub app_context: Arc<AppContext>,
    pub seed_hash: WalletSeedHash,
    amount_str: String,
    to_platform_address: Option<PlatformAddress>,
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

        // Try to find the first platform address from the wallet
        let to_platform_address = {
            let wallets = app_context.wallets.read().unwrap();
            wallets.get(&seed_hash).and_then(|w| {
                let wallet = w.read().unwrap();
                wallet
                    .platform_address_info
                    .keys()
                    .next()
                    .and_then(|addr| PlatformAddress::try_from(addr.clone()).ok())
            })
        };

        Self {
            app_context: app_context.clone(),
            seed_hash,
            amount_str: String::new(),
            to_platform_address,
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
            ui.label("Move credits from the shielded pool back to a platform address.");
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

            // Destination address display
            if let Some(addr) = &self.to_platform_address {
                ui.horizontal(|ui| {
                    ui.label("To platform address:");
                    ui.monospace(format!("{}", addr));
                });
                ui.add_space(10.0);
            } else {
                ui.colored_label(
                    Color32::from_rgb(255, 100, 100),
                    "No platform address found. Register an identity first.",
                );
                return;
            }

            // Amount input
            ui.horizontal(|ui| {
                ui.label("Amount (DASH or credits):");
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

            // Confirm
            let amount_ok = self
                .parse_amount_credits()
                .is_some_and(|a| a <= self.max_balance);
            let can_confirm = self.status == Status::NotStarted
                && amount_ok
                && self.to_platform_address.is_some();

            if self.status == Status::WaitingForResult {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Unshielding credits...");
                });
            } else {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            can_confirm,
                            egui::Button::new(
                                RichText::new("Unshield").color(Color32::WHITE).size(16.0),
                            )
                            .fill(crate::ui::theme::DashColors::DASH_BLUE),
                        )
                        .clicked()
                        && let (Some(amount), Some(addr)) =
                            (self.parse_amount_credits(), self.to_platform_address)
                    {
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
                self.success_message = Some(format!("Successfully unshielded {:.8} DASH", dash));
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
