use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::dashpay::auto_accept_handler::generate_proof_for_request;
use crate::backend_task::dashpay::auto_accept_proof::AutoAcceptProofData;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::collections::HashSet;
use std::sync::Arc;

pub struct QRScannerScreen {
    pub app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    qr_data_input: String,
    parsed_qr_data: Option<AutoAcceptProofData>,
    message: Option<(String, MessageType)>,
    sending: bool,
}

impl QRScannerScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_identity_string: String::new(),
            qr_data_input: String::new(),
            parsed_qr_data: None,
            message: None,
            sending: false,
        }
    }

    fn parse_qr_code(&mut self) {
        if self.qr_data_input.is_empty() {
            self.display_message("Please enter QR code data", MessageType::Error);
            return;
        }

        match AutoAcceptProofData::from_qr_string(&self.qr_data_input) {
            Ok(data) => {
                self.parsed_qr_data = Some(data);
                self.display_message("QR code parsed successfully", MessageType::Success);
            }
            Err(e) => {
                self.parsed_qr_data = None;
                self.display_message(&format!("Invalid QR code: {}", e), MessageType::Error);
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
                        self.display_message("No suitable signing key found", MessageType::Error);
                        return AppAction::None;
                    }
                };

                // Generate proof for the request
                let proof = match generate_proof_for_request(&self.qr_data_input, identity) {
                    Ok(p) => p,
                    Err(e) => {
                        self.display_message(
                            &format!("Failed to generate proof: {}", e),
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
                        auto_accept_proof: proof,
                    }));

                return AppAction::BackendTask(task);
            } else {
                self.display_message("Please parse a QR code first", MessageType::Error);
            }
        } else {
            self.display_message("Please select an identity", MessageType::Error);
        }

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Scan Contact QR Code");
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
            // Identity selector
            let identities = self
                .app_context
                .load_local_qualified_identities()
                .unwrap_or_default();

            if identities.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 165, 0),
                    "⚠️ No identities loaded. Please load or create an identity first.",
                );
                return;
            }

            ui.group(|ui| {
                ui.label(RichText::new("1. Select Your Identity").strong());
                ui.separator();

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
            });

            ui.add_space(20.0);

            ui.group(|ui| {
                ui.label(RichText::new("2. Enter QR Code Data").strong());
                ui.separator();

                ui.label(RichText::new("Paste the QR code data below:").small());

                ui.add(
                    TextEdit::multiline(&mut self.qr_data_input)
                        .hint_text("dashpay:...")
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
                        self.message = None;
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

                    ui.horizontal(|ui| {
                        if self.sending {
                            ui.spinner();
                            ui.label("Sending contact request...");
                        } else if ui.button("Send Contact Request").clicked() {
                            action = self.send_contact_request_with_proof();
                        }
                    });

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

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.sending = false;
        match result {
            BackendTaskSuccessResult::Message(msg) => {
                self.display_message(&msg, MessageType::Success);
                // Clear the form on success
                self.qr_data_input.clear();
                self.parsed_qr_data = None;
            }
            _ => {
                self.display_message("Contact request sent successfully", MessageType::Success);
            }
        }
    }
}

impl ScreenLike for QRScannerScreen {
    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        egui::CentralPanel::default().show(ctx, |ui| {
            action = self.render(ui);
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.display_task_result(result);
    }
}
