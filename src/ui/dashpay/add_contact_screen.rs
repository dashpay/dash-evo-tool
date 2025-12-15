use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    try_open_wallet_no_password, wallet_needs_unlock, WalletUnlockPopup, WalletUnlockResult,
};
use crate::ui::dashpay::DashPaySubscreen;
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::platform::IdentityPublicKey;
use egui::{Context, RichText, ScrollArea, TextEdit, Ui};
use std::sync::{Arc, RwLock};

const CONTACT_REQUEST_INFO_TEXT: &str = "About Contact Requests:\n\n\
    Contact requests establish secure communication channels.\n\n\
    Both parties must accept before payments can be sent.\n\n\
    Your display name and username will be shared with the contact.\n\n\
    You can manage contacts from the Contacts screen.";

#[derive(Debug, Clone, PartialEq)]
enum ContactRequestStatus {
    NotStarted,
    Sending,
    Success(String),     // Success message
    Error(DashPayError), // Structured error with user-friendly messaging
}

pub struct AddContactScreen {
    pub app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    username_or_id: String,
    account_label: String,
    message: Option<(String, MessageType)>,
    status: ContactRequestStatus,
    show_info_popup: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
}

impl AddContactScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_key: None,
            username_or_id: String::new(),
            account_label: String::new(),
            message: None,
            status: ContactRequestStatus::NotStarted,
            show_info_popup: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
        }
    }

    pub fn new_with_identity_id(app_context: Arc<AppContext>, identity_id: String) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_key: None,
            username_or_id: identity_id,
            account_label: String::new(),
            message: None,
            status: ContactRequestStatus::NotStarted,
            show_info_popup: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
        }
    }

    fn send_contact_request(&mut self) -> AppAction {
        if let (Some(identity), Some(signing_key)) =
            (self.selected_identity.clone(), self.selected_key.clone())
        {
            // Validate input using DashPayError system
            if self.username_or_id.is_empty() {
                let error = DashPayError::MissingField {
                    field: "username or identity ID".to_string(),
                };
                self.status = ContactRequestStatus::Error(error.clone());
                self.display_message(&error.user_message(), MessageType::Error);
                return AppAction::None;
            }

            // Validate username format if it looks like a username
            if self.username_or_id.contains('.') && !self.username_or_id.ends_with(".dash") {
                let error = DashPayError::InvalidUsername {
                    username: self.username_or_id.clone(),
                };
                self.status = ContactRequestStatus::Error(error.clone());
                self.display_message(&error.user_message(), MessageType::Error);
                return AppAction::None;
            }

            // Validate account label length
            if self.account_label.len() > 100 {
                let error = DashPayError::AccountLabelTooLong {
                    length: self.account_label.len(),
                    max: 100,
                };
                self.status = ContactRequestStatus::Error(error.clone());
                self.display_message(&error.user_message(), MessageType::Error);
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
            let error = if self.selected_identity.is_none() {
                DashPayError::MissingField {
                    field: "identity".to_string(),
                }
            } else {
                DashPayError::MissingField {
                    field: "signing key".to_string(),
                }
            };
            self.status = ContactRequestStatus::Error(error.clone());
            self.display_message(&error.user_message(), MessageType::Error);
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
                ("DashPay", AppAction::None),
                ("Send Contact Request", AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ctx, &self.app_context, DashPaySubscreen::Contacts);

        // Main content in island central panel
        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            // Show success screen if request was successful
            if matches!(self.status, ContactRequestStatus::Success(_)) {
                return self.show_success_screen(ui);
            }

            // Header with Back button and info icon
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    inner_action = AppAction::PopScreen;
                }
                ui.heading("Send Contact Request");
                ui.add_space(5.0);
                if crate::ui::helpers::info_icon_button(ui, CONTACT_REQUEST_INFO_TEXT).clicked() {
                    self.show_info_popup = true;
                }
            });
            ui.separator();

            // Show message if any (but not if we have an error status, to avoid duplication)
            if !matches!(self.status, ContactRequestStatus::Error(_))
                && let Some((message, message_type)) = &self.message
            {
                let color = match message_type {
                    MessageType::Success => egui::Color32::DARK_GREEN,
                    MessageType::Error => egui::Color32::DARK_RED,
                    MessageType::Info => egui::Color32::LIGHT_BLUE,
                };
                ui.colored_label(color, message);
                ui.separator();
            }

            // Identity and Key selector
            let identities = self
                .app_context
                .load_local_qualified_identities()
                .unwrap_or_default();

            if identities.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 165, 0),
                    "No identities loaded. Please load or create an identity first.",
                );
                return inner_action;
            }

            ui.group(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.label(
                    RichText::new("From (Sender)")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.separator();

                // Track identity before selection to detect changes
                let prev_identity_id = self.selected_identity.as_ref().map(|i| {
                    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                    i.identity.id()
                });

                let key_action = add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    identities.iter(),
                    &mut self.selected_identity,
                    &mut self.selected_key,
                    TransactionType::ContactRequest,
                );
                if !matches!(key_action, AppAction::None) {
                    inner_action = key_action;
                }

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
                    } else {
                        self.selected_wallet = None;
                    }
                }
            });

            ui.add_space(10.0);

            // Loading indicator
            if matches!(self.status, ContactRequestStatus::Sending) {
                ui.horizontal(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                    ui.label(
                        RichText::new("Sending contact request...")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                ui.separator();
            }

            // Show error if any
            if let ContactRequestStatus::Error(ref err) = self.status {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let error_color = if dark_mode {
                    egui::Color32::from_rgb(255, 100, 100)
                } else {
                    egui::Color32::DARK_RED
                };

                ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(err.user_message()).color(error_color));

                        // Show retry suggestion for recoverable errors
                        if err.is_recoverable() {
                            ui.label(RichText::new("You can try again.").small().color(DashColors::text_secondary(dark_mode)));
                        }

                        // Show action suggestion for user errors
                        if err.requires_user_action() {
                            match err {
                                DashPayError::UsernameResolutionFailed { .. } => {
                                    ui.label(RichText::new("Tip: Make sure the username is spelled correctly and exists on Dash Platform.").small().color(DashColors::text_secondary(dark_mode)));
                                }
                                DashPayError::InvalidUsername { .. } => {
                                    ui.label(RichText::new("Tip: Usernames must end with '.dash' (e.g., alice).").small().color(DashColors::text_secondary(dark_mode)));
                                }
                                DashPayError::AccountLabelTooLong { .. } => {
                                    ui.label(RichText::new("Tip: Try a shorter, more descriptive label.").small().color(DashColors::text_secondary(dark_mode)));
                                }
                                _ => {}
                            }
                        }
                    });
                });
            });
                ui.separator();
            }

            // Contact request form
            ScrollArea::vertical().show(ui, |ui| {
                ui.group(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new("To (Recipient)")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.separator();

                    // Username/ID and Relationship Label in 2x2 grid
                    egui::Grid::new("contact_request_form")
                        .num_columns(2)
                        .spacing([10.0, 10.0])
                        .show(ui, |ui| {
                            // Row 1: Username/ID
                            ui.label(
                                RichText::new("Username or Identity ID:")
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            ui.add(
                                TextEdit::singleline(&mut self.username_or_id)
                                    .hint_text("e.g., alice.dash or identity ID")
                                    .desired_width(350.0),
                            );
                            ui.end_row();

                            // Row 2: Relationship Label
                            ui.label(
                                RichText::new("Relationship Label (optional):")
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            ui.add(
                                TextEdit::singleline(&mut self.account_label)
                                    .hint_text("e.g., Friend, Family, Business Partner")
                                    .desired_width(350.0),
                            );
                        });

                    ui.add_space(10.0);
                });

                // Show summary if all required fields are filled
                if self.selected_identity.is_some() && !self.username_or_id.is_empty() {
                    ui.group(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.label(
                            RichText::new("Request Summary")
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.separator();

                        if let Some(identity) = &self.selected_identity {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("From:")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                ui.label(
                                    RichText::new(identity.to_string())
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });

                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("To:")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                ui.label(
                                    RichText::new(&self.username_or_id)
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });

                            if !self.account_label.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Label:")
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                    ui.label(
                                        RichText::new(&self.account_label)
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                });
                            }
                        }
                    });
                    ui.add_space(10.0);
                }

                ui.group(|ui| {
                    let _dark_mode = ui.ctx().style().visuals.dark_mode;

                    // Check wallet lock status before showing send button
                    let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                        if let Err(e) = try_open_wallet_no_password(wallet) {
                            self.message = Some((e, MessageType::Error));
                        }
                        wallet_needs_unlock(wallet)
                    } else {
                        false
                    };

                    if wallet_locked {
                        ui.add_space(10.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to send contact request.",
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                inner_action |= AppAction::PopScreen;
                            }
                            ui.add_space(10.0);
                            if ui.button("Unlock Wallet").clicked() {
                                self.wallet_unlock_popup.open();
                            }
                        });
                    } else {
                        // Action buttons
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                inner_action |= AppAction::PopScreen;
                            }

                            ui.add_space(10.0);

                            let send_button_enabled = !self.username_or_id.is_empty()
                                && self.selected_identity.is_some()
                                && self.selected_key.is_some();

                            let send_button = egui::Button::new(
                                RichText::new("Send Contact Request").color(egui::Color32::WHITE),
                            )
                            .fill(if send_button_enabled {
                                egui::Color32::from_rgb(0, 141, 228) // Dash blue
                            } else {
                                egui::Color32::GRAY
                            });

                            if ui.add_enabled(send_button_enabled, send_button).clicked() {
                                inner_action |= self.send_contact_request();
                            }

                            // Show retry button for recoverable errors
                            if let ContactRequestStatus::Error(ref err) = self.status
                                && err.is_recoverable()
                            {
                                ui.add_space(10.0);
                                if ui.button("Retry").clicked() {
                                    // Clear both status and message before retrying
                                    self.status = ContactRequestStatus::NotStarted;
                                    self.message = None;
                                    inner_action |= self.send_contact_request();
                                }
                            }
                        });
                    }
                });
            });

            inner_action
        });

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut popup =
                        InfoPopup::new("About Contact Requests", CONTACT_REQUEST_INFO_TEXT);
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open() {
            if let Some(wallet) = &self.selected_wallet {
                let result = self.wallet_unlock_popup.show(ctx, wallet, &self.app_context);
                if result == WalletUnlockResult::Unlocked {
                    // Wallet unlocked successfully, UI will update on next frame
                }
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
        if message_type == MessageType::Error {
            let error = DashPayError::Internal {
                message: message.to_string(),
            };
            self.status = ContactRequestStatus::Error(error);
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
                    // Try to parse structured error, fallback to generic
                    let error = if message.contains("not found") && message.contains("username") {
                        DashPayError::UsernameResolutionFailed {
                            username: self.username_or_id.clone(),
                        }
                    } else if message.contains("Identity not found") {
                        DashPayError::IdentityNotFound {
                            identity_id: dash_sdk::platform::Identifier::from_string(
                                &self.username_or_id,
                                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                            )
                            .unwrap_or_else(|_| dash_sdk::platform::Identifier::random()),
                        }
                    } else if message.contains("Network") || message.contains("connection") {
                        DashPayError::NetworkError {
                            reason: message.clone(),
                        }
                    } else {
                        DashPayError::Internal {
                            message: message.clone(),
                        }
                    };

                    self.status = ContactRequestStatus::Error(error.clone());
                    // Don't set message field to avoid duplicate error display
                    self.message = None;
                } else {
                    self.status = ContactRequestStatus::NotStarted;
                    self.display_message(&message, MessageType::Info);
                }
            }
            _ => {
                self.status =
                    ContactRequestStatus::Success("Contact request sent successfully!".to_string());
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
