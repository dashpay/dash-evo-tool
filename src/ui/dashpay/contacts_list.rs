use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike, ScreenType};
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
        Self {
            app_context,
            contacts: BTreeMap::new(),
            selected_identity: None,
            selected_identity_string: String::new(),
            search_query: String::new(),
            message: None,
            loading: false,
            show_hidden: false,
            search_filter: SearchFilter::All,
            sort_order: SortOrder::Name,
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
        // Just clear state, don't auto-fetch
        self.contacts.clear();
        self.message = None;
        self.loading = false;
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
                ui.label("Identity:");
                let response = ui.add(
                    IdentitySelector::new(
                        "contacts_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .width(300.0)
                    .other_option(false),
                );

                if response.changed() {
                    // Clear contacts when identity changes
                    self.contacts.clear();
                    self.message = None;
                    self.loading = false;
                }
            });

            ui.add_space(5.0);

            // Search bar
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(egui::TextEdit::singleline(&mut self.search_query).desired_width(200.0));
                if ui.button("Clear").clicked() {
                    self.search_query.clear();
                }
            });

            // Filter and sort options in one line
            ui.horizontal(|ui| {
                ui.label("Filter:");
                egui::ComboBox::from_id_source("filter_combo")
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
                        ui.selectable_value(&mut self.search_filter, SearchFilter::All, "All");
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
                        ui.selectable_value(&mut self.search_filter, SearchFilter::Hidden, "Hidden");
                        ui.selectable_value(&mut self.search_filter, SearchFilter::Visible, "Visible");
                    });

                ui.separator();

                ui.label("Sort:");
                egui::ComboBox::from_id_source("sort_combo")
                    .selected_text(match self.sort_order {
                        SortOrder::Name => "Name",
                        SortOrder::Username => "Username",
                        SortOrder::DateAdded => "Date",
                        SortOrder::AccountRef => "Account",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort_order, SortOrder::Name, "Name");
                        ui.selectable_value(&mut self.sort_order, SortOrder::Username, "Username");
                        ui.selectable_value(&mut self.sort_order, SortOrder::AccountRef, "Account");
                    });

                ui.separator();

                ui.checkbox(&mut self.show_hidden, "Show hidden");
            });
        }

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
                let search_in_text = |text: &str| {
                    text.to_lowercase().contains(&query)
                };

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

        // Show search stats
        if !self.contacts.is_empty() {
            let total_contacts = self.contacts.len();
            let visible_contacts = filtered_contacts.len();
            let hidden_count = self.contacts.values().filter(|c| c.is_hidden).count();

            ui.horizontal(|ui| {
                ui.label(format!(
                    "Showing {} of {} contacts",
                    visible_contacts, total_contacts
                ));
                if hidden_count > 0 && !self.show_hidden {
                    ui.label(
                        RichText::new(format!("({} hidden)", hidden_count))
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
            });
            ui.separator();
        }

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
                            ui.vertical(|ui| {
                                // Display name or username
                                let name = contact
                                    .nickname
                                    .as_ref()
                                    .or(contact.display_name.as_ref())
                                    .or(contact.username.as_ref())
                                    .map(|s| s.clone())
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
                                    // Action buttons
                                    if ui.button("Pay").clicked() {
                                        // TODO: Navigate to send payment screen with this contact
                                        action = AppAction::AddScreen(
                                            ScreenType::DashPaySendPayment(
                                                self.selected_identity.clone().unwrap(),
                                                contact.identity_id,
                                            )
                                            .create_screen(&self.app_context),
                                        );
                                    }

                                    if ui.button("Details").clicked() {
                                        // TODO: Navigate to contact details screen
                                        action = AppAction::AddScreen(
                                            ScreenType::DashPayContactDetails(
                                                self.selected_identity.clone().unwrap(),
                                                contact.identity_id,
                                            )
                                            .create_screen(&self.app_context),
                                        );
                                    }

                                    if ui.button("Edit").clicked() {
                                        action = AppAction::AddScreen(
                                            ScreenType::DashPayContactInfoEditor(
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
                            contact_id.to_string(Encoding::Base58)[0..8].to_string()
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

                // Convert ContactData to Contact structs
                for contact_data in contacts_data {
                    let contact = Contact {
                        identity_id: contact_data.identity_id,
                        username: None,
                        display_name: Some(format!(
                            "Contact ({})",
                            contact_data.identity_id.to_string(Encoding::Base58)[0..8].to_string()
                        )),
                        avatar_url: None,
                        bio: None,
                        nickname: contact_data.nickname,
                        is_hidden: contact_data.is_hidden,
                        account_reference: contact_data.account_reference,
                    };
                    self.contacts.insert(contact_data.identity_id, contact);
                }

                // Only show message if no contacts found
                if self.contacts.is_empty() {
                    self.message = Some(("No contacts found".to_string(), MessageType::Info));
                } else {
                    self.message = None; // Clear any existing message
                }
            }
            _ => {
                self.message = Some(("Operation completed".to_string(), MessageType::Success));
            }
        }
    }
}
