use crate::app::AppAction;
use crate::backend_task::dashpay::auto_accept_proof::generate_auto_accept_proof;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use eframe::epaint::TextureHandle;
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
                    self.display_message(
                        "Validity hours must be between 1 and 720",
                        MessageType::Error,
                    );
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
                    self.display_message(
                        &format!("Failed to generate QR code: {}", e),
                        MessageType::Error,
                    );
                }
            }
        } else {
            self.display_message("Please select an identity first", MessageType::Error);
        }
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Header with info icon
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Generate Contact QR Code");
            ui.add_space(10.0);
            crate::ui::helpers::info_icon_button(
                ui,
                "About Contact QR Codes:\n\n\
                • QR codes allow instant mutual contact establishment\n\
                • The recipient can scan to automatically send and accept contact requests\n\
                • QR codes expire after the specified validity period\n\
                • Each QR code is unique and can only be used once\n\
                • The account reference determines which wallet account to use\n\n\
                WARNING: Anyone with this QR code can automatically become your contact",
            );
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
                    DashColors::warning_color(dark_mode),
                    "⚠️ No identities loaded. Please load or create an identity first.",
                );
                return;
            }

            ui.group(|ui| {
                ui.label(
                    RichText::new("Configuration")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.separator();

                egui::Grid::new("qr_config_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Identity:").color(DashColors::text_primary(dark_mode)),
                        );
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

                        ui.label(
                            RichText::new("Account Reference:")
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.account_reference)
                                    .hint_text("0")
                                    .desired_width(100.0),
                            );
                            ui.label(
                                RichText::new("Which account index to use for this contact")
                                    .small()
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        });
                        ui.end_row();

                        ui.label(
                            RichText::new("Validity (hours):")
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.validity_hours)
                                    .hint_text("24")
                                    .desired_width(100.0),
                            );
                            ui.label(
                                RichText::new("How long the QR code remains valid")
                                    .small()
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
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
                    ui.label(
                        RichText::new("Generated QR Code")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.separator();

                    // Center the QR code
                    ui.vertical_centered(|ui| {
                        // Generate and display the actual QR code image
                        if let Ok(qr_image) = generate_qr_code_image(qr_data) {
                            let texture: TextureHandle = ui.ctx().load_texture(
                                "dashpay_qr_code",
                                qr_image,
                                egui::TextureOptions::LINEAR,
                            );
                            // Display at a reasonable size
                            ui.image(&texture);
                        } else {
                            ui.label(
                                RichText::new("Failed to generate QR code image")
                                    .color(DashColors::error_color(dark_mode)),
                            );
                        }
                    });

                    ui.add_space(10.0);

                    // Show the text data in a collapsible section
                    ui.collapsing("QR Code Data (text)", |ui| {
                        ui.code(qr_data);
                    });

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        let copy_text = qr_data.clone();
                        if ui.button("Copy Data to Clipboard").clicked() {
                            ui.output_mut(|o| o.copied_text = copy_text);
                            show_copied_message = true;
                        }
                    });

                    ui.add_space(10.0);

                    ui.label(
                        RichText::new(
                            "Share this QR code with someone to establish a mutual contact",
                        )
                        .small()
                        .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(
                            "WARNING: Anyone with this QR code can automatically become your contact",
                        )
                        .small()
                        .color(DashColors::warning_color(dark_mode)),
                    );
                });
            }

            if show_copied_message {
                self.display_message("Copied to clipboard", MessageType::Success);
            }
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

        // Add top panel
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("QR Generator", AppAction::None),
            ],
            vec![],
        );

        // Add left panel for DashPay navigation
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDashPayContacts,
        );

        // Add DashPay subscreen chooser panel
        action |= add_dashpay_subscreen_chooser_panel(
            ctx,
            &self.app_context,
            DashPaySubscreen::Contacts, // Use Contacts as the active subscreen since QR Generator is launched from there
        );

        // Main content area with island styling
        action |= island_central_panel(ctx, |ui| self.render(ui));

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }
}
