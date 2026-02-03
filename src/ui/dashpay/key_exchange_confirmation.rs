//! Key Exchange Confirmation Screen
//!
//! Displays the key exchange request details and allows the user to approve or reject.

use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::key_exchange_request::KeyExchangeRequest;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::dashpay::DashPaySubscreen;
use crate::ui::identities::get_selected_wallet;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{Context, RichText, ScrollArea, Ui};
use std::sync::{Arc, RwLock};

/// Status of the key exchange process
#[derive(Debug, Clone, PartialEq)]
enum KeyExchangeStatus {
    /// Waiting for user to approve or reject
    PendingApproval,
    /// Processing the request
    Processing,
    /// Successfully completed
    Success { app_label: Option<String> },
    /// Failed with error
    Error(String),
}

/// Screen for confirming a key exchange request
pub struct KeyExchangeConfirmationScreen {
    app_context: Arc<AppContext>,
    request: KeyExchangeRequest,
    request_network: Network,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    status: KeyExchangeStatus,
    message: Option<(String, MessageType)>,
}

impl KeyExchangeConfirmationScreen {
    /// Create a new key exchange confirmation screen
    pub fn new(
        app_context: Arc<AppContext>,
        request: KeyExchangeRequest,
        request_network: Network,
    ) -> Self {
        Self {
            app_context,
            request,
            request_network,
            selected_identity: None,
            selected_identity_string: String::new(),
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            status: KeyExchangeStatus::PendingApproval,
            message: None,
        }
    }

    /// Handle the approve action
    fn approve_request(&mut self) -> AppAction {
        let Some(identity) = self.selected_identity.clone() else {
            self.message = Some(("Please select an identity".to_string(), MessageType::Error));
            return AppAction::None;
        };

        self.status = KeyExchangeStatus::Processing;

        let task = BackendTask::DashPayTask(Box::new(DashPayTask::HandleKeyExchangeRequest {
            identity,
            request: self.request.clone(),
        }));

        AppAction::BackendTask(task)
    }

    /// Render the main content
    fn render_content(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Key Exchange Request");
        });
        ui.separator();

        // Show message if any
        if let Some((message, message_type)) = &self.message {
            let color = match message_type {
                MessageType::Success => DashColors::success_color(dark_mode),
                MessageType::Error => DashColors::error_color(dark_mode),
                MessageType::Info => DashColors::DASH_BLUE,
            };
            ui.colored_label(color, message);
            ui.add_space(10.0);
        }

        match &self.status {
            KeyExchangeStatus::Success { app_label } => {
                return self.render_success_screen(ui, app_label.clone());
            }
            KeyExchangeStatus::Error(error) => {
                return self.render_error_screen(ui, error.clone());
            }
            KeyExchangeStatus::Processing => {
                ui.horizontal(|ui| {
                    ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                    ui.label(
                        RichText::new("Processing key exchange request...")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                return action;
            }
            KeyExchangeStatus::PendingApproval => {}
        }

        // Check network mismatch
        if self.request_network != self.app_context.network {
            self.render_network_mismatch_warning(ui);
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {
            // Security warning banner
            self.render_security_warning(ui);

            ui.add_space(15.0);

            // Request details
            self.render_request_details(ui);

            ui.add_space(15.0);

            // Identity selector
            action |= self.render_identity_selector(ui);

            ui.add_space(15.0);

            // Action buttons
            action |= self.render_action_buttons(ui);
        });

        action
    }

    /// Render the network mismatch warning
    fn render_network_mismatch_warning(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.colored_label(
                DashColors::error_color(dark_mode),
                RichText::new("Network Mismatch").strong(),
            );
            ui.separator();

            ui.label(format!(
                "This request is for {} but you are connected to {}.",
                network_display_name(self.request_network),
                network_display_name(self.app_context.network)
            ));

            ui.add_space(10.0);

            ui.label("Please switch to the correct network or scan a different QR code.");
        });
    }

    /// Render the security warning banner
    fn render_security_warning(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Security Notice")
                        .strong()
                        .color(egui::Color32::from_rgb(255, 165, 0)),
                );
            });
            ui.separator();

            ui.label(
                RichText::new(
                    "A web application is requesting a login key for your identity. \
                     By approving, you will generate a deterministic key that the app can use \
                     to authenticate you.",
                )
                .color(DashColors::text_primary(dark_mode)),
            );

            ui.add_space(5.0);

            ui.label(
                RichText::new(
                    "Only approve if you trust the application and initiated this request.",
                )
                .small()
                .color(DashColors::text_secondary(dark_mode)),
            );
        });
    }

    /// Render the request details
    fn render_request_details(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.label(
                RichText::new("Request Details")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.separator();

            egui::Grid::new("key_exchange_details")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    // Application name
                    ui.label(
                        RichText::new("Application:").color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(self.request.display_name())
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();

                    // Contract ID
                    ui.label(
                        RichText::new("Contract ID:").color(DashColors::text_secondary(dark_mode)),
                    );
                    let contract_id_str = self.request.contract_id.to_string(Encoding::Base58);
                    // Truncate for display
                    let display_id = if contract_id_str.len() > 20 {
                        format!(
                            "{}...{}",
                            &contract_id_str[..10],
                            &contract_id_str[contract_id_str.len() - 10..]
                        )
                    } else {
                        contract_id_str.clone()
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&display_id)
                                .monospace()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(contract_id_str);
                        }
                    });
                    ui.end_row();

                    // Key index
                    ui.label(
                        RichText::new("Key Index:").color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format!("{}", self.request.key_index))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();

                    // Network
                    ui.label(
                        RichText::new("Network:").color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(network_display_name(self.request_network))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();

                    // Protocol version
                    ui.label(
                        RichText::new("Protocol Version:")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format!("{}", self.request.version))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();
                });
        });
    }

    /// Render the identity selector
    fn render_identity_selector(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            return super::render_no_identities_card(ui, &self.app_context);
        }

        ui.group(|ui| {
            ui.label(
                RichText::new("Select Identity")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.separator();

            ui.label(
                RichText::new("Choose which identity to use for this key exchange:")
                    .color(DashColors::text_secondary(dark_mode)),
            );

            ui.add_space(5.0);

            // Track previous identity to detect changes
            let prev_identity_id = self.selected_identity.as_ref().map(|i| {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                i.identity.id()
            });

            ui.horizontal(|ui| {
                ui.label("Identity:");
                ui.add(
                    IdentitySelector::new(
                        "key_exchange_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .width(300.0)
                    .other_option(false),
                );
            });

            // Update wallet if identity changed
            let new_identity_id = self.selected_identity.as_ref().map(|i| {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                i.identity.id()
            });

            if prev_identity_id != new_identity_id {
                if let Some(identity) = &self.selected_identity {
                    let mut error_message = None;
                    self.selected_wallet = get_selected_wallet(
                        identity,
                        Some(&self.app_context),
                        None,
                        &mut error_message,
                    );
                    if let Some(error) = error_message {
                        self.message = Some((error, MessageType::Error));
                    }
                } else {
                    self.selected_wallet = None;
                }
            }
        });

        action
    }

    /// Render the action buttons
    fn render_action_buttons(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.group(|ui| {
            // Check wallet lock status
            let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                if let Err(e) = try_open_wallet_no_password(wallet) {
                    self.message = Some((e, MessageType::Error));
                }
                wallet_needs_unlock(wallet)
            } else {
                false
            };

            if wallet_locked {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 50),
                    "Wallet is locked. Please unlock to proceed.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reject").clicked() {
                        action = AppAction::PopScreen;
                    }
                    ui.add_space(10.0);
                    if ui.button("Unlock Wallet").clicked() {
                        self.wallet_unlock_popup.open();
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    // Reject button
                    if ui.button("Reject").clicked() {
                        action = AppAction::PopScreen;
                    }

                    ui.add_space(20.0);

                    // Approve button
                    let can_approve = self.selected_identity.is_some()
                        && self.request_network == self.app_context.network;

                    let approve_button =
                        egui::Button::new(RichText::new("Approve").color(egui::Color32::WHITE))
                            .fill(if can_approve {
                                DashColors::DASH_BLUE
                            } else {
                                egui::Color32::GRAY
                            });

                    if ui.add_enabled(can_approve, approve_button).clicked() {
                        action = self.approve_request();
                    }
                });
            }
        });

        action
    }

    /// Render the success screen
    fn render_success_screen(&mut self, ui: &mut Ui, app_label: Option<String>) -> AppAction {
        crate::ui::helpers::show_success_screen(
            ui,
            format!(
                "Key exchange completed successfully{}!",
                app_label
                    .as_ref()
                    .map(|l| format!(" for {}", l))
                    .unwrap_or_default()
            ),
            vec![("Done".to_string(), AppAction::PopScreen)],
        )
    }

    /// Render the error screen
    fn render_error_screen(&mut self, ui: &mut Ui, error: String) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.colored_label(
                DashColors::error_color(dark_mode),
                RichText::new("Key Exchange Failed").strong(),
            );
            ui.separator();

            ui.label(RichText::new(&error).color(DashColors::error_color(dark_mode)));

            ui.add_space(15.0);

            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    action = AppAction::PopScreen;
                }

                ui.add_space(10.0);

                if ui.button("Try Again").clicked() {
                    self.status = KeyExchangeStatus::PendingApproval;
                    self.message = None;
                }
            });
        });

        action
    }
}

impl ScreenLike for KeyExchangeConfirmationScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Key Exchange", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ctx, &self.app_context, DashPaySubscreen::Contacts);

        action |= island_central_panel(ctx, |ui| self.render_content(ui));

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked, UI will update on next frame
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
        if message_type == MessageType::Error {
            self.status = KeyExchangeStatus::Error(message.to_string());
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::KeyExchangeComplete { app_label, .. } => {
                self.status = KeyExchangeStatus::Success { app_label };
            }
            BackendTaskSuccessResult::Message(message) => {
                if message.contains("Error") || message.contains("Failed") {
                    self.status = KeyExchangeStatus::Error(message);
                }
            }
            _ => {}
        }
    }
}

/// Get a user-friendly network name
fn network_display_name(network: Network) -> &'static str {
    match network {
        Network::Dash => "Mainnet",
        Network::Testnet => "Testnet",
        Network::Devnet => "Devnet",
        Network::Regtest => "Regtest",
        _ => "Unknown",
    }
}
