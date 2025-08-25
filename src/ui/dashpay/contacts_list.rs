use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;

use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, Ui};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Contact {
    pub identity_id: Identifier,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub nickname: Option<String>,
    pub is_hidden: bool,
    pub account_reference: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchFilter {
    All,
    WithUsernames,    // Only contacts with usernames
    WithoutUsernames, // Only contacts without usernames
    WithBio,          // Contacts with bio
    Recent,           // Recently added (TODO: needs database timestamp)
    Hidden,           // Only hidden contacts
    Visible,          // Only visible contacts
}

#[derive(Debug, Clone, PartialEq)]
pub enum SortOrder {
    Name,       // Sort by display name/username
    Username,   // Sort by username specifically
    DateAdded,  // Sort by date added (TODO: needs database timestamp)
    AccountRef, // Sort by account reference number
}

pub struct ContactsList {
    app_context: Arc<AppContext>,
    contacts: BTreeMap<Identifier, Contact>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    search_query: String,
    message: Option<(String, MessageType)>,
    loading: bool,
    show_hidden: bool,
    search_filter: SearchFilter,
    sort_order: SortOrder,
}

impl ContactsList {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let mut new_self = Self {
            app_context: app_context.clone(),
            contacts: BTreeMap::new(),
            selected_identity: None,
            selected_identity_string: String::new(),
            search_query: String::new(),
            message: None,
            loading: false,
            show_hidden: false,
            search_filter: SearchFilter::All,
            sort_order: SortOrder::Name,
        };

        // Auto-select first identity on creation if available
        if let Ok(identities) = app_context.load_local_qualified_identities() {
            if !identities.is_empty() {
                new_self.selected_identity = Some(identities[0].clone());
                new_self.selected_identity_string =
                    identities[0].identity.id().to_string(Encoding::Base58);
                eprintln!(
                    "[ContactsList::new] Auto-selected identity on creation: {}",
                    new_self.selected_identity_string
                );

                // Load contacts from database for this identity
                new_self.load_contacts_from_database();
                eprintln!(
                    "[ContactsList::new] Loaded {} contacts from database on creation",
                    new_self.contacts.len()
                );
            }
        }

        new_self
    }

    fn load_contacts_from_database(&mut self) {
        // Load saved contacts for the selected identity from database
        if let Some(identity) = &self.selected_identity {
            let identity_id = identity.identity.id();

            // Load saved contacts from database
            if let Ok(stored_contacts) = self.app_context.db.load_dashpay_contacts(&identity_id) {
                for stored_contact in stored_contacts {
                    // Convert stored contact to Contact struct
                    if let Ok(contact_id) =
                        Identifier::from_bytes(&stored_contact.contact_identity_id)
                    {
                        let contact = Contact {
                            identity_id: contact_id,
                            username: stored_contact.username.clone(),
                            display_name: stored_contact.display_name.clone().or_else(|| {
                                Some(format!(
                                    "Contact ({})",
                                    &contact_id.to_string(Encoding::Base58)[0..8]
                                ))
                            }),
                            avatar_url: stored_contact.avatar_url.clone(),
                            bio: None,        // Bio could be loaded from profile if needed
                            nickname: None,   // Will be loaded separately from contact_private_info
                            is_hidden: false, // Will be loaded separately from contact_private_info
                            account_reference: 0, // This would need to be loaded from contactInfo document
                        };

                        // Only add if contact status is accepted
                        if stored_contact.contact_status == "accepted" {
                            self.contacts.insert(contact_id, contact);
                        }
                    }
                }

                // Also load private contact info to populate nickname and hidden status
                if let Ok(private_infos) = self
                    .app_context
                    .db
                    .load_all_contact_private_info(&identity_id)
                {
                    for info in private_infos {
                        if let Ok(contact_id) = Identifier::from_bytes(&info.contact_identity_id) {
                            if let Some(contact) = self.contacts.get_mut(&contact_id) {
                                contact.nickname = if info.nickname.is_empty() {
                                    None
                                } else {
                                    Some(info.nickname)
                                };
                                contact.is_hidden = info.is_hidden;
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn trigger_fetch_contacts(&mut self) -> AppAction {
        // Only fetch if we have a selected identity
        if let Some(identity) = &self.selected_identity {
            self.loading = true;
            self.message = None; // Clear any existing message

            let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
                identity: identity.clone(),
            }));

            return AppAction::BackendTask(task);
        }

        AppAction::None
    }

    pub fn fetch_contacts(&mut self) -> AppAction {
        self.trigger_fetch_contacts()
    }

    pub fn refresh(&mut self) -> AppAction {
        // Don't clear contacts - preserve loaded state
        // Only clear temporary states
        self.message = None;
        self.loading = false;

        // Auto-select first identity if none selected
        if self.selected_identity.is_none() {
            if let Ok(identities) = self.app_context.load_local_qualified_identities() {
                if !identities.is_empty() {
                    self.selected_identity = Some(identities[0].clone());
                    self.selected_identity_string =
                        identities[0].identity.id().to_string(Encoding::Base58);
                    eprintln!(
                        "[ContactsList] Auto-selected identity: {}",
                        self.selected_identity_string
                    );
                }
            }
        }

        // Load contacts from database if we have an identity selected and no contacts loaded
        if self.selected_identity.is_some() && self.contacts.is_empty() {
            eprintln!(
                "[ContactsList] Loading contacts from database for identity: {}",
                self.selected_identity_string
            );
            self.load_contacts_from_database();
            eprintln!(
                "[ContactsList] Loaded {} contacts from database",
                self.contacts.len()
            );
        }

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header section
        ui.heading("My Contacts");

        ui.separator();

        // Identity selector
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 165, 0),
                "No identities loaded. Please load or create an identity first.",
            );
        } else {
            // Identity selector
            ui.horizontal(|ui| {
                let response = ui.add(
                    IdentitySelector::new(
                        "contacts_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .label("Identity:")
                    .width(300.0)
                    .other_option(false),
                );

                if response.changed() {
                    // Clear contacts when identity changes
                    self.contacts.clear();
                    self.message = None;
                    self.loading = false;

                    // Load contacts from database for the newly selected identity
                    self.load_contacts_from_database();
                }
            });

            ui.add_space(5.0);
            ui.separator();

            // Only show search/filter/sort controls if there are contacts
            if !self.contacts.is_empty() {
                // Search bar
                ui.horizontal(|ui| {
                    ui.set_min_height(40.0);
                    ui.label("Search:");
                    ui.add(egui::TextEdit::singleline(&mut self.search_query).desired_width(200.0));
                    if ui.button("Clear").clicked() {
                        self.search_query.clear();
                    }

                    ui.separator();

                    // Filter and sort options in one line
                    ui.vertical(|ui| {
                        ui.add_space(11.0);
                        ui.label("Filter:");
                    });
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        egui::ComboBox::from_id_salt("filter_combo")
                            .selected_text(match self.search_filter {
                                SearchFilter::All => "All",
                                SearchFilter::WithUsernames => "With usernames",
                                SearchFilter::WithoutUsernames => "No usernames",
                                SearchFilter::WithBio => "With bio",
                                SearchFilter::Recent => "Recent",
                                SearchFilter::Hidden => "Hidden",
                                SearchFilter::Visible => "Visible",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.search_filter,
                                    SearchFilter::All,
                                    "All",
                                );
                                ui.selectable_value(
                                    &mut self.search_filter,
                                    SearchFilter::WithUsernames,
                                    "With usernames",
                                );
                                ui.selectable_value(
                                    &mut self.search_filter,
                                    SearchFilter::WithoutUsernames,
                                    "No usernames",
                                );
                                ui.selectable_value(
                                    &mut self.search_filter,
                                    SearchFilter::WithBio,
                                    "With bio",
                                );
                                ui.selectable_value(
                                    &mut self.search_filter,
                                    SearchFilter::Hidden,
                                    "Hidden",
                                );
                                ui.selectable_value(
                                    &mut self.search_filter,
                                    SearchFilter::Visible,
                                    "Visible",
                                );
                            });
                    });

                    ui.separator();

                    ui.vertical(|ui| {
                        ui.add_space(11.0);
                        ui.label("Sort:");
                    });
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        egui::ComboBox::from_id_salt("sort_combo")
                            .selected_text(match self.sort_order {
                                SortOrder::Name => "Name",
                                SortOrder::Username => "Username",
                                SortOrder::DateAdded => "Date",
                                SortOrder::AccountRef => "Account",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.sort_order, SortOrder::Name, "Name");
                                ui.selectable_value(
                                    &mut self.sort_order,
                                    SortOrder::Username,
                                    "Username",
                                );
                                ui.selectable_value(
                                    &mut self.sort_order,
                                    SortOrder::AccountRef,
                                    "Account",
                                );
                            });
                    });

                    ui.separator();

                    ui.checkbox(&mut self.show_hidden, "Show hidden");
                });

                ui.separator();
            }
        }

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
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let spinner_color = if dark_mode {
                    egui::Color32::from_gray(200)
                } else {
                    egui::Color32::from_gray(60)
                };
                ui.add(egui::widgets::Spinner::default().color(spinner_color));
                ui.label("Loading contacts...");
            });
            return action;
        }

        // No identity selected or no identities available
        if identities.is_empty() {
            return action;
        }

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view contacts");
            return action;
        }

        // Filter contacts based on search, filter, and hidden status
        let query = self.search_query.to_lowercase();

        let mut filtered_contacts: Vec<_> = self
            .contacts
            .values()
            .filter(|contact| {
                // Apply search filter first
                match self.search_filter {
                    SearchFilter::WithUsernames if contact.username.is_none() => return false,
                    SearchFilter::WithoutUsernames if contact.username.is_some() => return false,
                    SearchFilter::WithBio if contact.bio.is_none() => return false,
                    SearchFilter::Hidden if !contact.is_hidden => return false,
                    SearchFilter::Visible if contact.is_hidden => return false,
                    SearchFilter::Recent => {
                        // TODO: Implement when we have timestamp data
                        // For now, treat as "All"
                    }
                    _ => {} // SearchFilter::All or other cases pass through
                }

                // Filter by hidden status (unless we're specifically filtering for hidden)
                if matches!(self.search_filter, SearchFilter::Hidden) {
                    // When filtering for hidden, ignore the show_hidden setting
                } else if contact.is_hidden && !self.show_hidden {
                    return false;
                }

                // Filter by search query
                if query.is_empty() {
                    return true;
                }

                // Enhanced search functionality
                let search_in_text = |text: &str| text.to_lowercase().contains(&query);

                // Search in username
                if let Some(username) = &contact.username {
                    if search_in_text(username) {
                        return true;
                    }
                }

                // Search in display name
                if let Some(display_name) = &contact.display_name {
                    if search_in_text(display_name) {
                        return true;
                    }
                }

                // Search in nickname
                if let Some(nickname) = &contact.nickname {
                    if search_in_text(nickname) {
                        return true;
                    }
                }

                // Search in bio
                if let Some(bio) = &contact.bio {
                    if search_in_text(bio) {
                        return true;
                    }
                }

                // Search in identity ID (partial match)
                let identity_str = contact.identity_id.to_string(Encoding::Base58);
                if search_in_text(&identity_str) {
                    return true;
                }

                false
            })
            .cloned()
            .collect();

        // Sort contacts based on selected sort order
        filtered_contacts.sort_by(|a, b| {
            match self.sort_order {
                SortOrder::Name => {
                    let name_a = a
                        .nickname
                        .as_ref()
                        .or(a.display_name.as_ref())
                        .or(a.username.as_ref())
                        .map(|s| s.to_lowercase())
                        .unwrap_or_else(|| "zzz".to_string());
                    let name_b = b
                        .nickname
                        .as_ref()
                        .or(b.display_name.as_ref())
                        .or(b.username.as_ref())
                        .map(|s| s.to_lowercase())
                        .unwrap_or_else(|| "zzz".to_string());
                    name_a.cmp(&name_b)
                }
                SortOrder::Username => {
                    let username_a = a
                        .username
                        .as_ref()
                        .map(|s| s.to_lowercase())
                        .unwrap_or_else(|| "zzz".to_string());
                    let username_b = b
                        .username
                        .as_ref()
                        .map(|s| s.to_lowercase())
                        .unwrap_or_else(|| "zzz".to_string());
                    username_a.cmp(&username_b)
                }
                SortOrder::AccountRef => a.account_reference.cmp(&b.account_reference),
                SortOrder::DateAdded => {
                    // TODO: Implement when we have timestamp data
                    // For now, sort by identity ID as a proxy
                    a.identity_id.cmp(&b.identity_id)
                }
            }
        });

        // Contacts list
        ScrollArea::vertical().show(ui, |ui| {
            if self.contacts.is_empty() {
                ui.label("No contacts loaded");
            } else if filtered_contacts.is_empty() {
                ui.label("No contacts match your search");
            } else {
                for contact in filtered_contacts {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            // Avatar placeholder or actual avatar
                            ui.vertical(|ui| {
                                ui.add_space(5.0);
                                // TODO: Display actual avatar if avatar_url is present
                                // For now, always show placeholder
                                ui.label(RichText::new("👤").size(40.0));
                            });

                            ui.add_space(10.0);

                            ui.vertical(|ui| {
                                // Display name or username
                                let name = contact
                                    .nickname
                                    .as_ref()
                                    .or(contact.display_name.as_ref())
                                    .or(contact.username.as_ref())
                                    .cloned()
                                    .unwrap_or_else(|| "Unknown".to_string());

                                let dark_mode = ui.ctx().style().visuals.dark_mode;

                                // Add hidden indicator to name if contact is hidden
                                let display_name = if contact.is_hidden {
                                    format!("[Hidden] {}", name)
                                } else {
                                    name
                                };

                                ui.label(
                                    RichText::new(display_name)
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                // Username if different from display name
                                if let Some(username) = &contact.username {
                                    if contact.display_name.is_some() || contact.nickname.is_some()
                                    {
                                        ui.label(
                                            RichText::new(format!("@{}", username))
                                                .small()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
                                    }
                                }

                                // Bio
                                if let Some(bio) = &contact.bio {
                                    ui.label(
                                        RichText::new(bio)
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                }

                                // Account reference
                                if contact.account_reference > 0 {
                                    ui.label(
                                        RichText::new(format!(
                                            "Account #{}",
                                            contact.account_reference
                                        ))
                                        .small()
                                        .color(DashColors::text_secondary(dark_mode)),
                                    );
                                }
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Just two buttons: Pay and View Profile
                                    if ui.button("Pay").clicked() {
                                        action = AppAction::AddScreen(
                                            ScreenType::DashPaySendPayment(
                                                self.selected_identity.clone().unwrap(),
                                                contact.identity_id,
                                            )
                                            .create_screen(&self.app_context),
                                        );
                                    }

                                    if ui.button("View Profile").clicked() {
                                        action = AppAction::AddScreen(
                                            ScreenType::DashPayContactProfileViewer(
                                                self.selected_identity.clone().unwrap(),
                                                contact.identity_id,
                                            )
                                            .create_screen(&self.app_context),
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

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
    }
}

impl ScreenLike for ContactsList {
    fn refresh_on_arrival(&mut self) {
        // Load contacts from database when screen is shown
        if self.selected_identity.is_some() && self.contacts.is_empty() {
            self.load_contacts_from_database();
        }
    }

    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ctx, |ui| {
            action = self.render(ui);
        });
        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.loading = false;
        self.message = Some((message.to_string(), message_type));
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPayContacts(contact_ids) => {
                // Clear existing contacts
                self.contacts.clear();

                // Convert contact IDs to Contact structs
                for contact_id in contact_ids {
                    let contact = Contact {
                        identity_id: contact_id,
                        username: None,
                        display_name: Some(format!(
                            "Contact ({})",
                            &contact_id.to_string(Encoding::Base58)[0..8]
                        )),
                        avatar_url: None,
                        bio: None,
                        nickname: None,
                        is_hidden: false,
                        account_reference: 0,
                    };
                    self.contacts.insert(contact_id, contact);
                }

                // Only show message if no contacts found
                if self.contacts.is_empty() {
                    self.message = Some(("No contacts found".to_string(), MessageType::Info));
                } else {
                    self.message = None; // Clear any existing message
                }
            }
            BackendTaskSuccessResult::DashPayContactsWithInfo(contacts_data) => {
                // Clear existing contacts
                self.contacts.clear();

                // Save contacts to database if we have a selected identity
                if let Some(identity) = &self.selected_identity {
                    let owner_id = identity.identity.id();

                    // Convert ContactData to Contact structs and save to database
                    for contact_data in contacts_data {
                        let contact = Contact {
                            identity_id: contact_data.identity_id,
                            username: None,
                            display_name: Some(format!(
                                "Contact ({})",
                                &contact_data.identity_id.to_string(Encoding::Base58)[0..8]
                            )),
                            avatar_url: None,
                            bio: None,
                            nickname: contact_data.nickname.clone(),
                            is_hidden: contact_data.is_hidden,
                            account_reference: contact_data.account_reference,
                        };
                        self.contacts.insert(contact_data.identity_id, contact);

                        // Save to database
                        let _ = self.app_context.db.save_dashpay_contact(
                            &owner_id,
                            &contact_data.identity_id,
                            None,       // username will be loaded when profile is fetched
                            None,       // display_name will be loaded when profile is fetched
                            None,       // avatar_url will be loaded when profile is fetched
                            None,       // public_message will be loaded when profile is fetched
                            "accepted", // Only accepted contacts are returned from load_contacts
                        );

                        // Save private info if present
                        if let Some(nickname) = &contact_data.nickname {
                            let _ = self.app_context.db.save_contact_private_info(
                                &owner_id,
                                &contact_data.identity_id,
                                nickname,
                                &contact_data.note.unwrap_or_default(),
                                contact_data.is_hidden,
                            );
                        }
                    }
                } else {
                    // No selected identity, just populate in-memory
                    for contact_data in contacts_data {
                        let contact = Contact {
                            identity_id: contact_data.identity_id,
                            username: None,
                            display_name: Some(format!(
                                "Contact ({})",
                                &contact_data.identity_id.to_string(Encoding::Base58)[0..8]
                            )),
                            avatar_url: None,
                            bio: None,
                            nickname: contact_data.nickname,
                            is_hidden: contact_data.is_hidden,
                            account_reference: contact_data.account_reference,
                        };
                        self.contacts.insert(contact_data.identity_id, contact);
                    }
                }

                // Only show message if no contacts found
                if self.contacts.is_empty() {
                    self.message = Some(("No contacts found".to_string(), MessageType::Info));
                } else {
                    self.message = None; // Clear any existing message
                }
            }
            BackendTaskSuccessResult::DashPayContactProfile(profile_doc) => {
                // Extract profile information from the document
                if let Some(doc) = profile_doc {
                    use dash_sdk::dpp::document::DocumentV0Getters;
                    let properties = doc.properties();
                    let contact_id = doc.owner_id();

                    let display_name = properties
                        .get("displayName")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());

                    let bio = properties
                        .get("bio")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());

                    let avatar_url = properties
                        .get("avatarUrl")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());

                    let public_message = properties
                        .get("publicMessage")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());

                    // Update the contact with profile information
                    if let Some(contact) = self.contacts.get_mut(&contact_id) {
                        if let Some(name) = &display_name {
                            contact.display_name = Some(name.clone());
                        }
                        if let Some(bio_text) = &bio {
                            contact.bio = Some(bio_text.clone());
                        }
                        if let Some(url) = &avatar_url {
                            contact.avatar_url = Some(url.clone());
                        }

                        // Save updated profile to database if we have a selected identity
                        if let Some(identity) = &self.selected_identity {
                            let owner_id = identity.identity.id();
                            let _ = self.app_context.db.save_dashpay_contact(
                                &owner_id,
                                &contact_id,
                                contact.username.as_deref(),
                                contact.display_name.as_deref(),
                                contact.avatar_url.as_deref(),
                                public_message.as_deref(),
                                "accepted",
                            );
                        }
                    }
                }
            }
            _ => {
                self.message = Some(("Operation completed".to_string(), MessageType::Success));
            }
        }
    }
}
