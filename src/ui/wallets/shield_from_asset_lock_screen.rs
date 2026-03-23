use crate::app::AppAction;
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
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

pub struct ShieldFromAssetLockScreen {
    pub app_context: Arc<AppContext>,
    pub seed_hash: WalletSeedHash,
    amount_str: String,
    core_balance_duffs: u64,
    status: Status,
    error_message: Option<String>,
    success_message: Option<String>,
}

impl ShieldFromAssetLockScreen {
    pub fn new(seed_hash: WalletSeedHash, app_context: &Arc<AppContext>) -> Self {
        let core_balance_duffs = {
            let wallets = app_context.wallets.read().unwrap();
            wallets
                .get(&seed_hash)
                .map(|w| {
                    let wallet = w.read().unwrap();
                    wallet.total_balance_duffs()
                })
                .unwrap_or(0)
        };

        Self {
            app_context: app_context.clone(),
            seed_hash,
            amount_str: String::new(),
            core_balance_duffs,
            status: Status::NotStarted,
            error_message: None,
            success_message: None,
        }
    }

    /// Parse amount input as DASH (decimal) and return duffs.
    fn parse_amount_duffs(&self) -> Option<u64> {
        let trimmed = self.amount_str.trim();
        if trimmed.is_empty() {
            return None;
        }
        let dash: f64 = trimmed.parse().ok()?;
        if dash <= 0.0 {
            return None;
        }
        let duffs = (dash * 1e8) as u64;
        if duffs == 0 {
            return None;
        }
        Some(duffs)
    }
}

impl ScreenLike for ShieldFromAssetLockScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::PopScreen),
                ("Shield from Core", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        island_central_panel(ctx, |ui| {
            ui.heading("Shield from Core Wallet");
            ui.add_space(10.0);
            ui.label("Send core DASH directly into the shielded pool via an asset lock.");
            ui.add_space(5.0);

            let dash_balance = self.core_balance_duffs as f64 / 1e8;
            ui.label(format!(
                "Available core wallet balance: {:.8} DASH",
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

            // Amount input
            ui.horizontal(|ui| {
                ui.label("Amount (DASH):");
                ui.text_edit_singleline(&mut self.amount_str);
            });
            if let Some(duffs) = self.parse_amount_duffs() {
                let credits = duffs * CREDITS_PER_DUFF;
                let dash = duffs as f64 / 1e8;
                ui.label(format!(
                    "= {:.8} DASH = {} credits on platform",
                    dash, credits
                ));
                if duffs > self.core_balance_duffs {
                    ui.colored_label(
                        Color32::from_rgb(255, 100, 100),
                        "Exceeds core wallet balance",
                    );
                }
            }
            ui.add_space(15.0);

            // Confirm
            let amount_ok = self
                .parse_amount_duffs()
                .is_some_and(|a| a <= self.core_balance_duffs);
            let can_confirm = self.status == Status::NotStarted && amount_ok;

            if self.status == Status::WaitingForResult {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Creating asset lock and shielding... (this may take a few minutes)");
                });
            } else {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            can_confirm,
                            egui::Button::new(
                                RichText::new("Shield from Core")
                                    .color(Color32::WHITE)
                                    .size(16.0),
                            )
                            .fill(crate::ui::theme::DashColors::DASH_BLUE),
                        )
                        .clicked()
                        && let Some(amount_duffs) = self.parse_amount_duffs()
                    {
                        self.status = Status::WaitingForResult;
                        self.error_message = None;
                        action = AppAction::BackendTask(BackendTask::ShieldedTask(
                            ShieldedTask::ShieldFromAssetLock {
                                seed_hash: self.seed_hash,
                                amount_duffs,
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
            BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                self.success_message = Some(format!(
                    "Successfully shielded {:.8} DASH from core wallet",
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
