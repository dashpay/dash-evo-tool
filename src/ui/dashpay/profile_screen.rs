use crate::app::AppAction;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::backend_task::dashpay::DashPayTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::MessageType;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct DashPayProfile {
    pub display_name: String,
    pub bio: String,
    pub avatar_url: String,
}

pub struct ProfileScreen {
    app_context: Arc<AppContext>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    profile: Option<DashPayProfile>,
    editing: bool,
    edit_display_name: String,
    edit_bio: String,
    edit_avatar_url: String,
    message: Option<(String, MessageType)>,
    loading: bool,
    saving: bool,  // Track if we're saving vs loading
    profile_load_attempted: bool,
}

impl ProfileScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_identity: None,
            selected_identity_string: String::new(),
            profile: None,
            editing: false,
            edit_display_name: String::new(),
            edit_bio: String::new(),
            edit_avatar_url: String::new(),
            message: None,
            loading: false,
            saving: false,
            profile_load_attempted: false,
        }
    }

    pub fn trigger_load_profile(&mut self) -> AppAction {
        if let Some(identity) = self.selected_identity.clone() {
            self.loading = true;
            self.profile_load_attempted = true;
            AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                DashPayTask::LoadProfile { identity }
            )))
        } else {
            AppAction::None
        }
    }

    pub fn refresh(&mut self) {
        // Don't set loading here - it will be set when actually triggering a backend task
        // This prevents stuck loading states
        self.loading = false;
        
        // Clear any old messages
        self.message = None;
    }

    fn start_editing(&mut self) {
        if let Some(profile) = &self.profile {
            self.edit_display_name = profile.display_name.clone();
            self.edit_bio = profile.bio.clone();
            self.edit_avatar_url = profile.avatar_url.clone();
            self.editing = true;
        } else {
            // New profile
            self.edit_display_name.clear();
            self.edit_bio.clear();
            self.edit_avatar_url.clear();
            self.editing = true;
        }
    }

    fn save_profile(&mut self) -> AppAction {
        if let Some(identity) = self.selected_identity.clone() {
            self.editing = false;
            self.saving = true;  // Set saving flag instead of loading
            
            // Trigger the actual DashPay profile update task
            AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                DashPayTask::UpdateProfile {
                    identity,
                    display_name: if self.edit_display_name.is_empty() { None } else { Some(self.edit_display_name.clone()) },
                    bio: if self.edit_bio.is_empty() { None } else { Some(self.edit_bio.clone()) },
                    avatar_url: if self.edit_avatar_url.is_empty() { None } else { Some(self.edit_avatar_url.clone()) },
                }
            )))
        } else {
            self.display_message("No identity selected", MessageType::Error);
            AppAction::None
        }
    }

    fn cancel_editing(&mut self) {
        self.editing = false;
        self.edit_display_name.clear();
        self.edit_bio.clear();
        self.edit_avatar_url.clear();
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.heading("My DashPay Profile");
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
                        "profile_identity_selector",
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
                    // Reset state when identity changes
                    self.profile = None;
                    self.profile_load_attempted = false;
                    self.loading = false;
                    self.message = None;
                }
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

        // No identity selected or no identities available
        if identities.is_empty() {
            return action;
        }

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view or edit profile");
            return action;
        }

        // Profile loading status
        if !self.profile_load_attempted && !self.loading {
            ui.label("Press \"Load Profile\" in the top panel");
        }
        
        // Loading or saving indicator
        if self.loading || self.saving {
            ui.horizontal(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let spinner_color = if dark_mode {
                    egui::Color32::from_gray(200)
                } else {
                    egui::Color32::from_gray(60)
                };
                ui.add(egui::widgets::Spinner::default().color(spinner_color));
                let status_text = if self.saving {
                    "Saving profile..."
                } else {
                    "Loading profile..."
                };
                ui.label(
                    RichText::new(status_text)
                        .color(DashColors::text_primary(dark_mode))
                );
            });
            ui.separator();
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {
            if self.editing {
                // Edit mode
                ui.group(|ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Edit Profile").strong().color(DashColors::text_primary(dark_mode)));
                        ui.add_space(10.0);
                        crate::ui::helpers::info_icon_button(ui, 
                            "Profile Guidelines:\n\n\
                            • Display names can include any UTF-8 characters (emojis, symbols, etc.)\n\
                            • Display names are limited to 25 characters\n\
                            • Bios are limited to 250 characters\n\
                            • Avatar URLs should point to publicly accessible images\n\
                            • Profiles are public and visible to all DashPay users");
                    });

                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Display Name:").color(DashColors::text_primary(dark_mode)));
                        ui.add_space(10.0);
                    });
                    ui.add(
                        TextEdit::singleline(&mut self.edit_display_name)
                            .hint_text("Enter your display name (max 25 characters)")
                            .desired_width(300.0),
                    );
                    ui.label(RichText::new(format!("{}/25", self.edit_display_name.len())).small().color(DashColors::text_secondary(dark_mode)));

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Bio/Status:").color(DashColors::text_primary(dark_mode)));
                        ui.add_space(10.0);
                    });
                    ui.add(
                        TextEdit::multiline(&mut self.edit_bio)
                            .hint_text("Tell others about yourself (max 250 characters)")
                            .desired_width(300.0)
                            .desired_rows(4),
                    );
                    ui.label(RichText::new(format!("{}/250", self.edit_bio.len())).small().color(DashColors::text_secondary(dark_mode)));

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Avatar URL:").color(DashColors::text_primary(dark_mode)));
                        ui.add_space(10.0);
                    });
                    ui.add(
                        TextEdit::singleline(&mut self.edit_avatar_url)
                            .hint_text("https://example.com/avatar.jpg")
                            .desired_width(300.0),
                    );

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.cancel_editing();
                        }

                        if ui.button("Save Profile").clicked() {
                            // Validation
                            if self.edit_display_name.len() > 25 {
                                self.display_message(
                                    "Display name must be 25 characters or less",
                                    MessageType::Error,
                                );
                            } else if self.edit_bio.len() > 250 {
                                self.display_message(
                                    "Bio must be 250 characters or less",
                                    MessageType::Error,
                                );
                            } else if !self.edit_avatar_url.is_empty()
                                && !self.edit_avatar_url.starts_with("http")
                            {
                                self.display_message(
                                    "Avatar URL must start with http:// or https://",
                                    MessageType::Error,
                                );
                            } else {
                                action |= self.save_profile();
                            }
                        }
                    });
                });
            } else {
                // View mode
                if let Some(profile) = self.profile.clone() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            // Avatar placeholder
                            ui.vertical(|ui| {
                                ui.add_space(5.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(10.0);
                                    ui.label(RichText::new("👤").size(50.0));
                                });
                            });

                            ui.vertical(|ui| {
                                // Display name
                                if !profile.display_name.is_empty() {
                                    ui.label(RichText::new(&profile.display_name).heading());
                                } else {
                                    ui.label(RichText::new("No display name set").weak());
                                }

                                // Username from identity
                                if let Some(identity) = &self.selected_identity {
                                    if !identity.dpns_names.is_empty() {
                                        ui.label(
                                            RichText::new(format!(
                                                "@{}",
                                                identity.dpns_names[0].name
                                            ))
                                            .strong(),
                                        );
                                    }
                                }

                                // Identity ID
                                if let Some(identity) = &self.selected_identity {
                                    ui.label(
                                        RichText::new(format!("ID: {}", identity.identity.id()))
                                            .small()
                                            .weak(),
                                    );
                                }
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                                if ui.button("Edit Profile").clicked() {
                                    self.start_editing();
                                }
                            });
                        });

                        ui.separator();

                        // Bio
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.label(RichText::new("Bio:").strong().color(DashColors::text_primary(dark_mode)));
                        if !profile.bio.is_empty() {
                            ui.label(RichText::new(&profile.bio).color(DashColors::text_primary(dark_mode)));
                        } else {
                            ui.label(RichText::new("No bio set").color(DashColors::text_secondary(dark_mode)));
                        }

                        ui.separator();

                        // Avatar URL
                        ui.label(RichText::new("Avatar URL:").strong().color(DashColors::text_primary(dark_mode)));
                        if !profile.avatar_url.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&profile.avatar_url).color(DashColors::text_primary(dark_mode)));
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(profile.avatar_url.clone());
                                    self.display_message(
                                        "Avatar URL copied to clipboard",
                                        MessageType::Info,
                                    );
                                }
                            });
                        } else {
                            ui.label(RichText::new("No avatar URL set").color(DashColors::text_secondary(dark_mode)));
                        }
                    });
                } else if self.profile_load_attempted {
                    // No profile exists (only show after we've tried to load)
                    ui.group(|ui| {
                        ui.label("No DashPay profile found for this identity.");
                        ui.add_space(10.0);
                        if ui.button("Create Profile").clicked() {
                            self.start_editing();
                        }
                    });
                }

            }
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
        // Clear loading/saving states on error
        if message_type == MessageType::Error {
            self.loading = false;
            self.saving = false;
        }
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // Always clear loading and saving states first
        self.loading = false;
        self.saving = false;
        self.profile_load_attempted = true;
        
        match result {
            BackendTaskSuccessResult::DashPayProfile(profile_data) => {
                if let Some((display_name, bio, avatar_url)) = profile_data {
                    self.profile = Some(DashPayProfile {
                        display_name,
                        bio,
                        avatar_url,
                    });
                    // Profile loaded successfully - no need to show a message
                } else {
                    // No profile found - clear any existing profile and show create button
                    self.profile = None;
                    // Don't show a message - let the UI show "Create Profile" button
                }
            }
            BackendTaskSuccessResult::Message(message) => {
                if message.contains("successfully") {
                    self.display_message(&message, MessageType::Success);
                    // After successful profile update, reset flag so user can reload
                    self.profile_load_attempted = false;
                } else {
                    self.display_message(&message, MessageType::Info);
                }
            }
            _ => {
                self.display_message("Operation completed", MessageType::Success);
            }
        }
    }
}
