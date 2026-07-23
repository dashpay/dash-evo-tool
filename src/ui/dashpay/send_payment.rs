use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::dashpay::{MAX_PAYMENT_MEMO_CHARS, validate_payment_memo};
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::MessageBanner;
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
    sending: bool,
    show_info_popup: bool,
    payment_success: bool,
    tx_id: Option<String>,
    // Wallet unlock
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
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
            sending: false,
            show_info_popup: false,
            payment_success: false,
            tx_id: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
        }
    }

    fn load_contact_info(&mut self) {
        // The DET contacts cache is gone after D3; the recipient name is supplied
        // via the routing screen (see ContactDetailsScreen / ContactsList) or
        // remains None and the UI falls back to displaying the contact ID.
    }

    fn send_payment(&mut self) -> AppAction {
        // Validate amount
        if self.amount.value() == 0 {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please enter an amount",
                MessageType::Error,
            );
            return AppAction::None;
        }

        // Check wallet is available and unlocked
        let Some(wallet) = &self.selected_wallet else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "No wallet is associated with this identity. Load its wallet and try again.",
                MessageType::Error,
            );
            return AppAction::None;
        };
        match wallet.read() {
            Ok(guard) if guard.is_open() => {}
            Ok(_) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Unlock the wallet before sending this payment, then try again.",
                    MessageType::Error,
                );
                return AppAction::None;
            }
            Err(error) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "The wallet could not be opened for this payment. Wait a moment and try again.",
                    MessageType::Error,
                )
                .with_details(error);
                return AppAction::None;
            }
        }

        // Resolve the amount in duffs at the UI edge — no floating-point value
        // crosses into the backend.
        let amount_duffs = match self.amount.dash_to_duffs() {
            Ok(duffs) => duffs,
            Err(error) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "The payment amount is not valid. Check the amount and try again.",
                    MessageType::Error,
                )
                .with_details(error);
                return AppAction::None;
            }
        };

        self.sending = true;

        // Fire the backend task
        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::SendPaymentToContact {
                identity: self.from_identity.clone(),
                contact_id: self.to_contact_id,
                amount_duffs,
                memo: if self.memo.is_empty() {
                    None
                } else {
                    Some(self.memo.clone())
                },
            },
        )))
    }

    fn show_success(&self, ui: &mut Ui) -> AppAction {
        let message = if let Some(tx_id) = &self.tx_id {
            format!(
                "Payment of {amount} sent successfully!\n\nTransaction ID: {tx_id}",
                amount = self.amount
            )
        } else {
            format!(
                "Payment of {amount} sent successfully!",
                amount = self.amount
            )
        };
        crate::ui::helpers::show_success_screen(
            ui,
            message,
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

        // Check wallet unlock
        let needs_unlock = if let Some(wallet) = &self.selected_wallet {
            if !self.wallet_open_attempted {
                if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                    MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                        .disable_auto_dismiss();
                }
                self.wallet_open_attempted = true;
            }
            wallet_needs_unlock(wallet)
        } else {
            false
        };

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
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new("From:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new(self.from_identity.to_string())
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });

                // Wallet Balance (from wallet, not identity)
                ui.horizontal(|ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new("Wallet Balance:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    let balance_dash = if let Some(wallet) = &self.selected_wallet {
                        if let Ok(wallet_guard) = wallet.read() {
                            self.app_context
                                .snapshot_balance(&wallet_guard.seed_hash())
                                .spendable() as f64
                                / 100_000_000.0
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    ui.label(
                        RichText::new(format!("{balance_dash:.8} DASH"))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });

                ui.separator();

                // To contact
                ui.horizontal(|ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new("To:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    if let Some(name) = &self.to_contact_name {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.label(RichText::new(name).color(DashColors::text_primary(dark_mode)));
                    } else {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.label(
                            RichText::new(self.to_contact_id.to_string(Encoding::Base58))
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    }
                });

                ui.separator();

                // Amount input - use the spendable wallet balance for max, so it
                // matches the coin selector (confirmed + unconfirmed) and does
                // not understate IS-locked funds awaiting their local flag.
                let max_balance = if let Some(wallet) = &self.selected_wallet {
                    if let Ok(wallet_guard) = wallet.read() {
                        self.app_context
                            .snapshot_balance(&wallet_guard.seed_hash())
                            .spendable()
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
                let dark_mode = ui.style().visuals.dark_mode;
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
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(
                    RichText::new(format!(
                        "{count}/{max} characters",
                        count = self.memo.chars().count(),
                        max = MAX_PAYMENT_MEMO_CHARS
                    ))
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
                            if let Err(error) = validate_payment_memo(&self.memo) {
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    "The memo is too long. Use 100 characters or fewer and try again.",
                                    MessageType::Error,
                                )
                                .with_details(error);
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

    pub fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
    }
}

impl ScreenLike for SendPaymentScreen {
    fn refresh(&mut self) {
        self.load_contact_info();
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Send Payment", AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ui, &self.app_context, DashPaySubscreen::Payments);

        action |= island_central_panel(ui, |ui| self.render(ui));

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("dashpay_send_payment_info_popup"),
                        "Payment Guidelines",
                        PAYMENT_GUIDELINES_INFO_TEXT,
                    );
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

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        self.sending = false;
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.sending = false;
        if let BackendTaskSuccessResult::DashPayPaymentSent(_recipient, address, _amount) = result {
            self.payment_success = true;
            self.tx_id = Some(format!("Sent to {address}"));
        }
    }
}

// Payment History Component (used in main DashPay screen)
pub struct PaymentHistory {
    pub app_context: Arc<AppContext>,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    payments: Vec<PaymentRecord>,
    loading: bool,
    has_searched: bool,
}

#[derive(Debug, Clone)]
pub struct PaymentRecord {
    pub tx_id: String,
    pub contact_name: String,
    /// Payment amount in **duffs** (1 DASH = 100,000,000 duffs), as provided by
    /// `DashPayPaymentHistory`. Despite the `Credits` alias this is a duff value,
    /// so render it with `format_duffs_as_dash`.
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
            loading: false,
            has_searched: false,
        };

        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        if let Ok(identities) = app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            let selected_id = app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());
            new_self.selected_identity = Some(preferred.clone());
            new_self.selected_identity_string = preferred.identity.id().to_string(Encoding::Base58);
        }

        new_self
    }

    pub fn trigger_fetch_payment_history(&mut self) -> AppAction {
        if let Some(identity) = &self.selected_identity {
            self.loading = true;
            // Mark the attempt at dispatch time, not on success. A failed load
            // resets `loading` in `display_message` but leaves this flag set, so
            // the auto-fetch gate fires exactly once and a transient error can't
            // drive a re-dispatch storm. A fresh attempt is opted into via
            // `refresh()` or an identity change.
            self.has_searched = true;

            let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory {
                identity: identity.clone(),
            }));

            return AppAction::BackendTask(task);
        }

        AppAction::None
    }

    pub fn refresh(&mut self) {
        // Don't clear if we have data, just clear temporary states
        self.loading = false;

        // Seed from the app-scoped selected identity if none yet selected (W3 SYNC).
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            let selected_id = self.app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());
            self.selected_identity = Some(preferred.clone());
            self.selected_identity_string = preferred.identity.id().to_string(Encoding::Base58);
        }

        // Reset the fetched flag if we have no payments; next render dispatches
        // `LoadPaymentHistory` via `has_searched == false`.
        if self.selected_identity.is_some() && self.payments.is_empty() {
            self.has_searched = false;
        }
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Auto-dispatch `LoadPaymentHistory` on first render or after identity change.
        if !self.has_searched && !self.loading && self.selected_identity.is_some() {
            action = self.trigger_fetch_payment_history();
        }

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();

        // Header with identity selector on the right
        ui.horizontal(|ui| {
            ui.heading("Payment History");

            if !identities.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // SYNC: write-back via syncing_global on user pick (FR-6: the source list is
                    // User-only, so a masternode/evonode can never leak to the app-global identity).
                    let response = ui.add(
                        IdentitySelector::new(
                            "payment_history_identity_selector",
                            &mut self.selected_identity_string,
                            &identities,
                        )
                        .selected_identity(&mut self.selected_identity)
                        .unwrap()
                        .width(300.0)
                        .other_option(false) // Disable "Other" option
                        .syncing_global(self.app_context.clone()),
                    );

                    if response.changed() {
                        self.refresh();
                        // The next render dispatches `LoadPaymentHistory`
                        // via `has_searched == false`.
                        self.has_searched = false;
                    }
                });
            }
        });

        ui.separator();

        if identities.is_empty() {
            return super::render_no_identities_card(ui, &self.app_context);
        }

        if self.selected_identity.is_none() {
            let dark_mode = ui.style().visuals.dark_mode;
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
                let dark_mode = ui.style().visuals.dark_mode;
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
                        let dark_mode = ui.style().visuals.dark_mode;
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

                                    // Amount (payment.amount is in duffs)
                                    let amount_str = format_duffs_as_dash(payment.amount);
                                    if payment.is_incoming {
                                        ui.label(
                                            RichText::new(format!("+{amount_str}"))
                                                .color(egui::Color32::DARK_GREEN),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!("-{amount_str}"))
                                                .color(egui::Color32::DARK_RED),
                                        );
                                    }
                                });

                                // Memo
                                if let Some(memo) = &payment.memo {
                                    ui.label(
                                        RichText::new(format!("\"{memo}\""))
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
                                        .map(|transaction| format!("• {transaction}"))
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

    pub fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for
        // side-effects. Settle the spinner so a failed `LoadPaymentHistory`
        // doesn't strand the widget on the loading state (`has_searched` was
        // already set at dispatch, so this won't re-trigger the auto-fetch).
        self.loading = false;
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
                            let start = contact_name
                                .find('(')
                                .expect("invariant: '(' present per the contains check above")
                                + 1;
                            let end = contact_name
                                .find(')')
                                .expect("invariant: ')' present per the contains check above");
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
                        // `payments::send_payment_to_contact` records outgoing payments
                        // through `WalletBackend::dashpay_record_payment` + the
                        // payment-timestamp sidecar, so the upstream wallet is the
                        // single source of truth; no mirror is needed here.
                        let _ = (contact_id, identity_id, tx_id, amount, memo, is_incoming);
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
            }
            _ => {
                // Ignore other results
            }
        }
    }
}
