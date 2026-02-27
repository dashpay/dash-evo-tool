use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::dashpay::auto_accept_proof::AutoAcceptProofData;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
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
use crate::ui::components::{MessageBanner, ResultBannerExt};
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::identities::get_selected_wallet;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

pub struct QRScannerScreen {
    pub app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    qr_data_input: String,
    parsed_qr_data: Option<AutoAcceptProofData>,
    sending: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
}

impl QRScannerScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_identity_string: String::new(),
            qr_data_input: String::new(),
            parsed_qr_data: None,
            sending: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
        }
    }

    fn parse_qr_code(&mut self) {
        if self.qr_data_input.is_empty() {
            self.parsed_qr_data = None;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please enter QR code data",
                MessageType::Error,
            );
            return;
        }

        match AutoAcceptProofData::from_qr_string(&self.qr_data_input) {
            Ok(data) => {
                self.parsed_qr_data = Some(data);
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "QR code parsed successfully",
                    MessageType::Success,
                );
            }
            Err(e) => {
                self.parsed_qr_data = None;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Invalid QR code: {}", e),
                    MessageType::Error,
                );
            }
        }
    }

    fn send_contact_request_with_proof(&mut self) -> AppAction {
        if let Some(identity) = &self.selected_identity {
            if let Some(qr_data) = &self.parsed_qr_data {
                // Get signing key
                let signing_key = match identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([
                        SecurityLevel::CRITICAL,
                        SecurityLevel::HIGH,
                        SecurityLevel::MEDIUM,
                    ]),
                    HashSet::from([KeyType::ECDSA_SECP256K1]),
                    false,
                ) {
                    Some(key) => key,
                    None => {
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "No suitable signing key found. This operation requires a ECDSA_SECP256K1 AUTHENTICATION key.",
                            MessageType::Error,
                        );
                        return AppAction::None;
                    }
                };

                self.sending = true;

                // Create task to send contact request with proof
                let task =
                    BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequestWithProof {
                        identity: identity.clone(),
                        signing_key: signing_key.clone(),
                        to_identity_id: qr_data.identity_id,
                        account_label: Some(format!(
                            "QR Contact (Account #{})",
                            qr_data.account_reference
                        )),
                        qr_auto_accept: qr_data.clone(),
                    }));

                return AppAction::BackendTask(task);
            } else {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Please parse a QR code first",
                    MessageType::Error,
                );
            }
        } else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please select an identity",
                MessageType::Error,
            );
        }

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.heading("Scan Contact QR Code");
        ui.add_space(10.0);

        // Identity selector
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            action |= super::render_no_identities_card(ui, &self.app_context);
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {

            ui.group(|ui| {
                ui.label(RichText::new("1. Select Your Identity").strong());
                ui.separator();

                // Track identity before selection to detect changes
                let prev_identity_id = self.selected_identity.as_ref().map(|i| i.identity.id());

                ui.horizontal(|ui| {
                    ui.label("Identity:");
                    ui.add(
                        IdentitySelector::new(
                            "qr_scanner_identity_selector",
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
                let new_identity_id = self.selected_identity.as_ref().map(|i| i.identity.id());
                if prev_identity_id != new_identity_id {
                    if let Some(identity) = &self.selected_identity {
                        self.selected_wallet = get_selected_wallet(
                            identity,
                            Some(&self.app_context),
                            None,
                        )
                        .or_show_error(self.app_context.egui_ctx())
                        .unwrap_or(None);
                    } else {
                        self.selected_wallet = None;
                    }
                }
            });

            ui.add_space(20.0);

            ui.group(|ui| {
                ui.label(RichText::new("2. Enter QR Code Data").strong());
                ui.separator();

                ui.label(RichText::new("Paste the QR code data below:").small());

                ui.add(
                    TextEdit::multiline(&mut self.qr_data_input)
                        .hint_text("dash:?di=...")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                );

                ui.horizontal(|ui| {
                    if ui.button("Parse QR Code").clicked() {
                        self.parse_qr_code();
                    }

                    if ui.button("Clear").clicked() {
                        self.qr_data_input.clear();
                        self.parsed_qr_data = None;
                    }
                });
            });

            ui.add_space(20.0);

            // Display parsed QR data
            if let Some(qr_data) = self.parsed_qr_data.clone() {
                ui.group(|ui| {
                    ui.label(RichText::new("3. QR Code Details").strong());
                    ui.separator();

                    egui::Grid::new("qr_details_grid")
                        .num_columns(2)
                        .spacing([10.0, 5.0])
                        .show(ui, |ui| {
                            ui.label("Contact Identity:");
                            ui.label(qr_data.identity_id.to_string(
                                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58
                            ));
                            ui.end_row();

                            ui.label("Account Reference:");
                            ui.label(format!("{}", qr_data.account_reference));
                            ui.end_row();

                            ui.label("Expires:");
                            let expiry_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(qr_data.expires_at);
                            ui.label(format!("{:?}", expiry_time));
                            ui.end_row();
                        });

                    ui.add_space(10.0);

                    // Check wallet lock status before showing send button
                    let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                        if let Err(e) = try_open_wallet_no_password(wallet) {
                            MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
                        }
                        wallet_needs_unlock(wallet)
                    } else {
                        false
                    };

                    if wallet_locked {
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to add contact.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                    } else {
                        ui.horizontal(|ui| {
                            if self.sending {
                                ui.spinner();
                                ui.label("Sending contact request...");
                            } else if ui.button("Add Contact").clicked() {
                                action = self.send_contact_request_with_proof();
                            }
                        });
                    }

                    ui.add_space(10.0);

                    ui.label(RichText::new("ℹ️ This will send a contact request that will be automatically accepted").small());
                    ui.label(RichText::new("⚡ Both you and the contact will become mutual contacts instantly").small());
                });
            }

            ui.add_space(20.0);

            // Information box
            ui.group(|ui| {
                ui.label(RichText::new("ℹ️ About QR Code Scanning").strong());
                ui.separator();
                ui.label("• QR codes enable instant mutual contact establishment");
                ui.label("• The contact request is automatically accepted by both parties");
                ui.label("• No manual approval is needed when using valid QR codes");
                ui.label("• QR codes expire after the specified time period");
                ui.label("• Each QR code can only be used once");
            });
        });

        action
    }

    pub fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.sending = false;
        if let BackendTaskSuccessResult::Message(_) = result {
            // Clear the form on success
            self.qr_data_input.clear();
            self.parsed_qr_data = None;
        }
    }
}

impl ScreenLike for QRScannerScreen {
    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Scan QR Code", AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashpay);

        // Add DashPay subscreen chooser panel
        action |=
            add_dashpay_subscreen_chooser_panel(ctx, &self.app_context, DashPaySubscreen::Contacts);

        // Main content area with island styling
        action |= island_central_panel(ctx, |ui| self.render(ui));

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully, UI will update on next frame
            }
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; no side-effects needed.
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.display_task_result(result);
    }
}
