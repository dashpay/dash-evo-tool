use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

pub struct SendPaymentScreen {
    pub app_context: Arc<AppContext>,
    pub from_identity: QualifiedIdentity,
    pub to_contact_id: Identifier,
    to_contact_name: Option<String>,
    amount_input: Option<AmountInput>,
    amount: Amount,
    memo: String,
    message: Option<(String, MessageType)>,
    sending: bool,
}

impl SendPaymentScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        from_identity: QualifiedIdentity,
        to_contact_id: Identifier,
    ) -> Self {
        Self {
            app_context: app_context.clone(),
            from_identity,
            to_contact_id,
            to_contact_name: None,
            amount_input: None,
            amount: Amount::new_dash(0.0),
            memo: String::new(),
            message: None,
            sending: false,
        }
    }

    fn load_contact_info(&mut self) {
        // TODO: Load contact info from backend/database
        // Mock data for now
        self.to_contact_name = Some("alice.dash".to_string());
    }

    fn send_payment(&mut self) {
        // TODO: Implement actual payment sending via backend
        self.sending = true;

        // Validate amount
        if self.amount.value() == 0 {
            self.display_message("Please enter an amount", MessageType::Error);
            self.sending = false;
            return;
        }

        // Mock successful send
        self.display_message(
            &format!("Payment of {} sent successfully", self.amount),
            MessageType::Success,
        );

        // Clear form
        self.amount_input = None;
        self.amount = Amount::new_dash(0.0);
        self.memo.clear();
        self.sending = false;
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Send Payment");
            ui.add_space(5.0);
            crate::ui::helpers::info_icon_button(
                ui,
                "Payment Guidelines:\n\n\
                • Payments to contacts use encrypted payment channels\n\
                • Only you and the recipient can see payment details\n\
                • Addresses are never reused for privacy\n\
                • Memos are stored locally and not sent on-chain",
            );
        });

        ui.separator();

        // Show message if any
        if let Some((message, message_type)) = &self.message {
            let color = match message_type {
                MessageType::Success => egui::Color32::DARK_GREEN,
                MessageType::Error => egui::Color32::DARK_RED,
                MessageType::Info => egui::Color32::LIGHT_BLUE,
            };
            ui.colored_label(color, message);
            ui.separator();
        }

        ScrollArea::vertical().show(ui, |ui| {
            ui.group(|ui| {
                // From identity
                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new("From:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new(&self.from_identity.to_string())
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });

                // Balance
                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new("Balance:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    let balance_dash = self.from_identity.identity.balance() as f64 * 1e-11;
                    ui.label(
                        RichText::new(format!("{:.6} Dash", balance_dash))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });

                ui.separator();

                // To contact
                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new("To:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    if let Some(name) = &self.to_contact_name {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.label(RichText::new(name).color(DashColors::text_primary(dark_mode)));
                    } else {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.label(
                            RichText::new(&format!("{}", self.to_contact_id))
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    }
                });

                ui.separator();

                // Amount input
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let balance = self.from_identity.identity.balance();
                let amount_input = self.amount_input.get_or_insert_with(|| {
                    AmountInput::new(&self.amount)
                        .with_hint_text("Enter amount in Dash")
                        .with_max_button(true)
                        .with_max_amount(Some(balance))
                        .with_label("Amount:")
                });
                // Update max amount in case balance changed
                amount_input.set_max_amount(Some(balance));
                let response = amount_input.show(ui);
                if response.inner.has_changed() {
                    if let Some(new_amount) = response.inner.changed_value() {
                        self.amount = new_amount.clone();
                    }
                }

                ui.add_space(10.0);

                // Memo field
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.label(
                    RichText::new("Memo (optional):")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add(
                    TextEdit::multiline(&mut self.memo)
                        .hint_text("Add a note to this payment")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.label(
                    RichText::new(format!("{}/100 characters", self.memo.len()))
                        .small()
                        .color(DashColors::text_secondary(dark_mode)),
                );

                ui.add_space(10.0);

                // Send button
                ui.horizontal(|ui| {
                    if self.sending {
                        ui.spinner();
                        ui.label("Sending payment...");
                    } else {
                        let send_enabled = self.amount.value() > 0;
                        let send_button = egui::Button::new(
                            RichText::new("Send Payment")
                                .color(egui::Color32::WHITE)
                        ).fill(
                            if send_enabled {
                                egui::Color32::from_rgb(0, 141, 228) // Dash blue
                            } else {
                                egui::Color32::GRAY
                            }
                        );
                        
                        if ui.add_enabled(send_enabled, send_button).clicked() {
                            if self.memo.len() > 100 {
                                self.display_message(
                                    "Memo must be 100 characters or less",
                                    MessageType::Error,
                                );
                            } else {
                                self.send_payment();
                            }
                        }

                        if ui.button("Cancel").clicked() {
                            action = AppAction::PopScreen;
                        }
                    }
                });
            });
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }
}

impl ScreenLike for SendPaymentScreen {
    fn refresh(&mut self) {
        self.load_contact_info();
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
    }

    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Send Payment", AppAction::None),
            ],
            vec![],
        );

        // Add left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDashPayPayments,
        );

        action |= island_central_panel(ctx, |ui| self.render(ui));

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }
}

// Payment History Component (used in main DashPay screen)
pub struct PaymentHistory {
    app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    payments: Vec<PaymentRecord>,
    message: Option<(String, MessageType)>,
    loading: bool,
}

#[derive(Debug, Clone)]
pub struct PaymentRecord {
    pub tx_id: String,
    pub contact_name: String,
    pub amount: Credits,
    pub is_incoming: bool,
    pub timestamp: u64,
    pub memo: Option<String>,
}

impl PaymentHistory {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_identity_string: String::new(),
            payments: Vec::new(),
            message: None,
            loading: false,
        }
    }

    pub fn trigger_fetch_payment_history(&mut self) -> AppAction {
        if let Some(identity) = &self.selected_identity {
            self.loading = true;
            self.message = Some(("Loading payment history...".to_string(), MessageType::Info));

            let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory {
                identity: identity.clone(),
            }));

            return AppAction::BackendTask(task);
        }

        AppAction::None
    }

    pub fn refresh(&mut self) {
        // Don't auto-fetch, just clear state
        self.payments.clear();
        self.message = None;
        self.loading = false;
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        // Header
        ui.heading("Payment History");
        ui.separator();

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 165, 0),
                "⚠️ No identities loaded. Please load or create an identity first.",
            );
        } else {
            ui.horizontal(|ui| {
                let response = ui.add(
                    IdentitySelector::new(
                        "payment_history_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .label("Identity:")
                    .width(300.0)
                    .other_option(false), // Disable "Other" option
                );

                if response.changed() {
                    self.refresh();
                }
            });
        }

        ui.separator();

        // Show message if any
        if let Some((message, message_type)) = &self.message {
            let color = match message_type {
                MessageType::Success => egui::Color32::DARK_GREEN,
                MessageType::Error => egui::Color32::DARK_RED,
                MessageType::Info => egui::Color32::LIGHT_BLUE,
            };
            ui.colored_label(color, message);
            ui.separator();
        }

        // No identity selected or no identities available
        if identities.is_empty() {
            return action;
        }

        if self.selected_identity.is_none() {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            ui.label(
                RichText::new("Please select an identity to view payment history")
                    .color(DashColors::text_primary(dark_mode)),
            );
            return action;
        }

        // Loading indicator
        if self.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading payment history...");
            });
            return action;
        }

        // Payment list
        ScrollArea::vertical().show(ui, |ui| {
            if self.payments.is_empty() {
                ui.label("No payments loaded");
            } else {
                for payment in &self.payments {
                    ui.group(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.horizontal(|ui| {
                            // Direction indicator
                            if payment.is_incoming {
                                ui.label(
                                    RichText::new("⬇")
                                        .color(egui::Color32::DARK_GREEN)
                                        .size(20.0),
                                );
                            } else {
                                ui.label(
                                    RichText::new("⬆").color(egui::Color32::DARK_RED).size(20.0),
                                );
                            }

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    // Contact name
                                    ui.label(
                                        RichText::new(&payment.contact_name)
                                            .strong()
                                            .color(DashColors::text_primary(dark_mode)),
                                    );

                                    // Amount
                                    let amount_str = format!("{} Dash", payment.amount.to_string());
                                    if payment.is_incoming {
                                        ui.label(
                                            RichText::new(format!("+{}", amount_str))
                                                .color(egui::Color32::DARK_GREEN),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!("-{}", amount_str))
                                                .color(egui::Color32::DARK_RED),
                                        );
                                    }
                                });

                                // Memo
                                if let Some(memo) = &payment.memo {
                                    ui.label(
                                        RichText::new(format!("\"{}\"", memo))
                                            .italics()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                }

                                ui.horizontal(|ui| {
                                    // Transaction ID
                                    ui.label(
                                        RichText::new(&payment.tx_id)
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );

                                    // Timestamp
                                    ui.label(
                                        RichText::new("• 2 days ago")
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                });
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            }
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPayPaymentHistory(payment_data) => {
                self.payments.clear();

                // Convert backend data to PaymentRecord structs
                for (tx_id, contact_name, amount, is_incoming, memo) in payment_data {
                    let payment = PaymentRecord {
                        tx_id,
                        contact_name,
                        amount: Credits::from(amount),
                        is_incoming,
                        timestamp: 0, // TODO: Include timestamp in backend data
                        memo: if memo.is_empty() { None } else { Some(memo) },
                    };
                    self.payments.push(payment);
                }

                self.message = Some((
                    format!("Found {} payments", self.payments.len()),
                    MessageType::Success,
                ));
            }
            _ => {
                self.message = Some(("Operation completed".to_string(), MessageType::Success));
            }
        }
    }
}
