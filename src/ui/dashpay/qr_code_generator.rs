use crate::app::AppAction;
use crate::backend_task::dashpay::auto_accept_proof::generate_auto_accept_proof;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::{MessageType, ScreenLike};
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

pub struct QRCodeGeneratorScreen {
    pub app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    account_reference: String,
    validity_hours: String,
    generated_qr_data: Option<String>,
    message: Option<(String, MessageType)>,
}

impl QRCodeGeneratorScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_identity_string: String::new(),
            account_reference: "0".to_string(),
            validity_hours: "24".to_string(),
            generated_qr_data: None,
            message: None,
        }
    }

    fn generate_qr_code(&mut self) {
        if let Some(identity) = &self.selected_identity {
            let account_ref = match self.account_reference.parse::<u32>() {
                Ok(v) => v,
                Err(_) => {
                    self.display_message("Invalid account reference number", MessageType::Error);
                    return;
                }
            };

            let validity = match self.validity_hours.parse::<u32>() {
                Ok(v) if v > 0 && v <= 720 => v, // Max 30 days
                _ => {
                    self.display_message("Validity hours must be between 1 and 720", MessageType::Error);
                    return;
                }
            };

            match generate_auto_accept_proof(identity, account_ref, validity) {
                Ok(proof_data) => {
                    let qr_string = proof_data.to_qr_string();
                    self.generated_qr_data = Some(qr_string);
                    self.display_message("QR code generated successfully", MessageType::Success);
                }
                Err(e) => {
                    self.display_message(&format!("Failed to generate QR code: {}", e), MessageType::Error);
                }
            }
        } else {
            self.display_message("Please select an identity first", MessageType::Error);
        }
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Generate Contact QR Code");
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
                ui.label(RichText::new("Configuration").strong());
                ui.separator();

                egui::Grid::new("qr_config_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Identity:");
                        ui.add(
                            IdentitySelector::new(
                                "qr_identity_selector",
                                &mut self.selected_identity_string,
                                &identities,
                            )
                            .selected_identity(&mut self.selected_identity)
                            .unwrap()
                            .width(300.0)
                            .other_option(false),
                        );
                        ui.end_row();

                        ui.label("Account Reference:");
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.account_reference)
                                    .hint_text("0")
                                    .desired_width(100.0)
                            );
                            ui.label(RichText::new("Which account index to use for this contact").small().weak());
                        });
                        ui.end_row();

                        ui.label("Validity (hours):");
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.validity_hours)
                                    .hint_text("24")
                                    .desired_width(100.0)
                            );
                            ui.label(RichText::new("How long the QR code remains valid").small().weak());
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Generate QR Code").clicked() {
                        self.generate_qr_code();
                    }

                    if self.generated_qr_data.is_some() {
                        if ui.button("Clear").clicked() {
                            self.generated_qr_data = None;
                            self.message = None;
                        }
                    }
                });
            });

            ui.add_space(20.0);

            // Display generated QR data
            let mut show_copied_message = false;
            if let Some(qr_data) = &self.generated_qr_data {
                ui.group(|ui| {
                    ui.label(RichText::new("Generated QR Code Data").strong());
                    ui.separator();

                    // Display as text for now (in production, would render actual QR code image)
                    ui.group(|ui| {
                        ui.label(RichText::new("QR Code (text representation):").small());
                        ui.code(qr_data);
                    });

                    ui.add_space(10.0);

                    let copy_text = qr_data.clone();
                    if ui.button("Copy to Clipboard").clicked() {
                        ui.output_mut(|o| o.copied_text = copy_text);
                        show_copied_message = true;
                    }

                    ui.add_space(10.0);

                    ui.label(RichText::new("ℹ️ Share this QR code with someone to establish a mutual contact").small());
                    ui.label(RichText::new("⚠️ Anyone with this QR code can automatically become your contact").small().color(egui::Color32::YELLOW));
                });
            }
            
            if show_copied_message {
                self.display_message("Copied to clipboard", MessageType::Success);
            }

            ui.add_space(20.0);

            // Information box
            ui.group(|ui| {
                ui.label(RichText::new("ℹ️ About Contact QR Codes").strong());
                ui.separator();
                ui.label("• QR codes allow instant mutual contact establishment");
                ui.label("• The recipient can scan to automatically send and accept contact requests");
                ui.label("• QR codes expire after the specified validity period");
                ui.label("• Each QR code is unique and can only be used once");
                ui.label("• The account reference determines which wallet account to use");
            });
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }
}

impl ScreenLike for QRCodeGeneratorScreen {
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
}