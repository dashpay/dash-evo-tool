use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
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

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    DisplayNameTooLong(usize),
    DisplayNameEmpty,
    BioTooLong(usize),
    InvalidAvatarUrl(String),
    AvatarUrlTooLong(usize),
}

impl ValidationError {
    pub fn message(&self) -> String {
        match self {
            ValidationError::DisplayNameTooLong(len) => {
                format!("Display name is {} characters, must be 25 or less", len)
            }
            ValidationError::DisplayNameEmpty => "Display name cannot be empty".to_string(),
            ValidationError::BioTooLong(len) => {
                format!("Bio is {} characters, must be 250 or less", len)
            }
            ValidationError::InvalidAvatarUrl(url) => {
                format!(
                    "Invalid avatar URL: '{}'. Must start with http:// or https://",
                    url
                )
            }
            ValidationError::AvatarUrlTooLong(len) => {
                format!("Avatar URL is {} characters, must be 500 or less", len)
            }
        }
    }
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
    saving: bool, // Track if we're saving vs loading
    profile_load_attempted: bool,
    validation_errors: Vec<ValidationError>,
    has_unsaved_changes: bool,
    original_display_name: String,
    original_bio: String,
    original_avatar_url: String,
}

impl ProfileScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let mut new_self = Self {
            app_context: app_context.clone(),
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
            validation_errors: Vec::new(),
            has_unsaved_changes: false,
            original_display_name: String::new(),
            original_bio: String::new(),
            original_avatar_url: String::new(),
        };


        // Auto-select identity on creation - prefer one with a profile
        if let Ok(identities) = app_context.load_local_qualified_identities() {
            if !identities.is_empty() {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                
                // Try to find an identity with an actual profile (not just a "no profile" marker)
                let mut selected_idx = 0;
                for (idx, identity) in identities.iter().enumerate() {
                    let identity_id = identity.identity.id();
                    if let Ok(Some(profile)) = app_context.db.load_dashpay_profile(&identity_id) {
                        // Check if this is an actual profile with data (not a "no profile" marker)
                        if profile.display_name.is_some() || profile.bio.is_some() || profile.avatar_url.is_some() {
                            selected_idx = idx;
                            break;
                        }
                    }
                }
                
                new_self.selected_identity = Some(identities[selected_idx].clone());
                new_self.selected_identity_string = identities[selected_idx]
                    .identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

                // Load profile from database for this identity
                new_self.load_profile_from_database();
            }
        }

        new_self
    }

    fn validate_profile(&mut self) {
        self.validation_errors.clear();

        // Display name validation
        if self.edit_display_name.trim().is_empty() {
            self.validation_errors
                .push(ValidationError::DisplayNameEmpty);
        } else if self.edit_display_name.len() > 25 {
            self.validation_errors
                .push(ValidationError::DisplayNameTooLong(
                    self.edit_display_name.len(),
                ));
        }

        // Bio validation
        if self.edit_bio.len() > 250 {
            self.validation_errors
                .push(ValidationError::BioTooLong(self.edit_bio.len()));
        }

        // Avatar URL validation
        if !self.edit_avatar_url.trim().is_empty() {
            let url = self.edit_avatar_url.trim();
            if url.len() > 500 {
                self.validation_errors
                    .push(ValidationError::AvatarUrlTooLong(url.len()));
            } else if !url.starts_with("http://") && !url.starts_with("https://") {
                self.validation_errors
                    .push(ValidationError::InvalidAvatarUrl(url.to_string()));
            }
        }
    }

    fn check_for_changes(&mut self) {
        self.has_unsaved_changes = self.edit_display_name != self.original_display_name
            || self.edit_bio != self.original_bio
            || self.edit_avatar_url != self.original_avatar_url;
    }

    fn is_valid(&self) -> bool {
        self.validation_errors.is_empty()
    }

    fn load_profile_from_database(&mut self) {
        // Load saved profile for the selected identity from database
        if let Some(identity) = &self.selected_identity {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            let identity_id = identity.identity.id();

            // Load profile from database
            match self.app_context.db.load_dashpay_profile(&identity_id) {
                Ok(Some(stored_profile)) => {
                    // Check if this is a "no profile exists" marker (all fields are None)
                    if stored_profile.display_name.is_none() 
                        && stored_profile.bio.is_none() 
                        && stored_profile.avatar_url.is_none() {
                        // This is a cached "no profile" state
                        self.profile = None;
                        self.profile_load_attempted = true;
                    } else {
                        // This is an actual profile with data
                        self.profile = Some(DashPayProfile {
                            display_name: stored_profile.display_name.unwrap_or_default(),
                            bio: stored_profile.bio.unwrap_or_default(),
                            avatar_url: stored_profile.avatar_url.unwrap_or_default(),
                        });

                        // Update edit fields with loaded profile
                        if let Some(ref profile) = self.profile {
                            self.edit_display_name = profile.display_name.clone();
                            self.edit_bio = profile.bio.clone();
                            self.edit_avatar_url = profile.avatar_url.clone();
                            
                            // Store original values for change detection
                            self.original_display_name = profile.display_name.clone();
                            self.original_bio = profile.bio.clone();
                            self.original_avatar_url = profile.avatar_url.clone();
                        }

                        // Mark as loaded from cache
                        self.profile_load_attempted = true;
                    }
                }
                Ok(None) => {
                }
                Err(e) => {
                }
            }
        } else {
        }
    }

    pub fn trigger_load_profile(&mut self) -> AppAction {
        if let Some(identity) = self.selected_identity.clone() {
            self.loading = true;
            self.profile_load_attempted = true;
            AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                DashPayTask::LoadProfile { identity },
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

        // Auto-select first identity if none selected
        if self.selected_identity.is_none() {
            if let Ok(identities) = self.app_context.load_local_qualified_identities() {
                if !identities.is_empty() {
                    self.selected_identity = Some(identities[0].clone());
                    self.selected_identity_string = identities[0].display_string();
                }
            }
        }

        // Load profile from database if we have an identity selected and no profile loaded
        if self.selected_identity.is_some()
            && self.profile.is_none()
            && !self.profile_load_attempted
        {
            self.load_profile_from_database();
        }
    }

    fn start_editing(&mut self) {
        if let Some(profile) = &self.profile {
            self.edit_display_name = profile.display_name.clone();
            self.edit_bio = profile.bio.clone();
            self.edit_avatar_url = profile.avatar_url.clone();

            // Store originals for change detection
            self.original_display_name = profile.display_name.clone();
            self.original_bio = profile.bio.clone();
            self.original_avatar_url = profile.avatar_url.clone();
        } else {
            // New profile
            self.edit_display_name.clear();
            self.edit_bio.clear();
            self.edit_avatar_url.clear();

            // Store empty originals
            self.original_display_name.clear();
            self.original_bio.clear();
            self.original_avatar_url.clear();
        }

        self.editing = true;
        self.has_unsaved_changes = false;
        self.validation_errors.clear();
        self.message = None;
    }

    fn save_profile(&mut self) -> AppAction {
        self.validate_profile();

        if !self.is_valid() {
            self.display_message(&self.validation_errors[0].message(), MessageType::Error);
            return AppAction::None;
        }

        if let Some(identity) = self.selected_identity.clone() {
            self.editing = false;
            self.saving = true;
            self.has_unsaved_changes = false;

            // Trim whitespace from inputs
            let display_name = self.edit_display_name.trim();
            let bio = self.edit_bio.trim();
            let avatar_url = self.edit_avatar_url.trim();

            // Trigger the actual DashPay profile update task
            AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                DashPayTask::UpdateProfile {
                    identity,
                    display_name: if display_name.is_empty() {
                        None
                    } else {
                        Some(display_name.to_string())
                    },
                    bio: if bio.is_empty() {
                        None
                    } else {
                        Some(bio.to_string())
                    },
                    avatar_url: if avatar_url.is_empty() {
                        None
                    } else {
                        Some(avatar_url.to_string())
                    },
                },
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
        self.validation_errors.clear();
        self.has_unsaved_changes = false;
        self.message = None;
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
                    self.editing = false;
                    self.validation_errors.clear();
                    self.has_unsaved_changes = false;
                    self.message = None;
                    
                    // Load profile from database for the newly selected identity
                    self.load_profile_from_database();
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
            ui.label("No profile loaded");
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
                ui.label(RichText::new(status_text).color(DashColors::text_primary(dark_mode)));
            });
            return action;
        } else {
            ScrollArea::vertical().show(ui, |ui| {
            if self.editing {
                // Edit mode
                ui.horizontal(|ui| {
                    // Main editing panel (left side)
                    ui.vertical(|ui| {
                        ui.group(|ui| {
                            let dark_mode = ui.ctx().style().visuals.dark_mode;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Edit Profile").strong().color(DashColors::text_primary(dark_mode)));

                                ui.add_space(5.0);
                                crate::ui::helpers::info_icon_button(ui,
                                    "Profile Guidelines:\n\n\
                                    • Display names can include any UTF-8 characters (emojis, symbols, etc.)\n\
                                    • Display names are limited to 25 characters\n\
                                    • Bios are limited to 250 characters\n\
                                    • Avatar URLs should point to publicly accessible images (max 500 chars)\n\
                                    • Profiles are public and visible to all DashPay users");
                            });

                            ui.separator();

                            // Unsaved changes indicator
                            if self.has_unsaved_changes {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("⚠").color(egui::Color32::ORANGE));
                                    ui.label(RichText::new("You have unsaved changes").color(egui::Color32::ORANGE).small());
                                });
                                ui.separator();
                            }

                            // Display Name Field
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Display Name:").color(DashColors::text_primary(dark_mode)));
                                ui.label(RichText::new("*").color(egui::Color32::RED)); // Required indicator
                            });

                            let display_name_response = ui.add(
                                TextEdit::singleline(&mut self.edit_display_name)
                                    .hint_text("Enter your display name (required)")
                                    .desired_width(300.0),
                            );

                            // Character count with color coding
                            let char_count = self.edit_display_name.len();
                            let count_color = if char_count > 25 {
                                egui::Color32::RED
                            } else if char_count > 20 {
                                egui::Color32::ORANGE
                            } else {
                                DashColors::text_secondary(dark_mode)
                            };
                            ui.label(RichText::new(format!("{}/25", char_count)).small().color(count_color));

                            if display_name_response.changed() {
                                self.check_for_changes();
                                self.validate_profile();
                            }

                            ui.add_space(10.0);

                            // Bio Field
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Bio/Status:").color(DashColors::text_primary(dark_mode)));
                            });

                            let bio_response = ui.add(
                                TextEdit::multiline(&mut self.edit_bio)
                                    .hint_text("Tell others about yourself (optional)")
                                    .desired_width(300.0)
                                    .desired_rows(4),
                            );

                            // Bio character count with color coding
                            let bio_count = self.edit_bio.len();
                            let bio_count_color = if bio_count > 250 {
                                egui::Color32::RED
                            } else if bio_count > 225 {
                                egui::Color32::ORANGE
                            } else {
                                DashColors::text_secondary(dark_mode)
                            };
                            ui.label(RichText::new(format!("{}/250", bio_count)).small().color(bio_count_color));

                            if bio_response.changed() {
                                self.check_for_changes();
                                self.validate_profile();
                            }

                            ui.add_space(10.0);

                            // Avatar URL Field
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Avatar URL:").color(DashColors::text_primary(dark_mode)));
                            });

                            let avatar_response = ui.add(
                                TextEdit::singleline(&mut self.edit_avatar_url)
                                    .hint_text("https://example.com/avatar.jpg (optional)")
                                    .desired_width(300.0),
                            );

                            // Avatar URL character count
                            let url_count = self.edit_avatar_url.len();
                            let url_count_color = if url_count > 500 {
                                egui::Color32::RED
                            } else if url_count > 450 {
                                egui::Color32::ORANGE
                            } else {
                                DashColors::text_secondary(dark_mode)
                            };
                            if !self.edit_avatar_url.is_empty() {
                                ui.label(RichText::new(format!("{}/500", url_count)).small().color(url_count_color));
                            }

                            if avatar_response.changed() {
                                self.check_for_changes();
                                self.validate_profile();
                            }

                            // Show validation errors
                            if !self.validation_errors.is_empty() {
                                ui.add_space(10.0);
                                ui.separator();
                                ui.label(RichText::new("Validation Errors:").color(egui::Color32::RED).strong());
                                for error in &self.validation_errors {
                                    ui.label(RichText::new(format!("• {}", error.message())).color(egui::Color32::RED).small());
                                }
                            }

                            ui.add_space(15.0);

                            // Action buttons
                            ui.horizontal(|ui| {
                                if ui.button("Cancel").clicked() {
                                    // Show confirmation if there are unsaved changes
                                    if self.has_unsaved_changes {
                                        // TODO: Add confirmation dialog
                                        self.cancel_editing();
                                    } else {
                                        self.cancel_editing();
                                    }
                                }

                                ui.add_space(10.0);

                                let save_button = egui::Button::new(
                                    RichText::new("Save Profile")
                                        .color(egui::Color32::WHITE)
                                ).fill(if self.is_valid() {
                                        egui::Color32::from_rgb(0, 141, 228) // Dash blue
                                    } else {
                                        egui::Color32::GRAY
                                    });

                                if ui.add_enabled(self.is_valid(), save_button).clicked() {
                                    action |= self.save_profile();
                                }

                                // Show save status
                                if self.has_unsaved_changes {
                                    ui.add_space(10.0);
                                    ui.label(RichText::new("Unsaved").color(egui::Color32::ORANGE).small());
                                }
                            });
                        });
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
                                let edit_button = egui::Button::new(
                                    RichText::new("Edit Profile")
                                        .color(egui::Color32::WHITE)
                                ).fill(egui::Color32::from_rgb(0, 141, 228)); // Dash blue

                                if ui.add(edit_button).clicked() {
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
                        let create_button = egui::Button::new(
                            RichText::new("Create Profile")
                                .color(egui::Color32::WHITE)
                        ).fill(egui::Color32::from_rgb(0, 141, 228)); // Dash blue

                        if ui.add(create_button).clicked() {
                            self.start_editing();
                        }
                    });
                }

            }
        });
        }

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
                        display_name: display_name.clone(),
                        bio: bio.clone(),
                        avatar_url: avatar_url.clone(),
                    });

                    // Save profile to database for caching
                    if let Some(ref identity) = self.selected_identity {
                        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                        let identity_id = identity.identity.id();
                        
                        if let Err(e) = self.app_context.db.save_dashpay_profile(
                            &identity_id,
                            Some(&display_name),
                            Some(&bio),
                            Some(&avatar_url),
                            None, // public_message not used in profile screen yet
                        ) {
                            eprintln!("Failed to cache profile in database: {}", e);
                        }
                    }
                    // Profile loaded successfully - no need to show a message
                } else {
                    // No profile found - clear any existing profile and show create button
                    self.profile = None;
                    
                    // Save "no profile" state to database to avoid repeated network queries
                    if let Some(ref identity) = self.selected_identity {
                        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                        let identity_id = identity.identity.id();
                        
                        // Save with all fields as None to indicate "no profile exists"
                        // This prevents unnecessary network queries on app restart
                        if let Err(e) = self.app_context.db.save_dashpay_profile(
                            &identity_id,
                            None, // display_name
                            None, // bio
                            None, // avatar_url
                            None, // public_message
                        ) {
                            eprintln!("Failed to cache 'no profile' state in database: {}", e);
                        }
                    }
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
