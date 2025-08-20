use crate::app::AppAction;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::backend_task::dashpay::DashPayTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike, ScreenType};
use dash_sdk::platform::Identifier;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
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

pub struct ContactsList {
    app_context: Arc<AppContext>,
    contacts: BTreeMap<Identifier, Contact>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    search_query: String,
    message: Option<(String, MessageType)>,
    loading: bool,
    show_hidden: bool,
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
            // Use grid for aligned inputs
            egui::Grid::new("contacts_input_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Identity:");
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            IdentitySelector::new(
                                "contacts_identity_selector",
                                &mut self.selected_identity_string,
                                &identities,
                            )
                            .selected_identity(&mut self.selected_identity)
                            .unwrap()
                            .width(300.0)
                            .other_option(false), // Disable "Other" option
                        );

                        if response.changed() {
                            // Clear contacts when identity changes, but don't auto-fetch
                            self.contacts.clear();
                            self.message = None;
                            self.loading = false;
                        }
                    });
                    ui.end_row();


                    ui.label("Search:");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.search_query);
                        if ui.button("Clear").clicked() {
                            self.search_query.clear();
                        }
                    });
                    ui.end_row();

                    ui.label("Options:");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.show_hidden, "Show hidden contacts");
                    });
                    ui.end_row();
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

        // Filter contacts based on search and hidden status
        let query = self.search_query.to_lowercase();
        let filtered_contacts: Vec<_> = self
            .contacts
            .values()
            .filter(|contact| {
                // Filter by hidden status
                if contact.is_hidden && !self.show_hidden {
                    return false;
                }
                // Filter by search query
                if query.is_empty() {
                    return true;
                }
                contact
                    .username
                    .as_ref()
                    .map_or(false, |u| u.to_lowercase().contains(&query))
                    || contact
                        .display_name
                        .as_ref()
                        .map_or(false, |d| d.to_lowercase().contains(&query))
                    || contact
                        .nickname
                        .as_ref()
                        .map_or(false, |n| n.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();

        // Contacts list
        ScrollArea::vertical().show(ui, |ui| {
            if filtered_contacts.is_empty() {
                ui.label("No contacts found");
            } else {
                for contact in filtered_contacts {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            // Avatar placeholder
                            ui.add(egui::Label::new(RichText::new("👤").size(30.0)));

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
                                    format!("👁‍🗨 {}", name)
                                } else {
                                    name
                                };
                                
                                ui.label(RichText::new(display_name).strong().color(DashColors::text_primary(dark_mode)));

                                // Username if different from display name
                                if let Some(username) = &contact.username {
                                    if contact.display_name.is_some() || contact.nickname.is_some()
                                    {
                                        ui.label(RichText::new(format!("@{}", username)).small().color(DashColors::text_secondary(dark_mode)));
                                    }
                                }

                                // Bio
                                if let Some(bio) = &contact.bio {
                                    ui.label(RichText::new(bio).small().color(DashColors::text_secondary(dark_mode)));
                                }
                                
                                // Account reference
                                if contact.account_reference > 0 {
                                    ui.label(RichText::new(format!("Account #{}", contact.account_reference)).small().color(DashColors::text_secondary(dark_mode)));
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
                        display_name: Some(format!("Contact ({})", contact_id.to_string(Encoding::Base58)[0..8].to_string())),
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
                    self.message = Some((
                        "No contacts found".to_string(),
                        MessageType::Info
                    ));
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
                        display_name: Some(format!("Contact ({})", contact_data.identity_id.to_string(Encoding::Base58)[0..8].to_string())),
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
                    self.message = Some((
                        "No contacts found".to_string(),
                        MessageType::Info
                    ));
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
