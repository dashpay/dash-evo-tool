use crate::app::AppAction;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::backend_task::dashpay::DashPayTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::{MessageType, ScreenLike};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::theme::DashColors;
use crate::ui::helpers::{add_identity_key_chooser, TransactionType};
use dash_sdk::platform::IdentityPublicKey;
use egui::{Context, RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
enum ContactRequestStatus {
    NotStarted,
    Sending,
    Success(String), // Success message
    Error(String),   // Error message
}

pub struct AddContactScreen {
    pub app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    selected_key: Option<IdentityPublicKey>,
    username_or_id: String,
    account_label: String,
    message: Option<(String, MessageType)>,
    status: ContactRequestStatus,
}

impl AddContactScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_identity_string: String::new(),
            selected_key: None,
            username_or_id: String::new(),
            account_label: String::new(),
            message: None,
            status: ContactRequestStatus::NotStarted,
        }
    }

    fn send_contact_request(&mut self) -> AppAction {
        if let (Some(identity), Some(signing_key)) = (self.selected_identity.clone(), self.selected_key.clone()) {
            if self.username_or_id.is_empty() {
                self.display_message("Please enter a username or identity ID", MessageType::Error);
                return AppAction::None;
            }

            self.status = ContactRequestStatus::Sending;
            
            // Create the backend task to send the contact request
            let task = BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequest {
                identity,
                signing_key,
                to_username: self.username_or_id.clone(),
                account_label: if self.account_label.is_empty() { 
                    None 
                } else { 
                    Some(self.account_label.clone()) 
                },
            }));
            
            AppAction::BackendTask(task)
        } else {
            if self.selected_identity.is_none() {
                self.display_message("No identity selected", MessageType::Error);
            } else {
                self.display_message("No signing key selected", MessageType::Error);
            }
            AppAction::None
        }
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            
            ui.heading("🎉");
            ui.heading("Contact Request Sent Successfully!");
            
            ui.add_space(20.0);
            
            if let ContactRequestStatus::Success(ref msg) = self.status {
                ui.label(RichText::new(msg).size(14.0));
            }
            
            ui.add_space(30.0);
            
            if ui.button("Send Another Request").clicked() {
                // Reset the form to send another request
                self.status = ContactRequestStatus::NotStarted;
                self.selected_key = None;
                action = AppAction::Refresh;
            }
            
            ui.add_space(10.0);
            
            if ui.button("Back to Contacts").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            
            ui.add_space(10.0);
            
            if ui.button("Back to DashPay").clicked() {
                action = AppAction::PopScreen;
            }
        });
        
        action
    }
}

impl ScreenLike for AddContactScreen {
    fn refresh(&mut self) {
        // Don't reset success status on refresh
        if !matches!(self.status, ContactRequestStatus::Success(_)) {
            self.status = ContactRequestStatus::NotStarted;
        }
        self.message = None;
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        // Add top panel with navigation breadcrumbs
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::GoToMainScreen),
                ("Send Contact Request", AppAction::None),
            ],
            vec![],
        );

        // Add left panel for DashPay navigation
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDashPayContacts,
        );

        // Main content in island central panel
        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            // Show success screen if request was successful
            if matches!(self.status, ContactRequestStatus::Success(_)) {
                return self.show_success_screen(ui);
            }

            // Header
            ui.heading("Send Contact Request");
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
            return inner_action;
        }

        ui.group(|ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            ui.label(RichText::new("Your Identity").strong().color(DashColors::text_primary(dark_mode)));
            ui.separator();
            
            ui.horizontal(|ui| {
                let response = ui.add(
                    IdentitySelector::new(
                        "add_contact_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .label("From Identity:")
                    .width(400.0)
                    .other_option(false),
                );

                if response.changed() {
                    self.selected_key = None; // Clear key when identity changes
                    self.message = None;
                }
            });
        });

        ui.add_space(10.0);

        // Key selector (only show if identity is selected)
        if let Some(selected_identity) = &self.selected_identity {
            ui.group(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.label(RichText::new("Signing Key").strong().color(DashColors::text_primary(dark_mode)));
                ui.label(RichText::new("Select a key for signing the contact request. Only ECDSA_SECP256K1 keys are shown as they support ECDH encryption.").small().color(DashColors::text_secondary(dark_mode)));
                ui.separator();
                
                add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    std::iter::once(selected_identity),
                    &mut Some(selected_identity.clone()),
                    &mut self.selected_key,
                    TransactionType::ContactRequest,
                );
            });
            ui.add_space(10.0);
        }

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to send contact request from");
            return inner_action;
        }
        
        if self.selected_key.is_none() {
            ui.label("Please select a signing key");
            return inner_action;
        }

        // Loading indicator
        if matches!(self.status, ContactRequestStatus::Sending) {
            ui.horizontal(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let spinner_color = if dark_mode {
                    egui::Color32::from_gray(200)
                } else {
                    egui::Color32::from_gray(60)
                };
                ui.add(egui::widgets::Spinner::default().color(spinner_color));
                ui.label(
                    RichText::new("Sending contact request...")
                        .color(DashColors::text_primary(dark_mode))
                );
            });
            ui.separator();
            return inner_action;
        }

        // Show error if any
        if let ContactRequestStatus::Error(ref err) = self.status {
            ui.colored_label(egui::Color32::DARK_RED, format!("Error: {}", err));
            ui.separator();
        }

        // Contact request form
        ScrollArea::vertical().show(ui, |ui| {
            ui.group(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.label(RichText::new("Contact Information").strong().color(DashColors::text_primary(dark_mode)));
                ui.separator();

                // Username or Identity ID input
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Username or Identity ID:").color(DashColors::text_primary(dark_mode)));
                    ui.add_space(10.0);
                });
                ui.add(
                    TextEdit::singleline(&mut self.username_or_id)
                        .hint_text("e.g., alice.dash or identity ID")
                        .desired_width(400.0),
                );
                ui.label(
                    RichText::new("Enter the DashPay username or full identity ID of the contact")
                        .small()
                        .color(DashColors::text_secondary(dark_mode))
                );

                ui.add_space(10.0);

                // Account label (optional)
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Account Label (optional):").color(DashColors::text_primary(dark_mode)));
                    ui.add_space(10.0);
                });
                ui.add(
                    TextEdit::singleline(&mut self.account_label)
                        .hint_text("e.g., Personal, Business, etc.")
                        .desired_width(400.0),
                );
                ui.label(
                    RichText::new("A label to help you identify this account relationship")
                        .small()
                        .color(DashColors::text_secondary(dark_mode))
                );

                ui.add_space(20.0);

                // Action buttons
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        inner_action |= AppAction::PopScreen;
                    }

                    if ui.button("Send Contact Request").clicked() {
                        inner_action |= self.send_contact_request();
                    }
                });
            });

            ui.add_space(20.0);

            // Additional information
            ui.group(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.label(RichText::new("About Contact Requests").strong().color(DashColors::text_primary(dark_mode)));
                ui.separator();
                ui.label(RichText::new("• Contact requests establish secure communication channels").color(DashColors::text_primary(dark_mode)));
                ui.label(RichText::new("• Both parties must accept before payments can be sent").color(DashColors::text_primary(dark_mode)));
                ui.label(RichText::new("• Your display name and username will be shared with the contact").color(DashColors::text_primary(dark_mode)));
                ui.label(RichText::new("• You can manage contacts from the Contacts screen").color(DashColors::text_primary(dark_mode)));
            });
        });

            inner_action
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
        if message_type == MessageType::Error {
            self.status = ContactRequestStatus::Error(message.to_string());
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::Message(message) => {
                if message.contains("successfully") {
                    // Set success status to show success screen
                    self.status = ContactRequestStatus::Success(message);
                    // Clear form for next use
                    self.username_or_id.clear();
                    self.account_label.clear();
                    self.selected_key = None;
                } else if message.contains("Error") || message.contains("Failed") {
                    self.status = ContactRequestStatus::Error(message.clone());
                    self.display_message(&message, MessageType::Error);
                } else {
                    self.status = ContactRequestStatus::NotStarted;
                    self.display_message(&message, MessageType::Info);
                }
            }
            _ => {
                self.status = ContactRequestStatus::Success("Contact request sent".to_string());
                self.username_or_id.clear();
                self.account_label.clear();
                self.selected_key = None;
            }
        }
    }

}

impl AddContactScreen {
    pub fn change_context(&mut self, app_context: Arc<AppContext>) {
        self.app_context = app_context;
    }
    
    pub fn refresh_on_arrival(&mut self) {
        self.refresh();
    }
}