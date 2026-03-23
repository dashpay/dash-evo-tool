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

pub struct ShieldedSendScreen {
    pub app_context: Arc<AppContext>,
    pub seed_hash: WalletSeedHash,
    amount_str: String,
    recipient_address_input: String,
    max_balance: u64,
    status: Status,
    error_message: Option<String>,
    success_message: Option<String>,
}

impl ShieldedSendScreen {
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
            recipient_address_input: String::new(),
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

    fn validate_recipient(&self) -> Option<Vec<u8>> {
        let trimmed = self.recipient_address_input.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Try bech32m first (dash1z... or tdash1z...)
        if let Ok((addr, _network)) =
            dash_sdk::dpp::address_funds::OrchardAddress::from_bech32m_string(trimmed)
        {
            return Some(addr.to_raw_bytes().to_vec());
        }
        // Fall back to raw hex (43 bytes = 86 hex chars)
        let bytes = hex::decode(trimmed).ok()?;
        if bytes.len() != 43 {
            return None;
        }
        Some(bytes)
    }
}

impl ScreenLike for ShieldedSendScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::PopScreen),
                ("Send (Private)", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        island_central_panel(ctx, |ui| {
            ui.heading("Send (Private)");
            ui.add_space(10.0);
            ui.label("Transfer credits privately within the shielded pool.");
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

            // Recipient address input
            ui.label("Recipient shielded address (dash1z.../tdash1z... or hex):");
            ui.add_space(2.0);
            ui.text_edit_singleline(&mut self.recipient_address_input);
            if !self.recipient_address_input.trim().is_empty()
                && self.validate_recipient().is_none()
            {
                ui.colored_label(Color32::from_rgb(255, 100, 100), "Invalid shielded address");
            }
            ui.add_space(10.0);

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
            let recipient_ok = self.validate_recipient().is_some();
            let can_confirm = self.status == Status::NotStarted && amount_ok && recipient_ok;

            if self.status == Status::WaitingForResult {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Sending privately...");
                });
            } else {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            can_confirm,
                            egui::Button::new(
                                RichText::new("Send").color(Color32::WHITE).size(16.0),
                            )
                            .fill(crate::ui::theme::DashColors::DASH_BLUE),
                        )
                        .clicked()
                        && let (Some(amount), Some(recipient_bytes)) =
                            (self.parse_amount_credits(), self.validate_recipient())
                    {
                        self.status = Status::WaitingForResult;
                        self.error_message = None;
                        action = AppAction::BackendTask(BackendTask::ShieldedTask(
                            ShieldedTask::ShieldedTransfer {
                                seed_hash: self.seed_hash,
                                amount,
                                recipient_address_bytes: recipient_bytes,
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
            BackendTaskSuccessResult::ShieldedTransferComplete { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                self.success_message =
                    Some(format!("Successfully sent {:.8} DASH privately", dash));
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
