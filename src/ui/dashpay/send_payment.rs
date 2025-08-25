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
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
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
                        RichText::new(self.from_identity.to_string())
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
                            RichText::new(format!("{}", self.to_contact_id))
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    }
                });

                ui.separator();

                // Amount input
                let _dark_mode = ui.ctx().style().visuals.dark_mode;
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
                            RichText::new("Send Payment").color(egui::Color32::WHITE),
                        )
                        .fill(if send_enabled {
                            egui::Color32::from_rgb(0, 141, 228) // Dash blue
                        } else {
                            egui::Color32::GRAY
                        });

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
        let mut new_self = Self {
            app_context: app_context.clone(),
            selected_identity: None,
            selected_identity_string: String::new(),
            payments: Vec::new(),
            message: None,
            loading: false,
        };

        // Auto-select first identity on creation if available
        if let Ok(identities) = app_context.load_local_qualified_identities() {
            if !identities.is_empty() {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                new_self.selected_identity = Some(identities[0].clone());
                new_self.selected_identity_string =
                    identities[0].identity.id().to_string(Encoding::Base58);

                // Load payments from database for this identity
                new_self.load_payments_from_database();
            }
        }

        new_self
    }

    fn load_payments_from_database(&mut self) {
        // Load saved payment history for the selected identity from database
        if let Some(identity) = &self.selected_identity {
            let identity_id = identity.identity.id();

            // Clear existing payments before loading
            self.payments.clear();

            // Load payment history from database (limit 100)
            if let Ok(stored_payments) = self.app_context.db.load_payment_history(&identity_id, 100)
            {
                for payment in stored_payments {
                    // Determine if incoming or outgoing based on identity
                    let is_incoming = payment.to_identity_id == identity_id.to_buffer().to_vec();
                    let contact_id = if is_incoming {
                        payment.from_identity_id
                    } else {
                        payment.to_identity_id
                    };

                    // Try to resolve contact name
                    let contact_name = if let Ok(contact_id) = Identifier::from_bytes(&contact_id) {
                        // First check if we have a saved contact with username
                        if let Ok(contacts) =
                            self.app_context.db.load_dashpay_contacts(&identity_id)
                        {
                            contacts
                                .iter()
                                .find(|c| c.contact_identity_id == contact_id.to_buffer().to_vec())
                                .and_then(|c| c.username.clone().or(c.display_name.clone()))
                                .unwrap_or_else(|| {
                                    format!(
                                        "Unknown ({})",
                                        &contact_id.to_string(Encoding::Base58)[0..8]
                                    )
                                })
                        } else {
                            format!(
                                "Unknown ({})",
                                &contact_id.to_string(Encoding::Base58)[0..8]
                            )
                        }
                    } else {
                        "Unknown".to_string()
                    };

                    let payment_record = PaymentRecord {
                        tx_id: payment.tx_id,
                        contact_name,
                        amount: Credits::from(payment.amount as u64),
                        is_incoming,
                        timestamp: payment.created_at as u64,
                        memo: payment.memo,
                    };

                    self.payments.push(payment_record);
                }
            }
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
        // Don't clear if we have data, just clear temporary states
        self.message = None;
        self.loading = false;

        // Auto-select first identity if none selected
        if self.selected_identity.is_none() {
            if let Ok(identities) = self.app_context.load_local_qualified_identities() {
                if !identities.is_empty() {
                    self.selected_identity = Some(identities[0].clone());
                    self.selected_identity_string = identities[0].display_string();
                }
            }
        }

        // Load payments from database if we have an identity selected and no payments loaded
        if self.selected_identity.is_some() && self.payments.is_empty() {
            self.load_payments_from_database();
        }
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

                    // Load payments from database for the newly selected identity
                    self.load_payments_from_database();
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
                            // Avatar placeholder
                            ui.vertical(|ui| {
                                ui.add_space(5.0);
                                ui.label(RichText::new("👤").size(30.0));
                            });

                            ui.add_space(5.0);

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
                                    let amount_str = format!("{} Dash", payment.amount);
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

                // Get current identity for saving to database
                if let Some(identity) = &self.selected_identity {
                    let identity_id = identity.identity.id();

                    // Convert backend data to PaymentRecord structs and save to database
                    for (tx_id, contact_name, amount, is_incoming, memo) in payment_data {
                        // Parse contact identity from contact_name if it contains ID
                        let contact_id = if contact_name.contains("(") && contact_name.contains(")")
                        {
                            // Extract ID from format "Unknown (abcd1234)"
                            let start = contact_name.find('(').unwrap() + 1;
                            let end = contact_name.find(')').unwrap();
                            let _id_str = &contact_name[start..end];
                            // This is likely a partial base58 ID, we'd need the full ID
                            // For now, we'll use a placeholder
                            Identifier::new([0; 32])
                        } else {
                            Identifier::new([0; 32])
                        };

                        let payment = PaymentRecord {
                            tx_id: tx_id.clone(),
                            contact_name,
                            amount: Credits::from(amount),
                            is_incoming,
                            timestamp: 0, // TODO: Include timestamp in backend data
                            memo: if memo.is_empty() {
                                None
                            } else {
                                Some(memo.clone())
                            },
                        };
                        self.payments.push(payment);

                        // Save to database
                        let (from_id, to_id, payment_type) = if is_incoming {
                            (contact_id, identity_id, "received")
                        } else {
                            (identity_id, contact_id, "sent")
                        };

                        let _ = self.app_context.db.save_payment(
                            &tx_id,
                            &from_id,
                            &to_id,
                            amount as i64,
                            if memo.is_empty() { None } else { Some(&memo) },
                            payment_type,
                        );
                    }
                } else {
                    // No selected identity, just populate in-memory
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
