use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, Ui};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ContactRequest {
    pub request_id: Identifier,
    pub from_identity: Identifier,
    pub to_identity: Identifier,
    pub from_username: Option<String>,
    pub from_display_name: Option<String>,
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
    app_context: Arc<AppContext>,
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
        };

        // Auto-select first identity on creation if available
        if let Ok(identities) = app_context.load_local_qualified_identities() {
            if !identities.is_empty() {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                new_self.selected_identity = Some(identities[0].clone());
                new_self.selected_identity_string = identities[0]
                    .identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

                // Load requests from database for this identity
                new_self.load_requests_from_database();
            }
        }

        new_self
    }

    fn load_requests_from_database(&mut self) {
        // Load saved contact requests for the selected identity from database
        if let Some(identity) = &self.selected_identity {
            let identity_id = identity.identity.id();

            // Clear existing requests before loading
            self.incoming_requests.clear();
            self.outgoing_requests.clear();

            // Load pending incoming requests from database
            if let Ok(incoming) = self
                .app_context
                .db
                .load_pending_contact_requests(&identity_id, "received")
            {
                for request in incoming {
                    if let Ok(from_id) = Identifier::from_bytes(&request.from_identity_id) {
                        let contact_request = ContactRequest {
                            request_id: Identifier::new([0; 32]), // We'll need to store this in DB
                            from_identity: from_id,
                            to_identity: identity_id,
                            from_username: request.to_username, // This field is misnamed in DB
                            from_display_name: None,
                            account_reference: 0,
                            account_label: request.account_label,
                            timestamp: request.created_at as u64,
                            auto_accept_proof: None,
                        };
                        self.incoming_requests.insert(from_id, contact_request);
                    }
                }
            }

            // Load pending outgoing requests from database
            if let Ok(outgoing) = self
                .app_context
                .db
                .load_pending_contact_requests(&identity_id, "sent")
            {
                for request in outgoing {
                    if let Ok(to_id) = Identifier::from_bytes(&request.to_identity_id) {
                        let contact_request = ContactRequest {
                            request_id: Identifier::new([0; 32]), // We'll need to store this in DB
                            from_identity: identity_id,
                            to_identity: to_id,
                            from_username: None,
                            from_display_name: None,
                            account_reference: 0,
                            account_label: request.account_label,
                            timestamp: request.created_at as u64,
                            auto_accept_proof: None,
                        };
                        self.outgoing_requests.insert(to_id, contact_request);
                    }
                }
            }
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

    pub fn fetch_all_requests(&mut self) -> AppAction {
        self.trigger_fetch_requests()
    }

    pub fn refresh(&mut self) -> AppAction {
        // Don't clear requests - preserve loaded state
        // Only clear temporary states
        self.message = None;
        self.loading = false;

        // Auto-select first identity if none selected
        if self.selected_identity.is_none() {
            if let Ok(identities) = self.app_context.load_local_qualified_identities() {
                if !identities.is_empty() {
                    self.selected_identity = Some(identities[0].clone());
                    self.selected_identity_string = identities[0].display_string();
                }
            }
        }

        // Load requests from database if we have an identity selected and no requests loaded
        if self.selected_identity.is_some()
            && self.incoming_requests.is_empty()
            && self.outgoing_requests.is_empty()
        {
            self.load_requests_from_database();
        }

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.heading("Contact Requests");

        ui.separator();

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 165, 0),
                "⚠️ No identities loaded. Please load or create an identity first.",
            );
        } else {
            ui.horizontal(|ui| {
                let response = ui.add(
                    IdentitySelector::new(
                        "requests_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .label("Identity:")
                    .width(300.0)
                    .other_option(false), // Disable "Other" option
                );

                if response.changed() {
                    // Clear the requests when identity changes
                    self.incoming_requests.clear();
                    self.outgoing_requests.clear();
                    self.message = None;
                    self.has_fetched_requests = false;

                    // Load requests from database for the newly selected identity
                    self.load_requests_from_database();
                }
            });
        }

        ui.separator();

        // No identity selected or no identities available
        if identities.is_empty() {
            return action;
        }

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view contact requests");
            return action;
        }

        // Tabs
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.active_tab == RequestTab::Incoming, "Incoming")
                .clicked()
            {
                self.active_tab = RequestTab::Incoming;
            }
            ui.separator();
            if ui
                .selectable_label(self.active_tab == RequestTab::Outgoing, "Outgoing")
                .clicked()
            {
                self.active_tab = RequestTab::Outgoing;
            }
        });

        ui.separator();

        // Display requests based on active tab
        match self.active_tab {
            RequestTab::Incoming => {
                // Loading indicator
                if self.loading {
                    ui.horizontal(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let spinner_color = if dark_mode {
                            egui::Color32::from_gray(200)
                        } else {
                            egui::Color32::from_gray(60)
                        };
                        ui.add(egui::widgets::Spinner::default().color(spinner_color));

                        // Show specific loading message based on current message
                        if let Some((msg, _)) = &self.message {
                            ui.label(msg);
                        } else {
                            ui.label("Loading...");
                        }
                    });
                } else {
                    ScrollArea::vertical().show(ui, |ui| {
                    if !self.has_fetched_requests {
                        ui.label("No contact requests loaded");
                    } else if self.incoming_requests.is_empty() {
                        ui.label("No incoming contact requests found");
                    } else {
                        let requests: Vec<_> = self.incoming_requests.values().cloned().collect();
                        for request in requests {
                            ui.group(|ui| {
                                let dark_mode = ui.ctx().style().visuals.dark_mode;
                                ui.horizontal(|ui| {
                                    // Avatar placeholder
                                    ui.add(egui::Label::new(RichText::new("👤").size(30.0)));

                                    ui.vertical(|ui| {
                                        use dash_sdk::dpp::platform_value::string_encoding::Encoding;

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
                                                    RichText::new("✓ Accepted")
                                                        .color(egui::Color32::from_rgb(0, 150, 0))
                                                        .strong()
                                                );
                                            } else if self.rejected_requests.contains(&request.request_id) {
                                                // Show X and "Rejected" text
                                                ui.label(
                                                    RichText::new("✗ Rejected")
                                                        .color(egui::Color32::from_rgb(150, 0, 0))
                                                        .strong()
                                                );
                                            } else {
                                                // Show Accept/Reject buttons
                                                if ui.button("Reject").clicked() {
                                                    if let Some(identity) = &self.selected_identity {
                                                        self.loading = true;
                                                        self.message = Some(("Rejecting contact request...".to_string(), MessageType::Info));

                                                        // Mark as rejected immediately for UI feedback
                                                        self.rejected_requests.insert(request.request_id);

                                                        let task = BackendTask::DashPayTask(Box::new(DashPayTask::RejectContactRequest {
                                                            identity: identity.clone(),
                                                            request_id: request.request_id,
                                                        }));

                                                        action |= AppAction::BackendTask(task);
                                                    }
                                                }

                                                if ui.button("Accept").clicked() {
                                                    if let Some(identity) = &self.selected_identity {
                                                        // Mark as accepted immediately for UI feedback
                                                        self.accepted_requests.insert(request.request_id);
                                                        self.loading = true;
                                                        self.message = Some(("Accepting contact request...".to_string(), MessageType::Info));

                                                        let task = BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
                                                            identity: identity.clone(),
                                                            request_id: request.request_id,
                                                        }));

                                                        action |= AppAction::BackendTask(task);
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
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let spinner_color = if dark_mode {
                            egui::Color32::from_gray(200)
                        } else {
                            egui::Color32::from_gray(60)
                        };
                        ui.add(egui::widgets::Spinner::default().color(spinner_color));

                        // Show specific loading message based on current message
                        if let Some((msg, _)) = &self.message {
                            ui.label(msg);
                        } else {
                            ui.label("Loading...");
                        }
                    });
                } else {
                    ScrollArea::vertical().show(ui, |ui| {
                    if !self.has_fetched_requests {
                        ui.label("No contact requests loaded");
                    } else if self.outgoing_requests.is_empty() {
                        ui.label("No outgoing contact requests found");
                    } else {
                        let requests: Vec<_> = self.outgoing_requests.values().cloned().collect();
                        for request in requests {
                            ui.group(|ui| {
                                let dark_mode = ui.ctx().style().visuals.dark_mode;
                                ui.horizontal(|ui| {
                                    // Avatar placeholder
                                    ui.add(egui::Label::new(RichText::new("👤").size(30.0)));

                                    ui.vertical(|ui| {
                                        use dash_sdk::dpp::platform_value::string_encoding::Encoding;

                                        // For outgoing requests, show the TO identity
                                        let id_str = request.to_identity.to_string(Encoding::Base58);
                                        let name = format!("To: {}...{}", &id_str[..6], &id_str[id_str.len()-6..]);

                                        ui.label(RichText::new(name).strong().color(DashColors::text_primary(dark_mode)));

                                        // Show full identity ID
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
                                            if ui.button("Cancel").clicked() {
                                                // TODO: Cancel outgoing request
                                                self.display_message(
                                                    "Request cancelled",
                                                    MessageType::Info,
                                                );
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
        }

        action
    }
}

impl ScreenLike for ContactRequests {
    fn refresh_on_arrival(&mut self) {
        // Load requests from database when screen is shown
        if self.selected_identity.is_some()
            && self.incoming_requests.is_empty()
            && self.outgoing_requests.is_empty()
        {
            self.load_requests_from_database();
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        // Create a simple central panel for rendering
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ctx, |ui| {
            action = self.render(ui);
        });
        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Clear loading state when displaying any message (including errors)
        self.loading = false;
        self.message = Some((message.to_string(), message_type));
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        use dash_sdk::dpp::document::DocumentV0Getters;

        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing } => {
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
                        from_username: None, // TODO: Resolve username from identity
                        from_display_name: None, // TODO: Fetch from profile
                        account_reference,
                        account_label: None, // TODO: Decrypt if present
                        timestamp,
                        auto_accept_proof: None,
                    };

                    self.incoming_requests.insert(*id, request.clone());

                    // Save to database as received request
                    let _ = self.app_context.db.save_contact_request(
                        &from_identity,
                        &current_identity_id,
                        None, // to_username
                        request.account_label.as_deref(),
                        "received",
                    );
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
                        from_username: None,     // This would be our username
                        from_display_name: None, // This would be our display name
                        account_reference,
                        account_label: None, // TODO: Decrypt if present
                        timestamp,
                        auto_accept_proof: None,
                    };

                    self.outgoing_requests.insert(*id, request.clone());

                    // Save to database as sent request
                    let _ = self.app_context.db.save_contact_request(
                        &current_identity_id,
                        &to_identity,
                        None, // to_username
                        request.account_label.as_deref(),
                        "sent",
                    );
                }

                // Don't show a message, just display the results
            }
            BackendTaskSuccessResult::Message(msg) => {
                // Refresh the list after successful accept/reject operations
                if msg.contains("Accepted") || msg.contains("Rejected") {
                    // Trigger a refresh
                    // Note: We can't return an action from display_task_result,
                    // so we'll need to handle this differently
                }
                self.message = Some((msg, MessageType::Success));
            }
            _ => {
                self.message = Some(("Operation completed".to_string(), MessageType::Success));
            }
        }
    }
}
