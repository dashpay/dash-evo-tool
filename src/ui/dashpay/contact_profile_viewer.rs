use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{RichText, ScrollArea, Ui};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ContactPublicProfile {
    pub identity_id: Identifier,
    pub display_name: Option<String>,
    pub public_message: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_hash: Option<Vec<u8>>,
    pub avatar_fingerprint: Option<Vec<u8>>,
}

pub struct ContactProfileViewerScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    pub contact_id: Identifier,
    profile: Option<ContactPublicProfile>,
    message: Option<(String, MessageType)>,
    loading: bool,
    initial_fetch_done: bool,
    // Private contact info fields
    nickname: String,
    notes: String,
    is_hidden: bool,
    editing_private_info: bool,
}

impl ContactProfileViewerScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        identity: QualifiedIdentity,
        contact_id: Identifier,
    ) -> Self {
        // Load private contact info from database
        let (nickname, notes, is_hidden) = app_context
            .db
            .load_contact_private_info(&identity.identity.id(), &contact_id)
            .unwrap_or((String::new(), String::new(), false));

        // Try to load cached contact profile from database
        let profile = if let Ok(contacts) = app_context
            .db
            .load_dashpay_contacts(&identity.identity.id())
        {
            contacts
                .iter()
                .find(|c| {
                    if let Ok(id) = Identifier::from_bytes(&c.contact_identity_id) {
                        id == contact_id
                    } else {
                        false
                    }
                })
                .map(|c| ContactPublicProfile {
                    identity_id: contact_id,
                    display_name: c.display_name.clone(),
                    public_message: c.public_message.clone(),
                    avatar_url: c.avatar_url.clone(),
                    avatar_hash: None,        // Not stored in contacts table yet
                    avatar_fingerprint: None, // Not stored in contacts table yet
                })
        } else {
            None
        };

        let initial_fetch_done = profile.is_some(); // Check before moving

        Self {
            app_context,
            identity,
            contact_id,
            profile,
            message: None,
            loading: false,
            initial_fetch_done, // If we have cached data, don't auto-fetch
            nickname,
            notes,
            is_hidden,
            editing_private_info: false,
        }
    }

    fn fetch_profile(&mut self) -> AppAction {
        self.loading = true;
        self.profile = None; // Clear any existing profile
        self.message = None; // Clear any existing message

        let task = BackendTask::DashPayTask(Box::new(DashPayTask::FetchContactProfile {
            identity: self.identity.clone(),
            contact_id: self.contact_id,
        }));

        AppAction::BackendTask(task)
    }

    fn save_private_info(&mut self) -> Result<(), String> {
        self.app_context
            .db
            .save_contact_private_info(
                &self.identity.identity.id(),
                &self.contact_id,
                &self.nickname,
                &self.notes,
                self.is_hidden,
            )
            .map_err(|e| e.to_string())
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Fetch profile on first render if not already done
        if !self.initial_fetch_done && !self.loading {
            self.initial_fetch_done = true;
            action = self.fetch_profile();
            // Return early with the fetch action
            return action;
        }

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Public Profile");
            ui.add_space(5.0);
            crate::ui::helpers::info_icon_button(
                ui,
                "About Public Profiles:\n\n\
                • This is the contact's public DashPay profile\n\
                • This information is published on Dash Platform\n\
                • Anyone can view this profile\n\
                • The contact controls what information to share\n\
                • This is different from your private notes about them",
            );
        });

        ui.separator();

        // Show message if any
        if let Some((message, message_type)) = &self.message {
            let color = match message_type {
                MessageType::Success => DashColors::success_color(dark_mode),
                MessageType::Error => DashColors::error_color(dark_mode),
                MessageType::Info => DashColors::DASH_BLUE,
            };
            ui.colored_label(color, message);
            ui.separator();
        }

        // Loading indicator
        if self.loading {
            ui.horizontal(|ui| {
                let spinner_color = if dark_mode {
                    egui::Color32::from_gray(200)
                } else {
                    egui::Color32::from_gray(60)
                };
                ui.add(egui::widgets::Spinner::default().color(spinner_color));
                ui.label("Loading public profile...");
            });
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {
            if let Some(profile) = &self.profile {
                // Profile header
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        // Avatar placeholder or image (fixed width)
                        ui.allocate_ui_with_layout(
                            egui::vec2(100.0, 120.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                if let Some(avatar_url) = &profile.avatar_url {
                                    // In production, would load and display actual image
                                    ui.label(RichText::new("🖼️").size(60.0));
                                    ui.label(
                                        RichText::new("Avatar")
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                    ui.label(
                                        RichText::new(avatar_url)
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode))
                                            .italics(),
                                    );
                                } else {
                                    ui.label(RichText::new("👤").size(60.0));
                                    ui.label(
                                        RichText::new("No avatar")
                                            .small()
                                            .color(DashColors::text_secondary(dark_mode)),
                                    );
                                }
                            }
                        );

                        ui.separator();

                        // Main content area (takes remaining space)
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            // Display name
                            if let Some(display_name) = &profile.display_name {
                                ui.label(
                                    RichText::new(display_name)
                                        .heading()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            } else {
                                ui.label(
                                    RichText::new("No display name set")
                                        .heading()
                                        .color(DashColors::text_secondary(dark_mode))
                                        .italics(),
                                );
                            }

                            // Identity ID
                            use dash_sdk::dpp::platform_value::string_encoding::Encoding;
                            ui.label(
                                RichText::new(format!(
                                    "Identity: {}",
                                    profile.identity_id.to_string(Encoding::Base58)
                                ))
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                            );

                            ui.add_space(10.0);

                            // Public message
                            ui.label(
                                RichText::new("Public Message:")
                                    .strong()
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            if let Some(public_message) = &profile.public_message {
                                ui.label(
                                    RichText::new(public_message)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            } else {
                                ui.label(
                                    RichText::new("No public message")
                                        .color(DashColors::text_secondary(dark_mode))
                                        .italics(),
                                );
                            }
                        });
                    });
                });

                ui.add_space(10.0);

                // Additional profile details if available
                if profile.avatar_hash.is_some() || profile.avatar_fingerprint.is_some() {
                    ui.group(|ui| {
                        ui.label(
                            RichText::new("Avatar Verification")
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.separator();

                        if let Some(hash) = &profile.avatar_hash {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Hash:")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                ui.label(
                                    RichText::new(hex::encode(hash))
                                        .small()
                                        .monospace()
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            });
                        }

                        if let Some(fingerprint) = &profile.avatar_fingerprint {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Fingerprint:")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                ui.label(
                                    RichText::new(hex::encode(fingerprint))
                                        .small()
                                        .monospace()
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            });
                        }
                    });
                }

                ui.add_space(10.0);

                // Action buttons
                ui.horizontal(|ui| {
                    if ui.button("Refresh Profile").clicked() {
                        action = self.fetch_profile();
                    }

                    let pay_button = egui::Button::new(
                        RichText::new("Pay")
                            .color(egui::Color32::WHITE)
                    ).fill(egui::Color32::from_rgb(0, 141, 228)); // Dash blue

                    if ui.add(pay_button).clicked() {
                        action = AppAction::AddScreen(
                            ScreenType::DashPaySendPayment(
                                self.identity.clone(),
                                self.contact_id,
                            )
                            .create_screen(&self.app_context),
                        );
                    }
                });
            } else if !self.loading {
                // No profile loaded and not loading
                ui.group(|ui| {
                    ui.label(
                        RichText::new("No profile found")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.separator();
                    ui.label("This contact has not created a public profile yet.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Retry").clicked() {
                            action = self.fetch_profile();
                        }

                        let pay_button = egui::Button::new(
                            RichText::new("Pay")
                                .color(egui::Color32::WHITE)
                        ).fill(egui::Color32::from_rgb(0, 141, 228)); // Dash blue

                        if ui.add(pay_button).clicked() {
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
            }

            // Private Contact Info Section - Always show this, regardless of whether profile exists
            if !self.loading {
                ui.add_space(10.0);

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(9.0);
                            ui.label(
                                RichText::new("Private Contact Information")
                                    .strong()
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                        });

                        ui.add_space(5.0);

                        ui.vertical(|ui| {
                            ui.add_space(9.0);
                            crate::ui::helpers::info_icon_button(
                                ui,
                                "This information is stored locally on your device and is not shared with anyone.",
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.editing_private_info {
                                if ui.button("Save").clicked() {
                                    match self.save_private_info() {
                                        Ok(_) => {
                                            self.editing_private_info = false;
                                            self.message = Some(("Private info saved".to_string(), MessageType::Success));
                                        }
                                        Err(e) => {
                                            self.message = Some((format!("Failed to save: {}", e), MessageType::Error));
                                        }
                                    }
                                }
                                if ui.button("Cancel").clicked() {
                                    self.editing_private_info = false;
                                    // Reload from database
                                    if let Ok((nick, notes, hidden)) = self.app_context.db.load_contact_private_info(&self.identity.identity.id(), &self.contact_id) {
                                        self.nickname = nick;
                                        self.notes = notes;
                                        self.is_hidden = hidden;
                                    }
                                }
                            } else {
                                if ui.button("Edit").clicked() {
                                    self.editing_private_info = true;
                                }
                            }
                        });
                    });

                    ui.separator();

                    // Nickname field
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Nickname:")
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        if self.editing_private_info {
                            ui.text_edit_singleline(&mut self.nickname);
                        } else {
                            let display_text = if self.nickname.is_empty() {
                                RichText::new("Not set")
                                    .italics()
                                    .color(DashColors::text_secondary(dark_mode))
                            } else {
                                RichText::new(&self.nickname)
                                    .color(DashColors::text_primary(dark_mode))
                            };
                            ui.label(display_text);
                        }
                    });

                    // Notes field
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Notes:")
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        if self.editing_private_info {
                            ui.text_edit_multiline(&mut self.notes);
                        } else {
                            let display_text = if self.notes.is_empty() {
                                RichText::new("No notes")
                                    .italics()
                                    .color(DashColors::text_secondary(dark_mode))
                            } else {
                                RichText::new(&self.notes)
                                    .color(DashColors::text_primary(dark_mode))
                            };
                            ui.label(display_text);
                        }
                    });

                    // Hidden toggle
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Hidden:")
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        if self.editing_private_info {
                            ui.checkbox(&mut self.is_hidden, "Hide this contact from the main list");
                        } else {
                            ui.label(
                                RichText::new(if self.is_hidden { "Yes" } else { "No" })
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                        }
                    });
                });
            }
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.loading = false;
        self.message = Some((message.to_string(), message_type));
    }

    pub fn refresh(&mut self) {
        // Don't auto-fetch on refresh - just clear temporary states
        self.loading = false;
        self.message = None;
    }

    pub fn refresh_on_arrival(&mut self) {
        // Reset the initial fetch flag when arriving at the screen
        // The fetch will happen on the first render
        if self.profile.is_none() && !self.loading {
            self.initial_fetch_done = false;
        }
    }
}

impl ScreenLike for ContactProfileViewerScreen {
    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Contact Profile", AppAction::None),
            ],
            vec![],
        );

        // Add left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDashPayContacts,
        );

        action |= island_central_panel(ctx, |ui| self.render(ui));

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPayContactProfile(profile_doc) => {
                if let Some(doc) = profile_doc {
                    // Extract profile data from the document
                    use dash_sdk::dpp::document::DocumentV0Getters;
                    let properties = match &doc {
                        dash_sdk::platform::Document::V0(doc_v0) => doc_v0.properties(),
                    };

                    let display_name = properties
                        .get("displayName")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());
                    let public_message = properties
                        .get("publicMessage")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());
                    let avatar_url = properties
                        .get("avatarUrl")
                        .and_then(|v| v.as_text())
                        .map(|s| s.to_string());
                    let avatar_hash = properties
                        .get("avatarHash")
                        .and_then(|v| v.as_bytes().map(|b| b.to_vec()));
                    let avatar_fingerprint = properties
                        .get("avatarFingerprint")
                        .and_then(|v| v.as_bytes().map(|b| b.to_vec()));

                    self.profile = Some(ContactPublicProfile {
                        identity_id: self.contact_id,
                        display_name: display_name.clone(),
                        public_message: public_message.clone(),
                        avatar_url: avatar_url.clone(),
                        avatar_hash: avatar_hash.clone(),
                        avatar_fingerprint: avatar_fingerprint.clone(),
                    });

                    // Save the contact profile to the database
                    if let Err(e) = self.app_context.db.save_dashpay_contact(
                        &self.identity.identity.id(),
                        &self.contact_id,
                        None, // username will be fetched separately if needed
                        display_name.as_deref(),
                        avatar_url.as_deref(),
                        public_message.as_deref(),
                        "accepted", // Status is accepted since we can view their profile
                    ) {
                        eprintln!("Failed to save contact profile to database: {}", e);
                    }

                    self.message = None;
                } else {
                    self.profile = None;
                    self.message = None; // Don't set message here, UI already shows "No profile found"
                }
            }
            BackendTaskSuccessResult::Message(msg) => {
                self.message = Some((msg, MessageType::Info));
            }
            _ => {
                self.message = Some(("Unexpected response type".to_string(), MessageType::Error));
            }
        }
    }
}
