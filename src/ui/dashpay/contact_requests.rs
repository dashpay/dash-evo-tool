use crate::app::AppAction;
use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::dashpay::{DashPayResult, DashPayTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{Document, Identifier};
use egui::{Frame, Margin, RichText, ScrollArea, Ui};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct ContactRequest {
    pub request_id: Identifier,
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum RequestTab {
    Incoming,
    Outgoing,
}

pub struct ContactRequests {
    pub app_context: Arc<AppContext>,
    incoming_requests: BTreeMap<Identifier, ContactRequest>,
    outgoing_requests: BTreeMap<Identifier, ContactRequest>,
    accepted_requests: HashSet<Identifier>,
    rejected_requests: HashSet<Identifier>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    active_tab: RequestTab,
    message: Option<(String, MessageType)>,
    loading: bool,
    has_fetched_requests: bool,
    accept_confirmation_dialog: Option<(ConfirmationDialog, ContactRequest)>,
    reject_confirmation_dialog: Option<(ConfirmationDialog, ContactRequest)>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    pub wallet_unlock_popup: WalletUnlockPopup,
    /// Structured error for displaying with action buttons
    error: Option<DashPayError>,
    /// Identity IDs that need profile fetching from Platform
    pending_profile_fetches: HashSet<Identifier>,
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
            message: None,
            loading: false,
            has_fetched_requests: false,
            accept_confirmation_dialog: None,
            reject_confirmation_dialog: None,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error: None,
            pending_profile_fetches: HashSet::new(),
        };

        // Auto-select first identity on creation if available
        if let Ok(identities) = app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            new_self.selected_identity = Some(identities[0].clone());
            new_self.selected_identity_string = identities[0]
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

            // Get wallet for the selected identity
            let mut error_message = None;
            new_self.selected_wallet =
                get_selected_wallet(&identities[0], Some(&app_context), None, &mut error_message);

            // Load requests from database for this identity
            new_self.load_requests_from_database();
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
                let mut error_message = None;
                self.selected_wallet =
                    get_selected_wallet(id, Some(&self.app_context), None, &mut error_message);
            } else {
                self.selected_identity_string.clear();
                self.selected_wallet = None;
            }

            // Clear the requests when identity changes
            self.incoming_requests.clear();
            self.outgoing_requests.clear();
            self.message = None;
            self.has_fetched_requests = false;
            self.pending_profile_fetches.clear();

            // Load requests from database for the newly selected identity
            self.load_requests_from_database();
        }
    }

    /// Render without the header and identity selector (for use when embedded in another component)
    pub fn render_embedded(&mut self, ui: &mut Ui) -> AppAction {
        self.render_content(ui, false)
    }

    fn load_requests_from_database(&mut self) {
        // Load saved contact requests for the selected identity from database
        if let Some(identity) = &self.selected_identity {
            let identity_id = identity.identity.id();

            // Clear existing requests before loading
            self.incoming_requests.clear();
            self.outgoing_requests.clear();

            let network_str = self.app_context.network.to_string();
            tracing::debug!(
                "Loading contact requests from database for identity {} on network {}",
                identity_id,
                network_str
            );

            // Load pending incoming requests from database
            match self.app_context.db.load_pending_contact_requests(
                &identity_id,
                &network_str,
                "received",
            ) {
                Ok(incoming) => {
                    tracing::debug!("Loaded {} incoming requests from database", incoming.len());
                    for request in incoming {
                        if let Ok(from_id) = Identifier::from_bytes(&request.from_identity_id) {
                            let contact_request = ContactRequest {
                                request_id: Identifier::new([0; 32]), // We'll need to store this in DB
                                from_identity: from_id,
                                to_identity: identity_id,
                                from_username: request.to_username, // This field is misnamed in DB
                                from_display_name: None,
                                to_username: None,
                                to_display_name: None,
                                account_reference: 0,
                                account_label: request.account_label,
                                timestamp: request.created_at as u64,
                                auto_accept_proof: None,
                            };
                            self.incoming_requests.insert(from_id, contact_request);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load incoming contact requests: {}", e);
                }
            }

            // Load pending outgoing requests from database
            match self.app_context.db.load_pending_contact_requests(
                &identity_id,
                &network_str,
                "sent",
            ) {
                Ok(outgoing) => {
                    tracing::debug!("Loaded {} outgoing requests from database", outgoing.len());
                    for request in outgoing {
                        if let Ok(to_id) = Identifier::from_bytes(&request.to_identity_id) {
                            let contact_request = ContactRequest {
                                request_id: Identifier::new([0; 32]), // We'll need to store this in DB
                                from_identity: identity_id,
                                to_identity: to_id,
                                from_username: None,
                                from_display_name: None,
                                to_username: None,
                                to_display_name: None,
                                account_reference: 0,
                                account_label: request.account_label,
                                timestamp: request.created_at as u64,
                                auto_accept_proof: None,
                            };
                            self.outgoing_requests.insert(to_id, contact_request);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load outgoing contact requests: {}", e);
                }
            }

            // Resolve names from local cache and mark unresolved for Platform fetch
            let unresolved = self.resolve_names_from_local_cache();
            self.pending_profile_fetches.extend(unresolved);
        }
    }

    /// Resolve usernames and display names for contact requests using local DB cache.
    /// Returns a list of identity IDs that were not found locally and need Platform fetching.
    fn resolve_names_from_local_cache(&mut self) -> Vec<Identifier> {
        let network_str = self.app_context.network.to_string();
        let mut unresolved_ids = Vec::new();

        // Resolve names for incoming requests (need from_identity info)
        for request in self.incoming_requests.values_mut() {
            if request.from_username.is_some() || request.from_display_name.is_some() {
                continue; // Already resolved
            }

            let identity_id = request.from_identity;
            let mut found = false;

            // Try profile cache first (has display_name)
            if let Ok(Some(profile)) = self
                .app_context
                .db
                .load_dashpay_profile(&identity_id, &network_str)
                && profile.display_name.is_some()
            {
                request.from_display_name = profile.display_name;
                found = true;
            }

            // Try contacts cache (has username and display_name)
            if let Some(selected_identity) = &self.selected_identity {
                let owner_id = selected_identity.identity.id();
                if let Ok(contacts) = self
                    .app_context
                    .db
                    .load_dashpay_contacts(&owner_id, &network_str)
                {
                    for contact in contacts {
                        if let Ok(contact_id) = Identifier::from_bytes(&contact.contact_identity_id)
                            && contact_id == identity_id
                        {
                            if request.from_username.is_none() {
                                request.from_username = contact.username;
                            }
                            if request.from_display_name.is_none() {
                                request.from_display_name = contact.display_name;
                            }
                            found = true;
                            break;
                        }
                    }
                }
            }

            if !found {
                unresolved_ids.push(identity_id);
            }
        }

        // Resolve names for outgoing requests (need to_identity info)
        for request in self.outgoing_requests.values_mut() {
            if request.to_username.is_some() || request.to_display_name.is_some() {
                continue; // Already resolved
            }

            let identity_id = request.to_identity;
            let mut found = false;

            // Try profile cache first
            if let Ok(Some(profile)) = self
                .app_context
                .db
                .load_dashpay_profile(&identity_id, &network_str)
                && profile.display_name.is_some()
            {
                request.to_display_name = profile.display_name;
                found = true;
            }

            // Try contacts cache
            if let Some(selected_identity) = &self.selected_identity {
                let owner_id = selected_identity.identity.id();
                if let Ok(contacts) = self
                    .app_context
                    .db
                    .load_dashpay_contacts(&owner_id, &network_str)
                {
                    for contact in contacts {
                        if let Ok(contact_id) = Identifier::from_bytes(&contact.contact_identity_id)
                            && contact_id == identity_id
                        {
                            if request.to_username.is_none() {
                                request.to_username = contact.username;
                            }
                            if request.to_display_name.is_none() {
                                request.to_display_name = contact.display_name;
                            }
                            found = true;
                            break;
                        }
                    }
                }
            }

            if !found {
                unresolved_ids.push(identity_id);
            }
        }

        // Deduplicate
        unresolved_ids.sort();
        unresolved_ids.dedup();
        unresolved_ids
    }

    /// Trigger backend fetches for identity profiles that aren't cached locally.
    fn fetch_unresolved_profiles(&self, unresolved_ids: Vec<Identifier>) -> AppAction {
        if unresolved_ids.is_empty() || self.selected_identity.is_none() {
            return AppAction::None;
        }

        let identity = self.selected_identity.as_ref().unwrap().clone();
        let mut action = AppAction::None;

        for contact_id in unresolved_ids {
            let task = BackendTask::DashPayTask(Box::new(DashPayTask::FetchContactProfile {
                identity: identity.clone(),
                contact_id,
            }));
            action |= AppAction::BackendTask(task);
        }

        action
    }

    /// Update contact request names from a fetched profile document.
    fn update_names_from_profile(&mut self, contact_id: Identifier, doc: &Document) {
        let display_name = doc
            .get("displayName")
            .and_then(|v| v.as_text())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());

        // Update incoming requests where from_identity matches
        for request in self.incoming_requests.values_mut() {
            if request.from_identity == contact_id && request.from_display_name.is_none() {
                request.from_display_name = display_name.clone();
            }
        }

        // Update outgoing requests where to_identity matches
        for request in self.outgoing_requests.values_mut() {
            if request.to_identity == contact_id && request.to_display_name.is_none() {
                request.to_display_name = display_name.clone();
            }
        }

        // Save to local DB for future lookups
        let network_str = self.app_context.network.to_string();
        let bio = doc
            .get("publicMessage")
            .and_then(|v| v.as_text())
            .filter(|s| !s.is_empty());
        let avatar_url = doc
            .get("avatarUrl")
            .and_then(|v| v.as_text())
            .filter(|s| !s.is_empty());

        if let Err(e) = self.app_context.db.save_dashpay_profile(
            &contact_id,
            &network_str,
            display_name.as_deref(),
            bio,
            avatar_url,
            None,
        ) {
            tracing::warn!("Failed to cache profile for {}: {}", contact_id, e);
        }
    }

    pub fn trigger_fetch_requests(&mut self) -> AppAction {
        // Only fetch if we have a selected identity
        if let Some(identity) = &self.selected_identity {
            self.loading = true;
            self.message = None;

            let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactRequests {
                identity: identity.clone(),
            }));

            return AppAction::BackendTask(task);
        }

        AppAction::None
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

    pub fn fetch_all_requests(&mut self) -> AppAction {
        self.trigger_fetch_requests()
    }

    pub fn refresh(&mut self) -> AppAction {
        // Don't clear requests - preserve loaded state
        // Only clear temporary states
        self.message = None;
        self.loading = false;

        // Auto-select first identity if none selected
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            self.selected_identity = Some(identities[0].clone());
            self.selected_identity_string = identities[0].display_string();
        }

        // Load requests from database if we have an identity selected
        if self.selected_identity.is_some() {
            self.load_requests_from_database();
        }

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        self.render_content(ui, true)
    }

    fn render_content(&mut self, ui: &mut Ui, show_header: bool) -> AppAction {
        let mut action = AppAction::None;

        // Trigger Platform fetches for unresolved profiles
        if !self.pending_profile_fetches.is_empty() {
            let pending: Vec<_> = self.pending_profile_fetches.drain().collect();
            action |= self.fetch_unresolved_profiles(pending);
        }

        // Handle accept confirmation dialog
        if let Some((dialog, request)) = &mut self.accept_confirmation_dialog {
            let response = dialog.show(ui);
            if response.inner.dialog_response == Some(ConfirmationStatus::Confirmed) {
                if let Some(identity) = &self.selected_identity {
                    // Don't mark as accepted yet - wait for backend confirmation
                    self.loading = true;
                    self.message = Some((
                        "Accepting contact request...".to_string(),
                        MessageType::Info,
                    ));

                    let task =
                        BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
                            identity: identity.clone(),
                            request_id: request.request_id,
                        }));

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
                if let Some(identity) = &self.selected_identity {
                    self.loading = true;
                    self.message = Some((
                        "Rejecting contact request...".to_string(),
                        MessageType::Info,
                    ));

                    // Don't mark as rejected yet - wait for backend confirmation

                    let task =
                        BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest {
                            identity: identity.clone(),
                            request_id: request.request_id,
                        }));

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
            .load_local_qualified_identities()
            .unwrap_or_default();

        // Header with identity selector on the right (only shown when not embedded)
        if show_header {
            ui.horizontal(|ui| {
                ui.heading("Contact Requests");

                if !identities.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response = ui.add(
                            IdentitySelector::new(
                                "requests_identity_selector",
                                &mut self.selected_identity_string,
                                &identities,
                            )
                            .selected_identity(&mut self.selected_identity)
                            .unwrap()
                            .width(300.0)
                            .other_option(false), // Disable "Other" option
                        );

                        if response.changed() {
                            // Clear the requests when identity changes
                            self.incoming_requests.clear();
                            self.outgoing_requests.clear();
                            self.message = None;
                            self.has_fetched_requests = false;
                            self.pending_profile_fetches.clear();

                            // Update wallet for the newly selected identity
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

                            // Load requests from database for the newly selected identity
                            self.load_requests_from_database();
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
        if let Some(err) = self.error.clone() {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            let error_color = if dark_mode {
                DashColors::ERROR
            } else {
                egui::Color32::DARK_RED
            };

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(err.user_message()).color(error_color));

                    // Show action button for missing encryption key
                    if matches!(err, DashPayError::MissingEncryptionKey) {
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
                            self.error = None;
                        }
                    }
                });
            });
            ui.separator();
        }

        // Show regular message if any (non-error)
        if let Some((message, message_type)) = &self.message {
            let color = match message_type {
                MessageType::Success => egui::Color32::DARK_GREEN,
                MessageType::Error => egui::Color32::DARK_RED,
                MessageType::Info => egui::Color32::LIGHT_BLUE,
            };
            // Only show error messages here if there's no structured error
            if message_type == &MessageType::Error && self.error.is_none() {
                ui.colored_label(color, RichText::new(message).strong());
                ui.separator();
            }
        }

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view contact requests");
            return action;
        }

        // Tabs
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        ui.horizontal(|ui| {
            let incoming_tab = egui::Button::new(RichText::new("Incoming").color(
                if self.active_tab == RequestTab::Incoming {
                    DashColors::WHITE
                } else {
                    DashColors::text_primary(dark_mode)
                },
            ))
            .fill(if self.active_tab == RequestTab::Incoming {
                DashColors::DASH_BLUE
            } else {
                DashColors::glass_white(dark_mode)
            })
            .stroke(if self.active_tab == RequestTab::Incoming {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0, DashColors::border(dark_mode))
            })
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::Vec2::new(120.0, 28.0));

            if ui.add(incoming_tab).clicked() {
                self.active_tab = RequestTab::Incoming;
            }

            ui.add_space(8.0);

            let outgoing_tab = egui::Button::new(RichText::new("Outgoing").color(
                if self.active_tab == RequestTab::Outgoing {
                    DashColors::WHITE
                } else {
                    DashColors::text_primary(dark_mode)
                },
            ))
            .fill(if self.active_tab == RequestTab::Outgoing {
                DashColors::DASH_BLUE
            } else {
                DashColors::glass_white(dark_mode)
            })
            .stroke(if self.active_tab == RequestTab::Outgoing {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0, DashColors::border(dark_mode))
            })
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::Vec2::new(120.0, 28.0));

            if ui.add(outgoing_tab).clicked() {
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

                        // Show specific loading message based on current message
                        if let Some((msg, _)) = &self.message {
                            ui.label(msg);
                        } else {
                            ui.label("Loading...");
                        }
                    });
                } else {
                    ScrollArea::vertical().id_salt("incoming_requests_scroll").show(ui, |ui| {
                    if self.incoming_requests.is_empty() {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                                        let dark_mode = ui.ctx().style().visuals.dark_mode;

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
                                        ui.label(
                                            RichText::new("Received: 1 day ago").small().color(DashColors::text_secondary(dark_mode)),
                                        );
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // Check if this request has been accepted or rejected
                                            if self.accepted_requests.contains(&request.request_id) {
                                                // Show checkmark and "Accepted" text
                                                ui.label(
                                                    RichText::new("Accepted")
                                                        .color(DashColors::SUCCESS)
                                                        .strong()
                                                );
                                            } else if self.rejected_requests.contains(&request.request_id) {
                                                // Show X and "Rejected" text
                                                ui.label(
                                                    RichText::new("Rejected")
                                                        .color(DashColors::DANGER_RED)
                                                        .strong()
                                                );
                                            } else {
                                                // Check wallet lock status before showing buttons
                                                let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                                                    if let Err(e) = try_open_wallet_no_password(wallet) {
                                                        self.message = Some((e, MessageType::Error));
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

                        // Show specific loading message based on current message
                        if let Some((msg, _)) = &self.message {
                            ui.label(msg);
                        } else {
                            ui.label("Loading...");
                        }
                    });
                } else {
                    ScrollArea::vertical().id_salt("outgoing_requests_scroll").show(ui, |ui| {
                    if self.outgoing_requests.is_empty() {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                                    .fill(DashColors::DASH_BLUE);
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
                                        let dark_mode = ui.ctx().style().visuals.dark_mode;

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
                                        ui.label(RichText::new("Sent: 2 days ago").small().color(DashColors::text_secondary(dark_mode)));
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let dark_mode = ui.ctx().style().visuals.dark_mode;
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
        // Load requests from database when screen is shown
        if self.selected_identity.is_some() {
            self.load_requests_from_database();
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        // Create a simple central panel for rendering
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ctx, |ui| {
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

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Clear loading state when displaying any message (including errors)
        self.loading = false;

        // Check if this is an error about missing keys
        if message_type == MessageType::Error {
            if message.contains("ENCRYPTION key") {
                self.error = Some(DashPayError::MissingEncryptionKey);
                self.message = None;
                return;
            } else if message.contains("DECRYPTION key") {
                self.error = Some(DashPayError::MissingDecryptionKey);
                self.message = None;
                return;
            }
        }

        self.message = Some((message.to_string(), message_type));
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactRequests {
                incoming,
                outgoing,
            }) => {
                tracing::debug!(
                    "Received DashPayContactRequests result: {} incoming, {} outgoing",
                    incoming.len(),
                    outgoing.len()
                );

                // Clear existing requests
                self.incoming_requests.clear();
                self.outgoing_requests.clear();

                // Mark as fetched
                self.has_fetched_requests = true;

                // Get current identity for saving to database
                let current_identity_id = self.selected_identity.as_ref().unwrap().identity.id();

                // Process incoming requests
                for (id, doc) in incoming.iter() {
                    let properties = doc.properties();
                    let from_identity = doc.owner_id();

                    let account_reference = properties
                        .get("accountReference")
                        .and_then(|v| v.as_integer::<i64>())
                        .and_then(|i| u32::try_from(i).ok())
                        .unwrap_or(0);

                    let timestamp = doc.created_at().or_else(|| doc.updated_at()).unwrap_or(0);

                    let request = ContactRequest {
                        request_id: *id,
                        from_identity,
                        to_identity: current_identity_id,
                        from_username: None,
                        from_display_name: None,
                        to_username: None,
                        to_display_name: None,
                        account_reference,
                        account_label: None, // TODO: Decrypt if present
                        timestamp,
                        auto_accept_proof: None,
                    };

                    self.incoming_requests.insert(*id, request.clone());

                    // Save to database as received request
                    let network_str = self.app_context.network.to_string();
                    tracing::debug!(
                        "Saving incoming contact request to database: from={}, to={}, network={}",
                        from_identity,
                        current_identity_id,
                        network_str
                    );
                    match self.app_context.db.save_contact_request(
                        &from_identity,
                        &current_identity_id,
                        &network_str,
                        None, // to_username
                        request.account_label.as_deref(),
                        "received",
                        Some(id),
                    ) {
                        Ok(row_id) => {
                            tracing::debug!("Saved incoming contact request with row id {}", row_id)
                        }
                        Err(e) => tracing::error!("Failed to save incoming contact request: {}", e),
                    }
                }

                // Process outgoing requests
                for (id, doc) in outgoing.iter() {
                    let properties = doc.properties();
                    let to_identity = properties
                        .get("toUserId")
                        .and_then(|v| v.to_identifier().ok())
                        .unwrap_or_default();

                    let account_reference = properties
                        .get("accountReference")
                        .and_then(|v| v.as_integer::<i64>())
                        .and_then(|i| u32::try_from(i).ok())
                        .unwrap_or(0);

                    let timestamp = doc.created_at().or_else(|| doc.updated_at()).unwrap_or(0);

                    let request = ContactRequest {
                        request_id: *id,
                        from_identity: current_identity_id,
                        to_identity,
                        from_username: None,
                        from_display_name: None,
                        to_username: None,
                        to_display_name: None,
                        account_reference,
                        account_label: None, // TODO: Decrypt if present
                        timestamp,
                        auto_accept_proof: None,
                    };

                    self.outgoing_requests.insert(*id, request.clone());

                    // Save to database as sent request
                    let network_str = self.app_context.network.to_string();
                    tracing::debug!(
                        "Saving outgoing contact request to database: from={}, to={}, network={}",
                        current_identity_id,
                        to_identity,
                        network_str
                    );
                    match self.app_context.db.save_contact_request(
                        &current_identity_id,
                        &to_identity,
                        &network_str,
                        None, // to_username
                        request.account_label.as_deref(),
                        "sent",
                        Some(id),
                    ) {
                        Ok(row_id) => {
                            tracing::debug!("Saved outgoing contact request with row id {}", row_id)
                        }
                        Err(e) => tracing::error!("Failed to save outgoing contact request: {}", e),
                    }
                }

                // Resolve names from local cache and trigger Platform fetches for unknowns
                let unresolved = self.resolve_names_from_local_cache();
                self.pending_profile_fetches.extend(unresolved);
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactProfile(Some(doc))) => {
                // A profile was fetched for an identity — update any matching requests
                let contact_id = doc.owner_id();
                self.update_names_from_profile(contact_id, &doc);
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactProfile(None)) => {
                // No profile found for this identity — nothing to update
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactRequestAccepted(
                request_id,
            )) => {
                // Mark as accepted only after successful backend operation
                self.accepted_requests.insert(request_id);
                self.message = Some((
                    "Contact request accepted successfully".to_string(),
                    MessageType::Success,
                ));
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactRequestRejected(
                request_id,
            )) => {
                // Mark as rejected only after successful backend operation
                self.rejected_requests.insert(request_id);
                self.message = Some(("Contact request rejected".to_string(), MessageType::Success));
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactAlreadyEstablished(_)) => {
                self.message = Some(("Contact already established".to_string(), MessageType::Info));
            }
            BackendTaskSuccessResult::Message(msg) => {
                // Check if this is an error message about missing keys
                if msg.contains("ENCRYPTION key") {
                    self.error = Some(DashPayError::MissingEncryptionKey);
                    self.message = None;
                } else if msg.contains("DECRYPTION key") {
                    self.error = Some(DashPayError::MissingDecryptionKey);
                    self.message = None;
                } else {
                    self.message = Some((msg, MessageType::Success));
                }
            }
            _ => {
                // Ignore other results
            }
        }
    }
}
