use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dashpay::validate_account_label;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::ResultBannerExt;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::dashpay::DashPaySubscreen;
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::theme::{DashColors, Typography};
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::{Arc, RwLock};

const CONTACT_REQUEST_INFO_TEXT: &str = "About Contact Requests:\n\n\
    Contact requests establish secure communication channels.\n\n\
    Both parties must accept before payments can be sent.\n\n\
    Your display name and username will be shared with the contact.\n\n\
    You can manage contacts from the Contacts screen.";

#[derive(Debug)]
enum ContactRequestStatus {
    NotStarted,
    Sending,
    Success,
    Error(DashPayError), // Structured error with user-friendly messaging
}

pub struct AddContactScreen {
    pub app_context: Arc<AppContext>,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    selected_key: Option<IdentityPublicKey>,
    username_or_id: String,
    account_label: String,
    status: ContactRequestStatus,
    show_info_popup: bool,
    show_advanced_options: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
}

impl AddContactScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        let identities = app_context.load_local_user_identities().unwrap_or_default();
        let selected_identity = app_context
            .selected_identity_id()
            .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
            .or_else(|| identities.first().cloned());
        let selected_identity_string = selected_identity
            .as_ref()
            .map(|qi| {
                qi.identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            })
            .unwrap_or_default();

        Self {
            app_context,
            selected_identity,
            selected_identity_string,
            selected_key: None,
            username_or_id: String::new(),
            account_label: String::new(),
            status: ContactRequestStatus::NotStarted,
            show_info_popup: false,
            show_advanced_options: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
        }
    }

    pub fn new_with_identity_id(app_context: Arc<AppContext>, identity_id: String) -> Self {
        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        let identities = app_context.load_local_user_identities().unwrap_or_default();
        let selected_identity = app_context
            .selected_identity_id()
            .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
            .or_else(|| identities.first().cloned());
        let selected_identity_string = selected_identity
            .as_ref()
            .map(|qi| {
                qi.identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            })
            .unwrap_or_default();

        Self {
            app_context,
            selected_identity,
            selected_identity_string,
            selected_key: None,
            username_or_id: identity_id,
            account_label: String::new(),
            status: ContactRequestStatus::NotStarted,
            show_info_popup: false,
            show_advanced_options: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
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
                self.status = ContactRequestStatus::Error(error);
                return AppAction::None;
            }

            // Validate username format if it looks like a username
            if crate::model::dpns::validate_dpns_input(&self.username_or_id).is_err() {
                self.status = ContactRequestStatus::Error(DashPayError::InvalidUsername {
                    username: self.username_or_id.trim().to_string(),
                });
                return AppAction::None;
            }

            // Validate account label length
            if let Err(error) = validate_account_label(&self.account_label) {
                let error = DashPayError::AccountLabelTooLong {
                    length: error.actual,
                    max: error.max,
                };
                self.status = ContactRequestStatus::Error(error);
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
            self.status = ContactRequestStatus::Error(error);
            AppAction::None
        }
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let action = crate::ui::helpers::show_success_screen(
            ui,
            "Contact Request Sent Successfully!".to_string(),
            vec![
                (
                    "Send Another Request".to_string(),
                    AppAction::Custom("send_another".to_string()),
                ),
                (
                    "Back to Contacts".to_string(),
                    AppAction::PopScreenAndRefresh,
                ),
                ("Back to DashPay".to_string(), AppAction::PopScreen),
            ],
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "send_another"
        {
            self.status = ContactRequestStatus::NotStarted;
            self.selected_key = None;
            return AppAction::Refresh;
        }

        action
    }
}

impl ScreenLike for AddContactScreen {
    fn refresh(&mut self) {
        // Don't reset success status on refresh
        if !matches!(self.status, ContactRequestStatus::Success) {
            self.status = ContactRequestStatus::NotStarted;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // Add top panel with navigation breadcrumbs
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Add Contact", AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ui, &self.app_context, DashPaySubscreen::Contacts);

        // Main content in island central panel
        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;

            // Show success screen if request was successful
            if matches!(self.status, ContactRequestStatus::Success) {
                return self.show_success_screen(ui);
            }

            // Header with Back button, info icon, and Advanced Options checkbox
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    inner_action = AppAction::PopScreen;
                }
                ui.heading("Add Contact");
                ui.add_space(5.0);
                if crate::ui::helpers::info_icon_button(ui, CONTACT_REQUEST_INFO_TEXT).clicked() {
                    self.show_info_popup = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                });
            });
            ui.separator();

            // Identity and Key selector
            let identities = self
                .app_context
                .load_local_user_identities()
                .unwrap_or_default();

            if identities.is_empty() {
                inner_action |= super::render_no_identities_card(ui, &self.app_context);
                return inner_action;
            }

            ui.group(|ui| {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(
                    RichText::new("From (Sender)")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.separator();

                // Identity selector — SYNC: write-back via syncing_global on user pick (FR-6:
                // User-only source, so no masternode can leak to the app-global identity).
                let response = ui.add(
                    IdentitySelector::new(
                        "contact_sender_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .width(300.0)
                    .label("Identity:")
                    .other_option(false)
                    .syncing_global(self.app_context.clone()),
                );

                // Handle identity change - auto-select key and update wallet
                // Also auto-select if we have an identity but no key (e.g., on initial load)
                let should_auto_select = response.changed()
                    || (self.selected_identity.is_some() && self.selected_key.is_none());

                if should_auto_select {
                    if let Some(identity) = &self.selected_identity {
                        // Auto-select a suitable AUTHENTICATION key for signing contact requests
                        // Platform requires CRITICAL or HIGH security level for contact request signing
                        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                        use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
                        use std::collections::HashSet;
                        self.selected_key = identity
                            .identity
                            .get_first_public_key_matching(
                                Purpose::AUTHENTICATION,
                                HashSet::from([SecurityLevel::CRITICAL, SecurityLevel::HIGH]),
                                KeyType::all_key_types().into(),
                                false,
                            )
                            .cloned();

                        // Update wallet if not already set
                        if self.selected_wallet.is_none() {
                            self.selected_wallet =
                                get_selected_wallet(identity, Some(&self.app_context), None)
                                    .or_show_error(self.app_context.egui_ctx())
                                    .unwrap_or(None);
                            self.wallet_open_attempted = false;
                        }
                    } else {
                        self.selected_key = None;
                        self.selected_wallet = None;
                        self.wallet_open_attempted = false;
                    }
                }

                // Key selector (only shown in advanced mode)
                if self.show_advanced_options {
                    ui.add_space(10.0);
                    if let Some(identity) = &self.selected_identity {
                        let key_action = add_key_chooser(
                            ui,
                            &self.app_context,
                            identity,
                            &mut self.selected_key,
                            TransactionType::ContactRequest,
                        );
                        if !matches!(key_action, AppAction::None) {
                            inner_action = key_action;
                        }
                    }
                }
            });

            ui.add_space(10.0);

            // Loading indicator
            if matches!(self.status, ContactRequestStatus::Sending) {
                ui.horizontal(|ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
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
                let dark_mode = ui.style().visuals.dark_mode;
                let error_color = if dark_mode {
                    DashColors::ERROR
                } else {
                    egui::Color32::DARK_RED
                };

                ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(err.to_string()).color(error_color));

                        // Show retry suggestion for recoverable errors
                        if err.is_recoverable() {
                            ui.label(RichText::new("You can try again.").font(Typography::hint()).color(DashColors::text_secondary(dark_mode)));
                        }

                        // Show action suggestion for user errors
                        if err.requires_user_action() {
                            match err {
                                DashPayError::UsernameResolutionFailed { .. } => {
                                    ui.label(RichText::new("Tip: Make sure the username is spelled correctly and exists on Dash Platform.").font(Typography::hint()).color(DashColors::text_secondary(dark_mode)));
                                }
                                DashPayError::InvalidUsername { .. } => {
                                    ui.label(RichText::new("Tip: Usernames must end with '.dash' (e.g., alice).").font(Typography::hint()).color(DashColors::text_secondary(dark_mode)));
                                }
                                DashPayError::AccountLabelTooLong { .. } => {
                                    ui.label(RichText::new("Tip: Try a shorter, more descriptive label.").font(Typography::hint()).color(DashColors::text_secondary(dark_mode)));
                                }
                                DashPayError::MissingEncryptionKey => {
                                    ui.add_space(5.0);
                                    if let Some(identity) = &self.selected_identity
                                        && ui.button("Add Encryption Key").clicked() {
                                            inner_action = AppAction::AddScreen(Screen::AddKeyScreen(
                                                AddKeyScreen::new_for_dashpay_encryption(
                                                    identity.clone(),
                                                    &self.app_context,
                                                ),
                                            ));
                                        }
                                }
                                // Note: `RecipientMissingDecryptionKey` is a
                                // recipient-side problem — the sender cannot fix it
                                // by adding a key to their own identity, so no
                                // self-remedy button is offered. The error message
                                // already tells the user what to do.
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
                    let dark_mode = ui.style().visuals.dark_mode;
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
                        let dark_mode = ui.style().visuals.dark_mode;
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
                    let _dark_mode = ui.style().visuals.dark_mode;

                    // Check wallet lock status before showing send button
                    let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                        if !self.wallet_open_attempted {
                            if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                                crate::ui::components::MessageBanner::set_global(
                                    ui.ctx(),
                                    &e,
                                    MessageType::Error,
                                )
                                .disable_auto_dismiss();
                            }
                            self.wallet_open_attempted = true;
                        }
                        wallet_needs_unlock(wallet)
                    } else {
                        false
                    };

                    if wallet_locked {
                        ui.add_space(10.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to add contact.",
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
                                RichText::new("Add Contact").color(egui::Color32::WHITE),
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
                                    // Clear status before retrying
                                    self.status = ContactRequestStatus::NotStarted;
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
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("dashpay_add_contact_info_popup"),
                        "About Contact Requests",
                        CONTACT_REQUEST_INFO_TEXT,
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
                // Wallet unlocked successfully, UI will update on next frame
            }
        }

        action
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::DashPayContactRequestSent(_recipient) = result {
            self.status = ContactRequestStatus::Success;
            self.username_or_id.clear();
            self.account_label.clear();
            self.selected_key = None;
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        match classify_send_error(error, &self.username_or_id) {
            Some(dashpay_error) => {
                self.status = ContactRequestStatus::Error(dashpay_error);
                true
            }
            None => {
                // No dedicated affordance: stop the spinner and let the global
                // banner report the error.
                if matches!(self.status, ContactRequestStatus::Sending) {
                    self.status = ContactRequestStatus::NotStarted;
                }
                false
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

/// Map a typed send-contact-request error onto the screen-local error category
/// that drives a dedicated affordance (key-add button, tip, retry). Returns
/// `None` when no add-contact-specific UI applies, leaving the global banner to
/// report the error. `username_or_id` is the current recipient input, used to
/// label an unresolved identity.
fn classify_send_error(error: &TaskError, username_or_id: &str) -> Option<DashPayError> {
    match error {
        TaskError::IdentityNotFound => Some(DashPayError::IdentityNotFound {
            identity_id: dash_sdk::platform::Identifier::from_string(
                username_or_id,
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
            )
            .ok()?,
        }),
        TaskError::DashPay(inner) => match inner {
            DashPayError::MissingEncryptionKey => Some(DashPayError::MissingEncryptionKey),
            DashPayError::RecipientMissingDecryptionKey => {
                Some(DashPayError::RecipientMissingDecryptionKey)
            }
            DashPayError::UsernameResolutionFailed { username } => {
                Some(DashPayError::UsernameResolutionFailed {
                    username: username.clone(),
                })
            }
            DashPayError::InvalidUsername { username } => Some(DashPayError::InvalidUsername {
                username: username.clone(),
            }),
            DashPayError::AccountLabelTooLong { length, max } => {
                Some(DashPayError::AccountLabelTooLong {
                    length: *length,
                    max: *max,
                })
            }
            DashPayError::CannotContactSelf => Some(DashPayError::CannotContactSelf),
            DashPayError::ContactRequestAlreadySent { to } => {
                Some(DashPayError::ContactRequestAlreadySent { to: to.clone() })
            }
            DashPayError::NetworkError => Some(DashPayError::NetworkError),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_missing_key_errors_for_add_key_affordance() {
        let enc = classify_send_error(
            &TaskError::DashPay(DashPayError::MissingEncryptionKey),
            "alice.dash",
        );
        assert!(matches!(enc, Some(DashPayError::MissingEncryptionKey)));

        // A recipient-side missing decryption key is classified so its
        // (recipient-attributed) message renders in-screen — but it carries no
        // sender self-remedy button, since the sender cannot fix it.
        let dec = classify_send_error(
            &TaskError::DashPay(DashPayError::RecipientMissingDecryptionKey),
            "alice.dash",
        );
        assert!(matches!(
            dec,
            Some(DashPayError::RecipientMissingDecryptionKey)
        ));
        assert!(
            !DashPayError::RecipientMissingDecryptionKey.requires_user_action(),
            "the sender has no self-remedy for a recipient-side missing key"
        );
    }

    #[test]
    fn classifies_username_resolution_failure_preserving_username() {
        let mapped = classify_send_error(
            &TaskError::DashPay(DashPayError::UsernameResolutionFailed {
                username: "bob.dash".to_string(),
            }),
            "bob.dash",
        );
        assert!(matches!(
            mapped,
            Some(DashPayError::UsernameResolutionFailed { username }) if username == "bob.dash"
        ));
    }

    #[test]
    fn recoverable_errors_map_through_so_retry_is_offered() {
        let mapped = classify_send_error(
            &TaskError::DashPay(DashPayError::NetworkError),
            "alice.dash",
        );
        let mapped = mapped.expect("network errors should be classified");
        assert!(mapped.is_recoverable());
    }

    #[test]
    fn identity_not_found_with_valid_base58_maps_to_typed_variant() {
        let id = dash_sdk::platform::Identifier::random()
            .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
        let mapped = classify_send_error(&TaskError::IdentityNotFound, &id);
        assert!(matches!(
            mapped,
            Some(DashPayError::IdentityNotFound { .. })
        ));
    }

    #[test]
    fn identity_not_found_with_invalid_base58_falls_back_to_global_banner() {
        let mapped = classify_send_error(&TaskError::IdentityNotFound, "not a valid id");
        assert!(mapped.is_none());
    }

    #[test]
    fn unrelated_errors_defer_to_global_banner() {
        let mapped = classify_send_error(
            &TaskError::EncryptionError {
                detail: "ecdh".to_string(),
            },
            "alice.dash",
        );
        assert!(mapped.is_none());
    }
}
