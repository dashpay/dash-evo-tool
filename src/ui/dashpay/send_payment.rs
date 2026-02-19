use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{Frame, Margin, RichText, ScrollArea, TextEdit, Ui};
use std::sync::{Arc, RwLock};

use super::format_relative_time;

const PAYMENT_GUIDELINES_INFO_TEXT: &str = "Payment Guidelines:\n\n\
    Payments to contacts use encrypted payment channels.\n\n\
    Only you and the recipient can see payment details.\n\n\
    Addresses are never reused for privacy.\n\n\
    Memos are stored locally and not sent on-chain.";

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
    show_info_popup: bool,
    payment_success: bool,
    tx_id: Option<String>,
    // Wallet unlock
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
}

impl SendPaymentScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        from_identity: QualifiedIdentity,
        to_contact_id: Identifier,
    ) -> Self {
        // Get wallet from identity's associated wallets
        let selected_wallet = from_identity.associated_wallets.values().next().cloned();

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
            show_info_popup: false,
            payment_success: false,
            tx_id: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
        }
    }

    fn load_contact_info(&mut self) {
        let owner_id = self.from_identity.identity.id();
        let network_str = self.app_context.network.to_string();
        if let Ok(contacts) = self
            .app_context
            .db
            .load_dashpay_contacts(&owner_id, &network_str)
        {
            let contact_bytes = self.to_contact_id.to_buffer().to_vec();
            if let Some(contact) = contacts
                .iter()
                .find(|c| c.contact_identity_id == contact_bytes)
            {
                self.to_contact_name = contact
                    .username
                    .clone()
                    .or_else(|| contact.display_name.clone());
            }
        }
    }

    fn send_payment(&mut self) -> AppAction {
        // Validate amount
        if self.amount.value() == 0 {
            self.display_message("Please enter an amount", MessageType::Error);
            return AppAction::None;
        }

        // Check wallet is available and unlocked
        let wallet_check = if let Some(wallet) = &self.selected_wallet {
            match wallet.read() {
                Ok(guard) => {
                    if guard.is_open() {
                        Ok(())
                    } else {
                        Err("Wallet must be unlocked to send a payment".to_string())
                    }
                }
                Err(e) => Err(format!("Failed to access wallet: {}", e)),
            }
        } else {
            Err("No wallet associated with this identity".to_string())
        };

        if let Err(e) = wallet_check {
            self.display_message(&e, MessageType::Error);
            return AppAction::None;
        }

        // Get amount in Dash (convert from duffs)
        let amount_dash = match self.amount.dash_to_duffs() {
            Ok(duffs) => duffs as f64 / 100_000_000.0,
            Err(e) => {
                self.display_message(&format!("Invalid amount: {}", e), MessageType::Error);
                return AppAction::None;
            }
        };

        self.sending = true;

        // Fire the backend task
        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::SendPaymentToContact {
                identity: self.from_identity.clone(),
                contact_id: self.to_contact_id,
                amount_dash,
                memo: if self.memo.is_empty() {
                    None
                } else {
                    Some(self.memo.clone())
                },
            },
        )))
    }

    fn show_success(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen(
            ui,
            format!(
                "Payment of {} sent successfully!{}",
                self.amount,
                if let Some(tx_id) = &self.tx_id {
                    format!("\n\nTransaction ID: {}", tx_id)
                } else {
                    String::new()
                }
            ),
            vec![
                ("Back to DashPay".to_string(), AppAction::GoToMainScreen),
                ("Send Another Payment".to_string(), AppAction::PopScreen),
            ],
        )
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Show success screen if payment was successful
        if self.payment_success {
            return self.show_success(ui);
        }

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Send Payment");
            ui.add_space(5.0);
            if crate::ui::helpers::info_icon_button(ui, PAYMENT_GUIDELINES_INFO_TEXT).clicked() {
                self.show_info_popup = true;
            }
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

        // Check wallet unlock
        let (wallet_open_error, needs_unlock) = if let Some(wallet) = &self.selected_wallet {
            let open_err = try_open_wallet_no_password(wallet).err();
            let needs = wallet_needs_unlock(wallet);
            (open_err, needs)
        } else {
            (None, false)
        };

        if let Some(e) = wallet_open_error {
            self.display_message(&e, MessageType::Error);
        }

        if needs_unlock {
            ui.add_space(10.0);
            ui.colored_label(
                egui::Color32::from_rgb(200, 150, 50),
                "Wallet is locked. Please unlock to send a payment.",
            );
            ui.add_space(8.0);
            if ui.button("Unlock Wallet").clicked() {
                self.wallet_unlock_popup.open();
            }
            ui.add_space(10.0);
            return AppAction::None;
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

                // Wallet Balance (from wallet, not identity)
                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new("Wallet Balance:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    let balance_dash = if let Some(wallet) = &self.selected_wallet {
                        if let Ok(wallet_guard) = wallet.read() {
                            wallet_guard.confirmed_balance_duffs() as f64 / 100_000_000.0
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    ui.label(
                        RichText::new(format!("{:.8} DASH", balance_dash))
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

                // Amount input - use wallet balance for max
                let max_balance = if let Some(wallet) = &self.selected_wallet {
                    if let Ok(wallet_guard) = wallet.read() {
                        wallet_guard.confirmed_balance_duffs()
                    } else {
                        0
                    }
                } else {
                    0
                };

                let amount_input = self.amount_input.get_or_insert_with(|| {
                    AmountInput::new(&self.amount)
                        .with_hint_text("Enter amount in Dash")
                        .with_max_button(true)
                        .with_max_amount(Some(max_balance))
                        .with_label("Amount:")
                });
                // Update max amount in case balance changed
                amount_input.set_max_amount(Some(max_balance));
                let response = amount_input.show(ui);
                if response.inner.has_changed()
                    && let Some(new_amount) = response.inner.changed_value()
                {
                    self.amount = new_amount.clone();
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
                                action = self.send_payment();
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

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ctx, &self.app_context, DashPaySubscreen::Payments);

        action |= island_central_panel(ctx, |ui| self.render(ui));

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut popup =
                        InfoPopup::new("Payment Guidelines", PAYMENT_GUIDELINES_INFO_TEXT);
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.sending = false;
        self.message = Some((message.to_string(), message_type));
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.sending = false;
        if let BackendTaskSuccessResult::DashPayPaymentSent(recipient, address, amount) = result {
            // Extract txid from the address (or we could modify the result to include it)
            self.payment_success = true;
            self.tx_id = Some(format!("Sent to {}", address));
            self.message = Some((
                format!("Payment of {} DASH sent to {}", amount, recipient),
                MessageType::Success,
            ));
        }
    }
}

// Payment History Component (used in main DashPay screen)
pub struct PaymentHistory {
    pub app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    payments: Vec<PaymentRecord>,
    message: Option<(String, MessageType)>,
    loading: bool,
    has_searched: bool,
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
            has_searched: false,
        };

        // Auto-select first identity on creation if available
        if let Ok(identities) = app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            new_self.selected_identity = Some(identities[0].clone());
            new_self.selected_identity_string =
                identities[0].identity.id().to_string(Encoding::Base58);

            // Load payments from database for this identity
            new_self.load_payments_from_database();
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
                        let network_str = self.app_context.network.to_string();
                        if let Ok(contacts) = self
                            .app_context
                            .db
                            .load_dashpay_contacts(&identity_id, &network_str)
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
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            self.selected_identity = Some(identities[0].clone());
            self.selected_identity_string = identities[0].display_string();
        }

        // Load payments from database if we have an identity selected and no payments loaded
        if self.selected_identity.is_some() && self.payments.is_empty() {
            self.load_payments_from_database();
        }
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        // Header with identity selector on the right
        ui.horizontal(|ui| {
            ui.heading("Payment History");

            if !identities.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let response = ui.add(
                        IdentitySelector::new(
                            "payment_history_identity_selector",
                            &mut self.selected_identity_string,
                            &identities,
                        )
                        .selected_identity(&mut self.selected_identity)
                        .unwrap()
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
        });

        ui.separator();

        if identities.is_empty() {
            return super::render_no_identities_card(ui, &self.app_context);
        }

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
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                Frame::group(ui.style())
                    .fill(ui.visuals().extreme_bg_color)
                    .corner_radius(5.0)
                    .outer_margin(Margin::same(20))
                    .shadow(ui.visuals().window_shadow)
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("No Payment History")
                                    .strong()
                                    .size(20.0)
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            ui.add_space(5.0);
                            ui.label(
                                RichText::new("No payments have been made with this identity.")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.add_space(10.0);
                        });
                    });
            } else {
                for payment in &self.payments {
                    ui.group(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.horizontal(|ui| {
                            // Avatar placeholder
                            ui.vertical(|ui| {
                                ui.add_space(5.0);
                                ui.label(
                                    RichText::new("👤").size(30.0).color(DashColors::DEEP_BLUE),
                                );
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
                                    let payment_time_text = format_relative_time(payment.timestamp)
                                        .map(|t| format!("• {}", t))
                                        .unwrap_or_default();
                                    if !payment_time_text.is_empty() {
                                        ui.label(
                                            RichText::new(payment_time_text)
                                                .small()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
                                    }
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
                self.has_searched = true;

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

                // Don't show message - let the UI handle empty state
                self.message = None;
            }
            _ => {
                // Ignore other results
            }
        }
    }
}
