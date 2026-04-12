use crate::app::AppAction;
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::wallet::WalletId;
use crate::ui::components::ComponentResponse;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
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
    pub wallet_id: WalletId,
    amount_input: Option<AmountInput>,
    amount: Option<Amount>,
    recipient_address_input: String,
    max_balance: u64,
    status: Status,
    error_message: Option<String>,
    success_message: Option<String>,
    /// Queued task to dispatch on next frame (e.g., sync notes after successful send).
    pending_refresh_task: Option<BackendTask>,
    /// Whether to show the balance-update-pending info banner on the success screen.
    balance_update_pending: bool,
}

impl ShieldedSendScreen {
    pub fn new(wallet_id: WalletId, app_context: &Arc<AppContext>) -> Self {
        let max_balance = app_context
            .shielded_states
            .lock()
            .ok()
            .and_then(|states| states.get(&wallet_id).map(|s| s.shielded_balance))
            .unwrap_or(0);

        Self {
            app_context: app_context.clone(),
            wallet_id,
            amount_input: None,
            amount: None,
            recipient_address_input: String::new(),
            max_balance,
            status: Status::NotStarted,
            error_message: None,
            success_message: None,
            pending_refresh_task: None,
            balance_update_pending: false,
        }
    }

    pub(crate) fn invalidate_address_input(&mut self) {
        self.recipient_address_input.clear();
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
    fn refresh_on_arrival(&mut self) {
        if let Ok(states) = self.app_context.shielded_states.lock()
            && let Some(state) = states.get(&self.wallet_id)
        {
            self.max_balance = state.shielded_balance;
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = self
            .pending_refresh_task
            .take()
            .map(AppAction::BackendTask)
            .unwrap_or(AppAction::None);

        action |= add_top_panel(
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
                if self.balance_update_pending {
                    ui.add_space(8.0);
                    ui.label(
                        "Your remaining balance will update after the next block is confirmed. \
                         The recipient's balance will also update after the next block and a wallet sync.",
                    );
                }
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
            let amount_input = self.amount_input.get_or_insert_with(|| {
                AmountInput::new(Amount::new_dash(0.0))
                    .with_label("Amount (DASH):")
                    .with_hint_text("Enter amount")
                    .with_max_button(true)
                    .with_desired_width(150.0)
            });
            amount_input.set_max_amount(Some(self.max_balance));
            let response = amount_input.show(ui);
            response.inner.update(&mut self.amount);
            ui.add_space(15.0);

            // Confirm
            let amount_ok = self.amount.is_some();
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
                        && let (Some(amount), Some(recipient_bytes)) = (
                            self.amount.as_ref().map(|a| a.value()),
                            self.validate_recipient(),
                        )
                    {
                        self.status = Status::WaitingForResult;
                        self.error_message = None;
                        action = AppAction::BackendTask(BackendTask::ShieldedTask(
                            ShieldedTask::ShieldedTransfer {
                                wallet_id: self.wallet_id,
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
            BackendTaskSuccessResult::ShieldedTransferComplete { wallet_id, amount }
                if wallet_id == self.wallet_id =>
            {
                tracing::info!(
                    "ShieldedSendScreen: transfer complete, amount={} credits, queueing post-transfer note sync",
                    amount,
                );
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                self.success_message =
                    Some(format!("Successfully sent {:.8} DASH privately", dash));
                self.pending_refresh_task =
                    Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                        wallet_id: self.wallet_id,
                    }));
                self.balance_update_pending = true;
            }
            BackendTaskSuccessResult::ShieldedNotesSynced {
                wallet_id,
                new_notes,
                balance,
            } if wallet_id == self.wallet_id => {
                tracing::debug!(
                    "ShieldedSendScreen: post-transfer sync complete, new_notes={}, balance={} credits",
                    new_notes,
                    balance,
                );
                self.max_balance = balance;
                self.balance_update_pending = false;
                let dash = balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                if let Some(msg) = self.success_message.as_mut() {
                    *msg = format!("{}\nBalance updated: {:.8} DASH remaining.", msg, dash,);
                }
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
