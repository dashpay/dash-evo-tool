use crate::app::AppAction;
use crate::backend_task::dashpay::{DashPayResult, DashPayTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::dashpay::DashPaySubscreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

const PRIVATE_CONTACT_INFO_TEXT: &str = "About Private Contact Information:\n\n\
    This information is encrypted and stored on Platform.\n\n\
    It is never shared with the contact - only you can decrypt it.\n\n\
    Only you can see these nicknames and notes.\n\n\
    Use this to organize and remember your contacts.";

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
    show_info_popup: bool,
    needs_backend_fetch: bool,
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
            show_info_popup: false,
            needs_backend_fetch: true,
        };
        screen.load_from_database();
        screen
    }

    /// Load contact data from local database for immediate display.
    fn load_from_database(&mut self) {
        let identity_id = self.identity.identity.id();
        let network_str = self.app_context.network.to_string();

        // Try to load the contact's public info from the dashpay_contacts table
        let mut username = None;
        let mut display_name = None;
        let mut avatar_url = None;
        let mut bio = None;

        if let Ok(stored_contacts) = self
            .app_context
            .db
            .load_dashpay_contacts(&identity_id, &network_str)
        {
            for stored_contact in stored_contacts {
                if let Ok(contact_id) = Identifier::from_bytes(&stored_contact.contact_identity_id)
                    && contact_id == self.contact_id
                {
                    username = stored_contact.username;
                    display_name = stored_contact.display_name;
                    avatar_url = stored_contact.avatar_url;
                    // bio is stored in profiles, not contacts table
                    break;
                }
            }
        }

        // Load the profile for bio if available
        if let Ok(Some(profile)) = self
            .app_context
            .db
            .load_dashpay_profile(&self.contact_id, &network_str)
        {
            bio = profile.bio;
            // Also prefer profile display_name and avatar_url if not already set
            if display_name.is_none() {
                display_name = profile.display_name;
            }
            if avatar_url.is_none() {
                avatar_url = profile.avatar_url;
            }
        }

        // Load private contact info (nickname, notes, hidden)
        let (nickname, note, is_hidden) = self
            .app_context
            .db
            .load_contact_private_info(&identity_id, &self.contact_id)
            .unwrap_or_default();

        let nickname = if nickname.is_empty() {
            None
        } else {
            Some(nickname)
        };
        let note = if note.is_empty() { None } else { Some(note) };

        self.contact_info = Some(ContactInfo {
            identity_id: self.contact_id,
            username,
            display_name,
            bio,
            avatar_url,
            nickname,
            note,
            is_hidden,
            account_reference: 0,
        });
    }

    /// Trigger a backend fetch to refresh contact profile data from Platform.
    fn trigger_backend_fetch(&mut self) -> AppAction {
        self.loading = true;

        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::FetchContactProfile {
                identity: self.identity.clone(),
                contact_id: self.contact_id,
            },
        )))
    }

    fn start_editing(&mut self) {
        if let Some(info) = &self.contact_info {
            self.edit_nickname = info.nickname.clone().unwrap_or_default();
            self.edit_note = info.note.clone().unwrap_or_default();
            self.edit_hidden = info.is_hidden;
            self.editing_info = true;
        }
    }

    fn save_contact_info(&mut self) -> AppAction {
        // Update local state immediately for responsive UI
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

        // Save to local database immediately
        let identity_id = self.identity.identity.id();
        if let Err(e) = self.app_context.db.save_contact_private_info(
            &identity_id,
            &self.contact_id,
            &self.edit_nickname,
            &self.edit_note,
            self.edit_hidden,
        ) {
            tracing::warn!("Failed to save contact private info to database: {}", e);
        }

        self.editing_info = false;
        self.loading = true;

        // Dispatch backend task to persist to Platform (encrypted)
        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::UpdateContactInfo {
                identity: self.identity.clone(),
                contact_id: self.contact_id,
                nickname: if self.edit_nickname.is_empty() {
                    None
                } else {
                    Some(self.edit_nickname.clone())
                },
                note: if self.edit_note.is_empty() {
                    None
                } else {
                    Some(self.edit_note.clone())
                },
                is_hidden: self.edit_hidden,
                accepted_accounts: vec![],
            },
        )))
    }

    fn cancel_editing(&mut self) {
        self.editing_info = false;
        self.edit_nickname.clear();
        self.edit_note.clear();
        self.edit_hidden = false;
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Dispatch deferred backend fetch if flagged
        if self.needs_backend_fetch {
            self.needs_backend_fetch = false;
            action = self.trigger_backend_fetch();
        }

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
                MessageType::Success => DashColors::SUCCESS,
                MessageType::Error => DashColors::ERROR,
                MessageType::Info => DashColors::INFO,
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
                            ui.label(RichText::new("👤").size(60.0).color(DashColors::DEEP_BLUE));
                            ui.small("Contact");
                        });

                        ui.vertical(|ui| {
                            // Display nickname if set, otherwise display name
                            let name = info
                                .nickname
                                .as_ref()
                                .or(info.display_name.as_ref())
                                .or(info.username.as_ref())
                                .cloned()
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
                            // Send Payment requires SPV which is dev mode only
                            if self.app_context.is_developer_mode()
                                && ui.button("Send Payment").clicked()
                            {
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
                        ui.label(RichText::new("Private Contact Information").strong());
                        ui.add_space(5.0);
                        if crate::ui::helpers::info_icon_button(ui, PRIVATE_CONTACT_INFO_TEXT)
                            .clicked()
                        {
                            self.show_info_popup = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.editing_info {
                                if ui.button("Cancel").clicked() {
                                    self.cancel_editing();
                                }
                                if ui.button("Save").clicked() {
                                    action = self.save_contact_info();
                                }
                            } else if ui.button("Edit").clicked() {
                                self.start_editing();
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
                                    .color(DashColors::WARNING_ORANGE),
                            );
                        }

                        if info.nickname.is_none() && info.note.is_none() && !info.is_hidden {
                            ui.label(
                                RichText::new(
                                    "No private info set. Click Edit to add a nickname or note.",
                                )
                                .weak(),
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
                                    ui.label(RichText::new("⬇").color(DashColors::SUCCESS));
                                } else {
                                    ui.label(RichText::new("⬆").color(DashColors::ERROR));
                                }

                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        // Amount
                                        let amount_str = format!("{} Dash", payment.amount);
                                        if payment.is_incoming {
                                            ui.label(
                                                RichText::new(format!("+{}", amount_str))
                                                    .color(DashColors::SUCCESS),
                                            );
                                        } else {
                                            ui.label(
                                                RichText::new(format!("-{}", amount_str))
                                                    .color(DashColors::ERROR),
                                            );
                                        }

                                        // Memo
                                        if let Some(memo) = &payment.memo {
                                            ui.label(
                                                RichText::new(format!("\"{}\"", memo))
                                                    .italics()
                                                    .color(DashColors::text_secondary(dark_mode)),
                                            );
                                        }
                                    });

                                    ui.horizontal(|ui| {
                                        // Transaction ID
                                        ui.label(
                                            RichText::new(&payment.tx_id)
                                                .small()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
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

                    ui.label(
                        RichText::new(
                            "Contact removal and blocking are not yet available. \
                             Contact requests cannot be revoked once sent on Platform.",
                        )
                        .weak(),
                    );
                });
            } else {
                // No contact info loaded
                ui.group(|ui| {
                    ui.label("No contact information available");
                    ui.separator();
                    ui.label(format!("Contact ID: {}", self.contact_id));
                    ui.add_space(10.0);
                    if ui.button("Refresh from Platform").clicked() {
                        action = self.trigger_backend_fetch();
                    }
                });

                ui.add_space(10.0);
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
        self.load_from_database();
        self.message = None;
    }

    fn refresh_on_arrival(&mut self) {
        self.load_from_database();
        self.message = None;
        // Flag that we need a backend fetch; it will be dispatched from render()
        self.needs_backend_fetch = true;
    }

    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel with contact name if available
        let contact_name = self
            .contact_info
            .as_ref()
            .and_then(|info| {
                info.nickname
                    .as_ref()
                    .or(info.display_name.as_ref().or(info.username.as_ref()))
            })
            .map(|name| format!("Contact: {}", name))
            .unwrap_or_else(|| "Contact Details".to_string());

        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                (&contact_name, AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashpay);
        action |=
            add_dashpay_subscreen_chooser_panel(ctx, &self.app_context, DashPaySubscreen::Contacts);

        action |= island_central_panel(ctx, |ui| self.render(ui));

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut popup =
                        InfoPopup::new("Private Contact Information", PRIVATE_CONTACT_INFO_TEXT);
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactProfile(Some(doc))) => {
                // Update contact info with fresh profile data from Platform
                use dash_sdk::dpp::document::DocumentV0Getters;
                let properties = doc.properties();

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

                // Save profile to local database for future offline access
                let network_str = self.app_context.network.to_string();
                if let Err(e) = self.app_context.db.save_dashpay_profile(
                    &self.contact_id,
                    &network_str,
                    display_name.as_deref(),
                    bio.as_deref(),
                    avatar_url.as_deref(),
                    None, // public_message
                ) {
                    tracing::warn!("Failed to save dashpay profile to database: {}", e);
                }

                // Update the in-memory contact info
                if let Some(info) = &mut self.contact_info {
                    if display_name.is_some() {
                        info.display_name = display_name;
                    }
                    if bio.is_some() {
                        info.bio = bio;
                    }
                    if avatar_url.is_some() {
                        info.avatar_url = avatar_url;
                    }
                } else {
                    // No existing info — create one with the profile data
                    self.contact_info = Some(ContactInfo {
                        identity_id: self.contact_id,
                        username: None,
                        display_name,
                        bio,
                        avatar_url,
                        nickname: None,
                        note: None,
                        is_hidden: false,
                        account_reference: 0,
                    });
                }
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactInfoUpdated(contact_id)) => {
                if contact_id == self.contact_id {
                    self.display_message("Contact info saved to Platform", MessageType::Success);
                }
            }
            BackendTaskSuccessResult::DashPay(DashPayResult::ContactsWithInfo(contacts_data)) => {
                // If a full contacts reload happened, update our contact if present
                for contact_data in contacts_data {
                    if contact_data.identity_id == self.contact_id {
                        if let Some(info) = &mut self.contact_info {
                            if contact_data.username.is_some() {
                                info.username = contact_data.username;
                            }
                            if contact_data.display_name.is_some() {
                                info.display_name = contact_data.display_name;
                            }
                            if contact_data.avatar_url.is_some() {
                                info.avatar_url = contact_data.avatar_url;
                            }
                            if contact_data.bio.is_some() {
                                info.bio = contact_data.bio;
                            }
                            info.nickname = contact_data.nickname;
                            info.note = contact_data.note;
                            info.is_hidden = contact_data.is_hidden;
                            info.account_reference = contact_data.account_reference;
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}
