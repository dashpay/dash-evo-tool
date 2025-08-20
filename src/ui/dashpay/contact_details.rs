use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Payment {
    pub tx_id: String,
    pub amount: Credits,
    pub timestamp: u64,
    pub is_incoming: bool,
    pub memo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContactInfo {
    pub identity_id: Identifier,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub is_hidden: bool,
    pub account_reference: u32,
}

pub struct ContactDetailsScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    pub contact_id: Identifier,
    contact_info: Option<ContactInfo>,
    payment_history: Vec<Payment>,
    editing_info: bool,
    edit_nickname: String,
    edit_note: String,
    edit_hidden: bool,
    message: Option<(String, MessageType)>,
    loading: bool,
}

impl ContactDetailsScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        identity: QualifiedIdentity,
        contact_id: Identifier,
    ) -> Self {
        let mut screen = Self {
            app_context,
            identity,
            contact_id,
            contact_info: None,
            payment_history: Vec::new(),
            editing_info: false,
            edit_nickname: String::new(),
            edit_note: String::new(),
            edit_hidden: false,
            message: None,
            loading: false,
        };
        screen.refresh();
        screen
    }

    pub fn refresh(&mut self) {
        // Don't set loading here - only when actually making backend requests
        self.loading = false;
        
        // Clear any existing data - real data should be loaded from backend when needed
        self.contact_info = None;
        self.payment_history.clear();
        self.message = None;
        
        // TODO: Implement real backend fetching of contact info and payment history
        // This should be triggered by user actions or specific backend tasks
    }

    fn start_editing(&mut self) {
        if let Some(info) = &self.contact_info {
            self.edit_nickname = info.nickname.clone().unwrap_or_default();
            self.edit_note = info.note.clone().unwrap_or_default();
            self.edit_hidden = info.is_hidden;
            self.editing_info = true;
        }
    }

    fn save_contact_info(&mut self) {
        // TODO: Save contact info via backend
        if let Some(info) = &mut self.contact_info {
            info.nickname = if self.edit_nickname.is_empty() {
                None
            } else {
                Some(self.edit_nickname.clone())
            };
            info.note = if self.edit_note.is_empty() {
                None
            } else {
                Some(self.edit_note.clone())
            };
            info.is_hidden = self.edit_hidden;
        }

        self.editing_info = false;
        self.display_message("Contact info updated", MessageType::Success);
    }

    fn cancel_editing(&mut self) {
        self.editing_info = false;
        self.edit_nickname.clear();
        self.edit_note.clear();
        self.edit_hidden = false;
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Contact Details");
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

        // Loading indicator
        if self.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading contact details...");
            });
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {
            if let Some(info) = self.contact_info.clone() {
                // Contact profile section
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        // Avatar placeholder
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("👤").size(60.0));
                            ui.small("Contact");
                        });

                        ui.vertical(|ui| {
                            // Display nickname if set, otherwise display name
                            let name = info
                                .nickname
                                .as_ref()
                                .or(info.display_name.as_ref())
                                .or(info.username.as_ref())
                                .map(|s| s.clone())
                                .unwrap_or_else(|| "Unknown".to_string());
                            ui.label(RichText::new(name).heading());

                            // Username
                            if let Some(username) = &info.username {
                                ui.label(RichText::new(format!("@{}", username)).strong());
                            }

                            // Bio
                            if let Some(bio) = &info.bio {
                                ui.label(RichText::new(bio).weak());
                            }

                            // Identity ID
                            ui.label(
                                RichText::new(format!("ID: {}", info.identity_id))
                                    .small()
                                    .weak(),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui.button("Send Payment").clicked() {
                                action = AppAction::AddScreen(
                                    ScreenType::DashPaySendPayment(
                                        self.identity.clone(),
                                        self.contact_id,
                                    )
                                    .create_screen(&self.app_context),
                                );
                            }
                        });
                    });
                });

                ui.add_space(10.0);

                // Contact info section
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Contact Information").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.editing_info {
                                if ui.button("Cancel").clicked() {
                                    self.cancel_editing();
                                }
                                if ui.button("Save").clicked() {
                                    self.save_contact_info();
                                }
                            } else {
                                if ui.button("Edit").clicked() {
                                    self.start_editing();
                                }
                            }
                        });
                    });

                    ui.separator();

                    if self.editing_info {
                        // Edit mode
                        ui.horizontal(|ui| {
                            ui.label("Nickname:");
                            ui.add(
                                TextEdit::singleline(&mut self.edit_nickname)
                                    .hint_text("Optional nickname for this contact"),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label("Note:");
                            ui.add(
                                TextEdit::multiline(&mut self.edit_note)
                                    .hint_text("Private notes about this contact")
                                    .desired_rows(3),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.edit_hidden, "Hide this contact");
                            if self.edit_hidden {
                                ui.label(
                                    RichText::new("(Contact will not appear in lists)")
                                        .small()
                                        .weak(),
                                );
                            }
                        });
                    } else {
                        // View mode
                        if let Some(nickname) = &info.nickname {
                            ui.horizontal(|ui| {
                                ui.label("Nickname:");
                                ui.label(nickname);
                            });
                        }

                        if let Some(note) = &info.note {
                            ui.horizontal(|ui| {
                                ui.label("Note:");
                                ui.label(note);
                            });
                        }

                        if info.is_hidden {
                            ui.label(
                                RichText::new("⚠️ This contact is hidden")
                                    .color(egui::Color32::YELLOW),
                            );
                        }
                    }
                });

                ui.add_space(10.0);

                // Payment history section
                ui.group(|ui| {
                    ui.label(RichText::new("Payment History").strong());
                    ui.separator();

                    if self.payment_history.is_empty() {
                        ui.label("No payment history with this contact");
                    } else {
                        for payment in &self.payment_history {
                            let dark_mode = ui.ctx().style().visuals.dark_mode;
                            ui.horizontal(|ui| {
                                // Direction indicator
                                if payment.is_incoming {
                                    ui.label(RichText::new("⬇").color(egui::Color32::DARK_GREEN));
                                } else {
                                    ui.label(RichText::new("⬆").color(egui::Color32::DARK_RED));
                                }

                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        // Amount
                                        let amount_str =
                                            format!("{} Dash", payment.amount.to_string());
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

                                        // Memo
                                        if let Some(memo) = &payment.memo {
                                            ui.label(
                                                RichText::new(format!("\"{}\"", memo)).italics().color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }
                                    });

                                    ui.horizontal(|ui| {
                                        // Transaction ID
                                        ui.label(RichText::new(&payment.tx_id).small().color(DashColors::text_secondary(dark_mode)));

                                        // Timestamp
                                        ui.label(RichText::new("• 2 days ago").small().color(DashColors::text_secondary(dark_mode)));
                                    });
                                });
                            });
                            ui.separator();
                        }
                    }
                });

                ui.add_space(10.0);

                // Actions section
                ui.group(|ui| {
                    ui.label(RichText::new("Actions").strong());
                    ui.separator();

                    ui.horizontal(|ui| {
                        if ui.button("Remove Contact").clicked() {
                            // TODO: Implement contact removal
                            self.display_message(
                                "Contact removal not yet implemented",
                                MessageType::Info,
                            );
                        }

                        if ui.button("Block Contact").clicked() {
                            // TODO: Implement contact blocking
                            self.display_message(
                                "Contact blocking not yet implemented",
                                MessageType::Info,
                            );
                        }
                    });
                });
            } else {
                // No contact info loaded
                ui.group(|ui| {
                    ui.label("No contact information available");
                    ui.separator();
                    ui.label(format!("Contact ID: {}", self.contact_id));
                    ui.add_space(10.0);
                    ui.label("Contact information will be loaded automatically when available from the backend.");
                });
            }
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }
}

impl ScreenLike for ContactDetailsScreen {
    fn refresh(&mut self) {
        self.refresh();
    }

    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel with contact name if available
        let contact_name = self.contact_info.as_ref()
            .and_then(|info| info.nickname.as_ref().or(info.display_name.as_ref().or(info.username.as_ref())))
            .map(|name| format!("Contact: {}", name))
            .unwrap_or_else(|| "Contact Details".to_string());

        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![("DashPay", AppAction::None), (&contact_name, AppAction::None)],
            vec![],
        );

        // Add left panel  
        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashPayContacts);

        action |= island_central_panel(ctx, |ui| {
            self.render(ui)
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }
}
