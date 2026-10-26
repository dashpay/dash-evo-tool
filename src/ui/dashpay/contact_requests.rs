use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dashpay::{ContactRequestKey, DashPaySnapshot, PendingContactRequest};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{MessageBanner, ResultBannerExt};
use crate::ui::identity::get_selected_wallet;
use crate::ui::identity::keys::add_key_screen::AddKeyScreen;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::{MessageType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{Frame, Margin, RichText, ScrollArea, Ui};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

use super::format_relative_time;

#[derive(Debug, Clone)]
pub struct ContactRequest {
    /// The selected identity and the other identity of the request.
    pub request: ContactRequestKey,
    pub from_identity: Identifier,
    pub to_identity: Identifier,
    pub from_username: Option<String>,
    pub from_display_name: Option<String>,
    /// Username of the recipient (used for outgoing requests)
    pub to_username: Option<String>,
    /// Display name of the recipient (used for outgoing requests)
    pub to_display_name: Option<String>,
    pub account_reference: u32,
    pub account_label: Option<String>,
    pub timestamp: u64,
    pub auto_accept_proof: Option<Vec<u8>>,
}

impl ContactRequest {
    /// `owner`'s pending request with `pending.counterparty_id`, received when
    /// `incoming`, sent otherwise.
    fn from_pending(owner: Identifier, pending: &PendingContactRequest, incoming: bool) -> Self {
        let counterparty = pending.counterparty_id;
        let (from_identity, to_identity) = if incoming {
            (counterparty, owner)
        } else {
            (owner, counterparty)
        };
        let (from_username, from_display_name, to_username, to_display_name) = if incoming {
            (
                pending.username.clone(),
                pending.display_name.clone(),
                None,
                None,
            )
        } else {
            (
                None,
                None,
                pending.username.clone(),
                pending.display_name.clone(),
            )
        };
        Self {
            request: ContactRequestKey {
                owner,
                counterparty,
            },
            from_identity,
            to_identity,
            from_username,
            from_display_name,
            to_username,
            to_display_name,
            account_reference: pending.account_reference,
            account_label: None,
            timestamp: pending.created_at_ms,
            auto_accept_proof: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RequestTab {
    Incoming,
    Outgoing,
}

pub struct ContactRequests {
    pub app_context: Arc<AppContext>,
    incoming_requests: BTreeMap<Identifier, ContactRequest>,
    outgoing_requests: BTreeMap<Identifier, ContactRequest>,
    /// Counterparties whose request was accepted or rejected here.
    accepted_requests: HashSet<Identifier>,
    rejected_requests: HashSet<Identifier>,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    active_tab: RequestTab,
    loading: bool,
    request_in_flight: Option<ContactRequestKey>,
    request_task_in_flight: Option<DashPayTask>,
    /// The snapshot the lists were built from.
    applied: Option<Arc<DashPaySnapshot>>,
    accept_confirmation_dialog: Option<(ConfirmationDialog, ContactRequest)>,
    reject_confirmation_dialog: Option<(ConfirmationDialog, ContactRequest)>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    pub wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    /// Structured error for displaying with action buttons
    error: Option<DashPayError>,
}

impl ContactRequests {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let mut new_self = Self {
            app_context: app_context.clone(),
            incoming_requests: BTreeMap::new(),
            outgoing_requests: BTreeMap::new(),
            accepted_requests: HashSet::new(),
            rejected_requests: HashSet::new(),
            selected_identity: None,
            selected_identity_string: String::new(),
            active_tab: RequestTab::Incoming,
            loading: false,
            request_in_flight: None,
            request_task_in_flight: None,
            applied: None,
            accept_confirmation_dialog: None,
            reject_confirmation_dialog: None,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            error: None,
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
            new_self.selected_identity_string = preferred
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

            // Get wallet for the selected identity
            new_self.selected_wallet = get_selected_wallet(&preferred, Some(&app_context), None)
                .or_show_error(app_context.egui_ctx())
                .unwrap_or(None);
        }

        new_self
    }

    /// Set the selected identity from an external source (e.g., when embedded in ContactsList)
    pub fn set_selected_identity(&mut self, identity: Option<QualifiedIdentity>) {
        let identity_changed = match (&self.selected_identity, &identity) {
            (Some(current), Some(new)) => current.identity.id() != new.identity.id(),
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if identity_changed {
            self.selected_identity = identity.clone();
            if let Some(id) = &identity {
                self.selected_identity_string = id
                    .identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

                // Update wallet for the newly selected identity
                self.selected_wallet = get_selected_wallet(id, Some(&self.app_context), None)
                    .or_show_error(self.app_context.egui_ctx())
                    .unwrap_or(None);
                self.wallet_open_attempted = false;
            } else {
                self.selected_identity_string.clear();
                self.selected_wallet = None;
                self.wallet_open_attempted = false;
            }

            // Clear the requests when identity changes; the next render
            // rebuilds them from the new identity's snapshot.
            self.forget_requests();
            self.request_in_flight = None;
            self.request_task_in_flight = None;
            self.loading = false;
        }
    }

    /// Render without the header and identity selector (for use when embedded in another component)
    pub fn render_embedded(&mut self, ui: &mut Ui) -> AppAction {
        self.render_content(ui, false)
    }

    /// Returns the count of pending incoming requests (not yet accepted or rejected)
    pub fn pending_incoming_count(&self) -> usize {
        self.incoming_requests
            .keys()
            .filter(|id| {
                !self.accepted_requests.contains(*id) && !self.rejected_requests.contains(*id)
            })
            .count()
    }

    /// Drop the lists, to be rebuilt from the selected identity's snapshot.
    fn forget_requests(&mut self) {
        self.incoming_requests.clear();
        self.outgoing_requests.clear();
        self.applied = None;
    }

    /// Rebuild the lists from the selected identity's DashPay snapshot when
    /// it is not the one they were built from.
    fn apply_snapshot(&mut self) {
        let Some(owner) = self.selected_identity.as_ref().map(|i| i.identity.id()) else {
            return;
        };
        let Some(snapshot) = self.app_context.dashpay_snapshot(&owner) else {
            return;
        };
        if self
            .applied
            .as_ref()
            .is_some_and(|applied| Arc::ptr_eq(applied, &snapshot))
        {
            return;
        }
        self.incoming_requests = snapshot
            .incoming
            .iter()
            .map(|pending| {
                (
                    pending.counterparty_id,
                    ContactRequest::from_pending(owner, pending, true),
                )
            })
            .collect();
        self.outgoing_requests = snapshot
            .outgoing
            .iter()
            .map(|pending| {
                (
                    pending.counterparty_id,
                    ContactRequest::from_pending(owner, pending, false),
                )
            })
            .collect();
        self.applied = Some(snapshot);
    }

    pub fn refresh(&mut self) -> AppAction {
        // Don't clear requests - preserve loaded state
        // Only clear temporary states
        if self.request_in_flight.is_none() {
            self.loading = false;
        }

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
            self.selected_identity_string = preferred
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
        }

        self.applied = None;

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        self.render_content(ui, true)
    }

    fn render_content(&mut self, ui: &mut Ui, show_header: bool) -> AppAction {
        let mut action = AppAction::None;

        self.apply_snapshot();

        // Handle accept confirmation dialog
        if let Some((dialog, request)) = &mut self.accept_confirmation_dialog {
            let response = dialog.show(ui);
            if response.inner.dialog_response == Some(ConfirmationStatus::Confirmed) {
                if self
                    .app_context
                    .contact_request_action_is_in_flight(&request.request)
                {
                    MessageBanner::set_global(
                        ui.ctx(),
                        "This contact request action is already running.",
                        MessageType::Info,
                    );
                } else if let Some(identity) = &self.selected_identity {
                    // Don't mark as accepted yet - wait for backend confirmation
                    self.loading = true;
                    self.request_in_flight = Some(request.request);

                    let request_task = DashPayTask::AcceptContactRequest {
                        identity: identity.clone(),
                        counterparty_id: request.request.counterparty,
                    };
                    self.request_task_in_flight = Some(request_task.clone());
                    let task = BackendTask::DashPayTask(Box::new(request_task));

                    action |= AppAction::BackendTask(task);
                }
                self.accept_confirmation_dialog = None;
            } else if response.inner.dialog_response == Some(ConfirmationStatus::Canceled) {
                self.accept_confirmation_dialog = None;
            }
        }

        // Handle reject confirmation dialog
        if let Some((dialog, request)) = &mut self.reject_confirmation_dialog {
            let response = dialog.show(ui);
            if response.inner.dialog_response == Some(ConfirmationStatus::Confirmed) {
                if self
                    .app_context
                    .contact_request_action_is_in_flight(&request.request)
                {
                    MessageBanner::set_global(
                        ui.ctx(),
                        "This contact request action is already running.",
                        MessageType::Info,
                    );
                } else if let Some(identity) = &self.selected_identity {
                    self.loading = true;
                    self.request_in_flight = Some(request.request);

                    // Don't mark as rejected yet - wait for backend confirmation

                    let request_task = DashPayTask::RejectContactRequest {
                        identity: identity.clone(),
                        counterparty_id: request.request.counterparty,
                    };
                    self.request_task_in_flight = Some(request_task.clone());
                    let task = BackendTask::DashPayTask(Box::new(request_task));

                    action |= AppAction::BackendTask(task);
                }
                self.reject_confirmation_dialog = None;
            } else if response.inner.dialog_response == Some(ConfirmationStatus::Canceled) {
                self.reject_confirmation_dialog = None;
            }
        }

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();

        // Header with identity selector on the right (only shown when not embedded)
        if show_header {
            ui.horizontal(|ui| {
                ui.heading("Contact Requests");

                if !identities.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // SYNC: write-back via syncing_global on user pick (FR-6: the source list is
                        // User-only, so a masternode/evonode can never leak to the app-global identity).
                        let response = ui.add(
                            IdentitySelector::new(
                                "requests_identity_selector",
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
                            // Clear the requests when identity changes
                            self.forget_requests();

                            // Update wallet for the newly selected identity
                            if let Some(identity) = &self.selected_identity {
                                self.selected_wallet =
                                    get_selected_wallet(identity, Some(&self.app_context), None)
                                        .or_show_error(self.app_context.egui_ctx())
                                        .unwrap_or(None);
                            } else {
                                self.selected_wallet = None;
                            }
                            self.wallet_open_attempted = false;
                        }
                    });
                }
            });

            ui.separator();

            if identities.is_empty() {
                return super::render_no_identities_card(ui, &self.app_context);
            }
        }

        // Show structured error with action buttons if any
        let mut dismiss_error = false;
        if let Some(err) = &self.error {
            let dark_mode = ui.style().visuals.dark_mode;
            let error_color = if dark_mode {
                DashColors::ERROR
            } else {
                egui::Color32::DARK_RED
            };
            let error_msg = err.to_string();
            let is_missing_encryption_key = matches!(err, DashPayError::MissingEncryptionKey);

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(error_msg).color(error_color));

                    // Show action button for missing encryption key
                    if is_missing_encryption_key {
                        ui.add_space(5.0);
                        if let Some(identity) = &self.selected_identity
                            && ui.button("Add Encryption Key").clicked()
                        {
                            action = AppAction::AddScreen(Screen::AddKeyScreen(
                                AddKeyScreen::new_for_dashpay_encryption(
                                    identity.clone(),
                                    &self.app_context,
                                ),
                            ));
                            dismiss_error = true;
                        }
                    }
                });
            });
            ui.separator();
        }
        if dismiss_error {
            self.error = None;
        }

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view contact requests");
            return action;
        }

        // Tabs
        let dark_mode = ui.style().visuals.dark_mode;
        ui.horizontal(|ui| {
            let incoming_response = if self.active_tab == RequestTab::Incoming {
                ComponentStyles::add_primary_button(ui, "Incoming")
            } else {
                ComponentStyles::add_secondary_button(ui, "Incoming", dark_mode)
            };
            if incoming_response.clicked() {
                self.active_tab = RequestTab::Incoming;
            }

            ui.add_space(8.0);

            let outgoing_response = if self.active_tab == RequestTab::Outgoing {
                ComponentStyles::add_primary_button(ui, "Outgoing")
            } else {
                ComponentStyles::add_secondary_button(ui, "Outgoing", dark_mode)
            };
            if outgoing_response.clicked() {
                self.active_tab = RequestTab::Outgoing;
            }
        });

        ui.add_space(8.0);

        // Display requests based on active tab
        match self.active_tab {
            RequestTab::Incoming => {
                // Loading indicator
                if self.loading {
                    ui.horizontal(|ui| {
                        ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                        ui.label("Loading...");
                    });
                } else {
                    ScrollArea::vertical().id_salt("incoming_requests_scroll").show(ui, |ui| {
                    if self.incoming_requests.is_empty() {
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
                                        RichText::new("No Incoming Requests")
                                            .strong()
                                            .size(20.0)
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    ui.add_space(5.0);
                                    ui.label(
                                        RichText::new("You don't have any pending contact requests.")
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                    ui.add_space(10.0);
                                });
                            });
                    } else {
                        let requests: Vec<_> = self.incoming_requests.values().cloned().collect();
                        for request in requests {
                            ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                    // Avatar placeholder
                                    ui.add(egui::Label::new(RichText::new("👤").size(30.0).color(DashColors::DEEP_BLUE)));

                                    ui.vertical(|ui| {
                                        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
                                        let dark_mode = ui.style().visuals.dark_mode;

                                        // Display name or username or identity ID
                                        let name = request
                                            .from_display_name
                                            .as_ref()
                                            .or(request.from_username.as_ref()).cloned()
                                            .unwrap_or_else(|| {
                                                // Show truncated identity ID if no name available
                                                let id_str = request.from_identity.to_string(Encoding::Base58);
                                                format!("{}...{}", &id_str[..6], &id_str[id_str.len()-6..])
                                            });

                                        ui.label(RichText::new(name).strong().color(DashColors::text_primary(dark_mode)));

                                        // Username or identity ID
                                        if let Some(username) = &request.from_username {
                                            ui.label(
                                                RichText::new(format!("@{}", username)).small().color(DashColors::text_secondary(dark_mode)),
                                            );
                                        } else {
                                            // Show full identity ID
                                            ui.label(
                                                RichText::new(format!("ID: {}", request.from_identity.to_string(Encoding::Base58)))
                                                    .small()
                                                    .color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }

                                        // Account label
                                        if let Some(label) = &request.account_label {
                                            ui.label(
                                                RichText::new(format!("Account: {}", label))
                                                    .small()
                                                    .color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }

                                        // Timestamp
                                        let time_text = format_relative_time(request.timestamp)
                                            .map(|t| format!("Received: {}", t))
                                            .unwrap_or_else(|| "Received: unknown".to_string());
                                        ui.label(
                                            RichText::new(time_text).small().color(DashColors::text_secondary(dark_mode)),
                                        );
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // Check if this request has been accepted or rejected
                                            if self.accepted_requests.contains(&request.request.counterparty) {
                                                // Show checkmark and "Accepted" text
                                                ui.label(
                                                    RichText::new("Accepted")
                                                        .color(egui::Color32::from_rgb(0, 150, 0))
                                                        .strong()
                                                );
                                            } else if self.rejected_requests.contains(&request.request.counterparty) {
                                                // Show X and "Rejected" text
                                                ui.label(
                                                    RichText::new("Rejected")
                                                        .color(egui::Color32::from_rgb(150, 0, 0))
                                                        .strong()
                                                );
                                            } else {
                                                // Check wallet lock status before showing buttons
                                                let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                                                    if !self.wallet_open_attempted {
                                                        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                                                            crate::ui::components::MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                                                                .disable_auto_dismiss();
                                                        }
                                                        self.wallet_open_attempted = true;
                                                    }
                                                    wallet_needs_unlock(wallet)
                                                } else {
                                                    false
                                                };

                                                if wallet_locked {
                                                    if ui.button("Unlock Wallet").clicked() {
                                                        self.wallet_unlock_popup.open();
                                                    }
                                                } else {
                                                    // Show Accept/Reject buttons
                                                    if ui.button("Reject").clicked() {
                                                        // Show confirmation dialog for reject
                                                        let name = request.from_display_name.as_ref()
                                                            .or(request.from_username.as_ref())
                                                            .cloned()
                                                            .unwrap_or_else(|| {
                                                                let id_str = request.from_identity.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
                                                                format!("{}...{}", &id_str[..6], &id_str[id_str.len()-6..])
                                                            });

                                                        self.reject_confirmation_dialog = Some((
                                                            ConfirmationDialog::new(
                                                                "Reject Contact Request",
                                                                format!("Are you sure you want to reject the contact request from {}?", name)
                                                            )
                                                            .confirm_text(Some("Reject"))
                                                            .cancel_text(Some("Cancel"))
                                                            .danger_mode(true),
                                                            request.clone()
                                                        ));
                                                    }

                                                    if ui.button("Accept").clicked() {
                                                        // Show confirmation dialog for accept
                                                        let name = request.from_display_name.as_ref()
                                                            .or(request.from_username.as_ref())
                                                            .cloned()
                                                            .unwrap_or_else(|| {
                                                                let id_str = request.from_identity.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
                                                                format!("{}...{}", &id_str[..6], &id_str[id_str.len()-6..])
                                                            });

                                                        self.accept_confirmation_dialog = Some((
                                                            ConfirmationDialog::new(
                                                                "Accept Contact Request",
                                                                format!("Are you sure you want to accept the contact request from {}?", name)
                                                            )
                                                            .confirm_text(Some("Accept"))
                                                            .cancel_text(Some("Cancel")),
                                                            request.clone()
                                                        ));
                                                    }
                                                }
                                            }
                                        },
                                    );
                                });
                            });
                            ui.add_space(4.0);
                        }
                    }
                });
                }
            }
            RequestTab::Outgoing => {
                // Loading indicator
                if self.loading {
                    ui.horizontal(|ui| {
                        ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                        ui.label("Loading...");
                    });
                } else {
                    ScrollArea::vertical().id_salt("outgoing_requests_scroll").show(ui, |ui| {
                    if self.outgoing_requests.is_empty() {
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
                                        RichText::new("No Outgoing Requests")
                                            .strong()
                                            .size(20.0)
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    ui.add_space(5.0);
                                    ui.label(
                                        RichText::new("You haven't sent any contact requests.")
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                    ui.add_space(15.0);
                                    let add_button = egui::Button::new(
                                        RichText::new("Add Contact").color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(0, 141, 228));
                                    if ui.add(add_button).clicked() {
                                        action = AppAction::AddScreen(
                                            ScreenType::DashPayAddContact.create_screen(&self.app_context),
                                        );
                                    }
                                    ui.add_space(10.0);
                                });
                            });
                    } else {
                        let requests: Vec<_> = self.outgoing_requests.values().cloned().collect();
                        for request in requests {
                            ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                    // Avatar placeholder
                                    ui.add(egui::Label::new(RichText::new("👤").size(30.0).color(DashColors::DEEP_BLUE)));

                                    ui.vertical(|ui| {
                                        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
                                        let dark_mode = ui.style().visuals.dark_mode;

                                        // For outgoing requests, show display name or username or truncated ID
                                        let id_str = request.to_identity.to_string(Encoding::Base58);
                                        let name = request
                                            .to_display_name
                                            .as_ref()
                                            .or(request.to_username.as_ref())
                                            .cloned()
                                            .unwrap_or_else(|| {
                                                format!("{}...{}", &id_str[..6], &id_str[id_str.len()-6..])
                                            });

                                        ui.label(RichText::new(format!("To: {}", name)).strong().color(DashColors::text_primary(dark_mode)));

                                        // Show username if display name is shown
                                        if let Some(username) = &request.to_username
                                            && request.to_display_name.is_some()
                                        {
                                            ui.label(
                                                RichText::new(format!("@{}", username)).small().color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }

                                        // Show identity ID
                                        ui.label(
                                            RichText::new(format!("ID: {}", id_str))
                                                .small()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );

                                        // Account label
                                        if let Some(label) = &request.account_label {
                                            ui.label(
                                                RichText::new(format!("Account: {}", label))
                                                    .small()
                                                    .color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }

                                        // Status
                                        ui.label(RichText::new("Status: Pending").small().color(DashColors::text_secondary(dark_mode)));
                                        let sent_time_text = format_relative_time(request.timestamp)
                                            .map(|t| format!("Sent: {}", t))
                                            .unwrap_or_else(|| "Sent: unknown".to_string());
                                        ui.label(RichText::new(sent_time_text).small().color(DashColors::text_secondary(dark_mode)));
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let dark_mode = ui.style().visuals.dark_mode;
                                            ui.label(
                                                RichText::new("Cannot be cancelled once sent")
                                                    .small()
                                                    .italics()
                                                    .color(DashColors::text_secondary(dark_mode)),
                                            );
                                        },
                                    );
                                });
                            });
                            ui.add_space(4.0);
                        }
                    }
                });
                }
            }
        }

        action
    }
}

impl ScreenLike for ContactRequests {
    fn refresh_on_arrival(&mut self) {
        self.applied = None;
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // Create a simple central panel for rendering
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.render(ui);
        });

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
        // Banner display is handled globally by AppState; this is only for side-effects.
        if self.request_in_flight.is_none() {
            self.loading = false;
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        let matches_request =
            request_error_id(error).is_some_and(|id| self.request_in_flight.as_ref() == Some(id));
        if matches_request {
            self.request_in_flight = None;
            self.request_task_in_flight = None;
            self.loading = false;
        } else if self.request_in_flight.is_none() {
            self.loading = false;
        }
        match classify_request_error(error) {
            Some(dashpay_error) => {
                self.error = Some(dashpay_error);
                true
            }
            None => false,
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        let completed_request = match &result {
            BackendTaskSuccessResult::DashPayContactRequestAccepted(request)
            | BackendTaskSuccessResult::DashPayContactRequestRejected(request)
            | BackendTaskSuccessResult::DashPayContactAlreadyEstablished(request) => {
                self.request_in_flight.as_ref() == Some(request)
            }
            _ => false,
        };
        if completed_request {
            self.request_in_flight = None;
            self.request_task_in_flight = None;
        }
        if self.request_in_flight.is_none() {
            self.loading = false;
        }

        match result {
            BackendTaskSuccessResult::DashPayContactRequestAccepted(request) => {
                // Mark as accepted only after successful backend operation
                self.accepted_requests.insert(request.counterparty);
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Contact request accepted successfully",
                    MessageType::Success,
                );
            }
            BackendTaskSuccessResult::DashPayContactRequestRejected(request) => {
                // Mark as rejected only after successful backend operation
                self.rejected_requests.insert(request.counterparty);
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Contact request rejected",
                    MessageType::Success,
                );
            }
            BackendTaskSuccessResult::DashPayContactAlreadyEstablished(_) => {
                // Message display is handled globally by AppState
            }
            _ => {
                // Ignore other results
            }
        }
    }
}

fn request_error_id(error: &TaskError) -> Option<&ContactRequestKey> {
    match error {
        TaskError::DashPayContactRequestActionFailed { request, .. } => Some(request),
        _ => None,
    }
}

/// Map a typed accept/reject error onto the screen-local error category that
/// drives a dedicated affordance (the "Add Encryption Key" button). Returns
/// `None` when no request-specific UI applies, leaving the global banner to
/// report the error.
fn classify_request_error(error: &TaskError) -> Option<DashPayError> {
    match error {
        TaskError::DashPayContactRequestActionFailed { source, .. } => {
            classify_request_error(source)
        }
        TaskError::DashPay(DashPayError::MissingEncryptionKey) => {
            Some(DashPayError::MissingEncryptionKey)
        }
        TaskError::DashPay(DashPayError::RecipientMissingDecryptionKey) => {
            Some(DashPayError::RecipientMissingDecryptionKey)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_missing_encryption_key() {
        let mapped =
            classify_request_error(&TaskError::DashPay(DashPayError::MissingEncryptionKey));
        assert!(matches!(mapped, Some(DashPayError::MissingEncryptionKey)));
    }

    #[test]
    fn classifies_recipient_missing_decryption_key() {
        let mapped = classify_request_error(&TaskError::DashPay(
            DashPayError::RecipientMissingDecryptionKey,
        ));
        assert!(matches!(
            mapped,
            Some(DashPayError::RecipientMissingDecryptionKey)
        ));
    }

    #[test]
    fn classifies_a_missing_key_carried_by_a_request_action_failure() {
        let mapped = classify_request_error(&TaskError::DashPayContactRequestActionFailed {
            request: ContactRequestKey {
                owner: Identifier::from([1; 32]),
                counterparty: Identifier::from([2; 32]),
            },
            source: Box::new(TaskError::DashPay(DashPayError::MissingEncryptionKey)),
        });
        assert!(
            matches!(mapped, Some(DashPayError::MissingEncryptionKey)),
            "wrapping an accept failure must not cost the user the Add Encryption Key button"
        );
    }

    #[test]
    fn unrelated_dashpay_error_defers_to_global_banner() {
        let mapped =
            classify_request_error(&TaskError::DashPay(DashPayError::MissingAuthenticationKey));
        assert!(mapped.is_none());
    }

    #[test]
    fn unrelated_task_error_defers_to_global_banner() {
        let mapped = classify_request_error(&TaskError::DocumentNotFound);
        assert!(mapped.is_none());
    }

    #[test]
    fn request_error_id_correlates_only_the_matching_paid_action() {
        let request = ContactRequestKey {
            owner: Identifier::from([1; 32]),
            counterparty: Identifier::from([2; 32]),
        };
        let error = TaskError::DashPayContactRequestActionFailed {
            request,
            source: Box::new(TaskError::DocumentNotFound),
        };

        assert_eq!(request_error_id(&error), Some(&request));
        assert_eq!(request_error_id(&TaskError::DocumentNotFound), None);
    }
}
