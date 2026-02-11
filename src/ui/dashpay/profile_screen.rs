use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::MessageType;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::get_selected_wallet;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{ColorImage, Frame, Margin, RichText, ScrollArea, TextEdit, TextureHandle, Ui};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

const PROFILE_GUIDELINES_INFO_TEXT: &str = "Profile Guidelines:\n\n\
    Display names can include any UTF-8 characters (emojis, symbols, etc.).\n\n\
    Display names are limited to 25 characters.\n\n\
    Bios are limited to 140 characters.\n\n\
    Avatar URLs should point to publicly accessible images (max 500 chars).\n\n\
    Profiles are public and visible to all DashPay users.";

const AVATAR_URL_INFO_TEXT: &str = "Avatar Image Guidelines:\n\n\
    The URL must point to a publicly accessible image.\n\n\
    Recommended: Square images (e.g., 256x256 or 512x512 pixels).\n\n\
    Supported formats: JPEG, PNG, WebP, or GIF.\n\n\
    Maximum URL length: 500 characters.\n\n\
    Example URL:\nhttps://example.com/images/avatar.jpg\n\n\
    Tip: Use image hosting services like Imgur, Cloudinary, or your own server.";

#[derive(Debug, Clone)]
pub struct DashPayProfile {
    pub display_name: String,
    pub bio: String,
    pub avatar_url: String,
    pub avatar_bytes: Option<Vec<u8>>,
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
                format!("Bio is {} characters, must be 140 or less", len)
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
    pub app_context: Arc<AppContext>,
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
    avatar_textures: HashMap<String, TextureHandle>, // Cache for avatar textures
    avatar_loading: bool,                            // Track if avatar is being loaded
    pending_action: Option<Box<AppAction>>,          // Action to execute on next frame
    show_info_popup: bool,
    show_avatar_info_popup: bool,
    show_avatar_url_popup: bool, // Show avatar URL when clicking on avatar in view mode
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    show_success: bool,
    was_creating_new: bool, // Track if we were creating vs updating
    confirmation_dialog: Option<ConfirmationDialog>,
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
            avatar_textures: HashMap::new(),
            avatar_loading: false,
            pending_action: None,
            show_info_popup: false,
            show_avatar_info_popup: false,
            show_avatar_url_popup: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_success: false,
            was_creating_new: false,
            confirmation_dialog: None,
        };

        // Auto-select identity on creation - prefer one with a profile
        if let Ok(identities) = app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

            // Try to find an identity with an actual profile (not just a "no profile" marker)
            let network_str = app_context.network.to_string();
            tracing::info!(
                "ProfileScreen::new - checking {} identities on network {}",
                identities.len(),
                network_str
            );

            let mut selected_idx = 0;
            for (idx, identity) in identities.iter().enumerate() {
                let identity_id = identity.identity.id();
                tracing::debug!("Checking identity {} for profile in DB", identity_id);
                match app_context
                    .db
                    .load_dashpay_profile(&identity_id, &network_str)
                {
                    Ok(Some(profile)) => {
                        tracing::debug!(
                            "Found profile for identity {}: display_name={:?}",
                            identity_id,
                            profile.display_name
                        );
                        if profile.display_name.is_some()
                            || profile.bio.is_some()
                            || profile.avatar_url.is_some()
                        {
                            // Check if this is an actual profile with data (not a "no profile" marker)
                            selected_idx = idx;
                            tracing::info!("Selected identity {} with profile", identity_id);
                            break;
                        }
                    }
                    Ok(None) => {
                        tracing::debug!("No profile in DB for identity {}", identity_id);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Error loading profile for identity {}: {}",
                            identity_id,
                            e
                        );
                    }
                }
            }

            new_self.selected_identity = Some(identities[selected_idx].clone());
            new_self.selected_identity_string = identities[selected_idx]
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

            tracing::info!(
                "ProfileScreen::new - selected identity {}",
                new_self.selected_identity_string
            );

            // Get wallet for the selected identity
            let mut error_message = None;
            new_self.selected_wallet = get_selected_wallet(
                &identities[selected_idx],
                Some(&app_context),
                None,
                &mut error_message,
            );

            // Load profile from database for this identity
            new_self.load_profile_from_database();
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
        if self.edit_bio.len() > 140 {
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
            let network_str = self.app_context.network.to_string();

            tracing::debug!(
                "Loading profile from database for identity {} on network {}",
                identity_id,
                network_str
            );

            // Load profile from database
            match self
                .app_context
                .db
                .load_dashpay_profile(&identity_id, &network_str)
            {
                Ok(Some(stored_profile)) => {
                    tracing::debug!(
                        "Found profile in database: display_name={:?}, bio={:?}, avatar_url={:?}",
                        stored_profile.display_name,
                        stored_profile.bio,
                        stored_profile.avatar_url
                    );
                    // Check if this is a "no profile exists" marker (all fields are None)
                    if stored_profile.display_name.is_none()
                        && stored_profile.bio.is_none()
                        && stored_profile.avatar_url.is_none()
                    {
                        // This is a cached "no profile" state
                        self.profile = None;
                        self.profile_load_attempted = true;
                    } else {
                        // This is an actual profile with data
                        self.profile = Some(DashPayProfile {
                            display_name: stored_profile.display_name.unwrap_or_default(),
                            bio: stored_profile.bio.unwrap_or_default(),
                            avatar_url: stored_profile.avatar_url.unwrap_or_default(),
                            avatar_bytes: stored_profile.avatar_bytes,
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
                    tracing::debug!("No profile found in database for identity {}", identity_id);
                }
                Err(e) => {
                    tracing::error!("Error loading profile from database: {}", e);
                }
            }
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
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            self.selected_identity = Some(identities[0].clone());
            self.selected_identity_string = identities[0].display_string();
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
            // Track if this is a new profile creation
            self.was_creating_new = self.profile.is_none();
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

    /// Load avatar texture from network (fetches bytes and processes them)
    fn load_avatar_texture(&mut self, ctx: &egui::Context, url: &str) {
        let ctx_clone = ctx.clone();
        let url_clone = url.to_string();

        // Spawn async task to fetch and load the image
        tokio::spawn(async move {
            match crate::backend_task::dashpay::avatar_processing::fetch_image_bytes(&url_clone)
                .await
            {
                Ok(image_bytes) => {
                    Self::process_avatar_bytes_async(ctx_clone, url_clone, image_bytes, true);
                }
                Err(e) => {
                    eprintln!("Failed to fetch avatar image: {}", e);
                }
            }
        });
    }

    /// Load avatar texture from cached bytes synchronously
    /// Returns the ColorImage if successful, or None if processing failed
    fn process_avatar_bytes_sync(image_bytes: &[u8]) -> Option<ColorImage> {
        // Try to load the image
        if let Ok(image) = image::load_from_memory(image_bytes) {
            // Convert to RGBA
            let rgba_image = image.to_rgba8();
            let width = rgba_image.width();
            let height = rgba_image.height();

            // Center-crop to square if not already square
            let cropped_image = if width != height {
                let size = width.min(height);
                let x_offset = (width - size) / 2;
                let y_offset = (height - size) / 2;
                image::imageops::crop_imm(&rgba_image, x_offset, y_offset, size, size).to_image()
            } else {
                rgba_image
            };

            let size = [
                cropped_image.width() as usize,
                cropped_image.height() as usize,
            ];
            let pixels = cropped_image.into_raw();

            Some(ColorImage::from_rgba_unmultiplied(size, &pixels))
        } else {
            None
        }
    }

    /// Process avatar bytes asynchronously and store result for UI thread
    /// If `from_network` is true, also stores the raw bytes for database caching
    fn process_avatar_bytes_async(
        ctx: egui::Context,
        url: String,
        image_bytes: Vec<u8>,
        from_network: bool,
    ) {
        // Try to load the image
        if let Ok(image) = image::load_from_memory(&image_bytes) {
            // Convert to RGBA
            let rgba_image = image.to_rgba8();
            let width = rgba_image.width();
            let height = rgba_image.height();

            // Center-crop to square if not already square
            let cropped_image = if width != height {
                let size = width.min(height);
                let x_offset = (width - size) / 2;
                let y_offset = (height - size) / 2;
                image::imageops::crop_imm(&rgba_image, x_offset, y_offset, size, size).to_image()
            } else {
                rgba_image
            };

            let size = [
                cropped_image.width() as usize,
                cropped_image.height() as usize,
            ];
            let pixels = cropped_image.into_raw();

            // Create ColorImage
            let color_image = ColorImage::from_rgba_unmultiplied(size, &pixels);

            // Request repaint to load texture in UI thread
            ctx.request_repaint();

            // Store the image data temporarily for the UI thread to pick up
            ctx.data_mut(|data| {
                data.insert_temp(egui::Id::new(format!("avatar_data_{}", url)), color_image);
                // Only store raw bytes if fetched from network (for database caching)
                if from_network {
                    data.insert_temp(egui::Id::new(format!("avatar_bytes_{}", url)), image_bytes);
                }
            });
        }
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let success_message = if self.was_creating_new {
            "DashPay Profile Created Successfully!"
        } else {
            "DashPay Profile Updated Successfully!"
        };

        let action = crate::ui::helpers::show_success_screen(
            ui,
            success_message.to_string(),
            vec![(
                "View Profile".to_string(),
                AppAction::Custom("view_profile".to_string()),
            )],
        );

        // Handle the custom action
        if let AppAction::Custom(ref s) = action
            && s == "view_profile"
        {
            self.show_success = false;
            self.profile_load_attempted = true; // We already have the profile in memory
            // Profile is already in self.profile from display_task_result, no need to reload
            return AppAction::None;
        }

        action
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Check for pending action from previous frame
        if let Some(pending) = self.pending_action.take() {
            action = *pending;
        }

        // Show success screen if profile was just created/updated
        if self.show_success {
            return self.show_success_screen(ui);
        }

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        // Header with identity selector on the right
        ui.horizontal(|ui| {
            ui.heading("My DashPay Profile");

            if !identities.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let response = ui.add(
                        IdentitySelector::new(
                            "profile_identity_selector",
                            &mut self.selected_identity_string,
                            &identities,
                        )
                        .selected_identity(&mut self.selected_identity)
                        .unwrap()
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
                        self.avatar_loading = false;
                        // Don't clear avatar_textures - they're keyed by URL so can be reused

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

                        // Load profile from database for the newly selected identity
                        self.load_profile_from_database();
                    }
                });
            }
        });

        ui.separator();

        if identities.is_empty() {
            return super::render_no_identities_card(ui, &self.app_context);
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

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view or edit profile");
            return action;
        }

        // Profile loading status - styled card when no profile loaded
        if !self.profile_load_attempted && !self.loading {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            Frame::group(ui.style())
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(5.0)
                .outer_margin(Margin::same(20))
                .shadow(ui.visuals().window_shadow)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(5.0);
                        ui.label(
                            RichText::new("No Profile Loaded")
                                .strong()
                                .size(25.0)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(5.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label("The profile for this identity hasn't been loaded yet.");
                        ui.add_space(10.0);
                        ui.label("Click the 'Refresh' button above to fetch it from the network.");
                        ui.add_space(10.0);
                    });
                });
            return action;
        }

        // Loading or saving indicator
        if self.loading || self.saving {
            ui.horizontal(|ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
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
                                    ui.label(
                                        RichText::new("Edit Profile")
                                            .strong()
                                            .color(DashColors::text_primary(dark_mode)),
                                    );

                                    ui.add_space(5.0);
                                    if crate::ui::helpers::info_icon_button(
                                        ui,
                                        PROFILE_GUIDELINES_INFO_TEXT,
                                    )
                                    .clicked()
                                    {
                                        self.show_info_popup = true;
                                    }
                                });

                                ui.separator();

                                // Display Name Field
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Display Name:")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    ui.label(RichText::new("*").color(egui::Color32::RED)); // Required indicator
                                });

                                let display_name_response = ui.add(
                                    TextEdit::singleline(&mut self.edit_display_name)
                                        .hint_text(egui::RichText::new("Enter your display name (required)").color(DashColors::text_secondary(dark_mode)))
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
                                ui.label(
                                    RichText::new(format!("{}/25", char_count))
                                        .small()
                                        .color(count_color),
                                );

                                if display_name_response.changed() {
                                    self.check_for_changes();
                                    self.validate_profile();
                                }

                                ui.add_space(10.0);

                                // Bio Field
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Bio/Status:")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                });

                                let bio_response = ui.add(
                                    TextEdit::multiline(&mut self.edit_bio)
                                        .hint_text(egui::RichText::new("Tell others about yourself (optional)").color(DashColors::text_secondary(dark_mode)))
                                        .desired_width(300.0)
                                        .desired_rows(4),
                                );

                                // Bio character count with color coding
                                let bio_count = self.edit_bio.len();
                                let bio_count_color = if bio_count > 140 {
                                    egui::Color32::RED
                                } else if bio_count > 120 {
                                    egui::Color32::ORANGE
                                } else {
                                    DashColors::text_secondary(dark_mode)
                                };
                                ui.label(
                                    RichText::new(format!("{}/140", bio_count))
                                        .small()
                                        .color(bio_count_color),
                                );

                                if bio_response.changed() {
                                    self.check_for_changes();
                                    self.validate_profile();
                                }

                                ui.add_space(10.0);

                                // Avatar URL Field
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Avatar URL:")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    if crate::ui::helpers::info_icon_button(
                                        ui,
                                        AVATAR_URL_INFO_TEXT,
                                    )
                                    .clicked()
                                    {
                                        self.show_avatar_info_popup = true;
                                    }
                                });

                                let avatar_response = ui.add(
                                    TextEdit::singleline(&mut self.edit_avatar_url)
                                        .hint_text(egui::RichText::new("https://example.com/avatar.jpg (optional)").color(DashColors::text_secondary(dark_mode)))
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
                                    ui.label(
                                        RichText::new(format!("{}/500", url_count))
                                            .small()
                                            .color(url_count_color),
                                    );
                                }

                                if avatar_response.changed() {
                                    self.check_for_changes();
                                    self.validate_profile();
                                }

                                // Show validation errors
                                if !self.validation_errors.is_empty() {
                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.label(
                                        RichText::new("Validation Errors:")
                                            .color(egui::Color32::RED)
                                            .strong(),
                                    );
                                    for error in &self.validation_errors {
                                        ui.label(
                                            RichText::new(format!("• {}", error.message()))
                                                .color(egui::Color32::RED)
                                                .small(),
                                        );
                                    }
                                }

                                ui.add_space(15.0);

                                // Check wallet lock status before showing save button
                                let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                                    if let Err(e) = try_open_wallet_no_password(wallet) {
                                        self.message = Some((e, MessageType::Error));
                                    }
                                    wallet_needs_unlock(wallet)
                                } else {
                                    false
                                };

                                if wallet_locked {
                                    ui.add_space(10.0);
                                    ui.colored_label(
                                        egui::Color32::from_rgb(200, 150, 50),
                                        "Wallet is locked. Please unlock to save profile.",
                                    );
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        if ui.button("Cancel").clicked() {
                                            self.cancel_editing();
                                        }
                                        ui.add_space(10.0);
                                        if ui.button("Unlock Wallet").clicked() {
                                            self.wallet_unlock_popup.open();
                                        }
                                    });
                                } else {
                                    // Fee estimation display
                                    let fee_estimator = self.app_context.fee_estimator();
                                    // Profile creation/update is a document operation
                                    let estimated_fee = if self.profile.is_some() {
                                        fee_estimator.estimate_document_replace()
                                    } else {
                                        fee_estimator.estimate_document_create()
                                    };

                                    Frame::group(ui.style())
                                        .fill(DashColors::surface(dark_mode))
                                        .inner_margin(Margin::symmetric(10, 8))
                                        .corner_radius(5.0)
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new("Estimated fee:")
                                                        .color(DashColors::text_secondary(dark_mode))
                                                        .size(14.0),
                                                );
                                                ui.label(
                                                    RichText::new(format_credits_as_dash(estimated_fee))
                                                        .color(DashColors::text_primary(dark_mode))
                                                        .size(14.0),
                                                );
                                            });
                                        });

                                    ui.add_space(10.0);

                                    // Check if identity has enough balance
                                    let has_enough_balance = self
                                        .selected_identity
                                        .as_ref()
                                        .map(|id| id.identity.balance() > estimated_fee)
                                        .unwrap_or(false);

                                    // Action buttons
                                    ui.horizontal(|ui| {
                                        if ui.button("Cancel").clicked() {
                                            if self.has_unsaved_changes {
                                                self.confirmation_dialog = Some(
                                                    ConfirmationDialog::new(
                                                        "Discard Changes?",
                                                        "You have unsaved profile changes. Are you sure you want to discard them?",
                                                    )
                                                    .confirm_text(Some("Discard"))
                                                    .cancel_text(Some("Keep Editing"))
                                                    .danger_mode(true),
                                                );
                                            } else {
                                                self.cancel_editing();
                                            }
                                        }

                                        ui.add_space(10.0);

                                        let can_save = self.is_valid() && has_enough_balance;
                                        let save_button = egui::Button::new(
                                            RichText::new("Save Profile")
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(if can_save {
                                            egui::Color32::from_rgb(0, 141, 228) // Dash blue
                                        } else {
                                            egui::Color32::GRAY
                                        });

                                        let hover_text = if !has_enough_balance {
                                            format!(
                                                "Insufficient identity balance for fee (need at least {})",
                                                format_credits_as_dash(estimated_fee)
                                            )
                                        } else if !self.is_valid() {
                                            "Please fix validation errors".to_string()
                                        } else {
                                            "Save profile changes".to_string()
                                        };

                                        if ui
                                            .add_enabled(can_save, save_button)
                                            .on_hover_text(&hover_text)
                                            .on_disabled_hover_text(&hover_text)
                                            .clicked()
                                        {
                                            action |= self.save_profile();
                                        }
                                    });
                                }
                            });
                        });
                    });
                } else {
                    // View mode
                    if let Some(profile) = self.profile.clone() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                // Avatar display
                                ui.vertical(|ui| {
                                    ui.add_space(5.0);
                                    ui.horizontal(|ui| {
                                        // Check if we have an avatar URL and try to display it
                                        if !profile.avatar_url.is_empty() {
                                            let texture_id =
                                                format!("avatar_{}", profile.avatar_url);

                                            // Check if texture is already cached in memory
                                            if let Some(texture) =
                                                self.avatar_textures.get(&texture_id)
                                            {
                                                // Display the cached avatar image (clickable)
                                                let image_response = ui.add(
                                                    egui::Image::new(texture)
                                                        .fit_to_exact_size(egui::vec2(80.0, 80.0))
                                                        .corner_radius(8.0)
                                                        .sense(egui::Sense::click()),
                                                ).on_hover_text("Click to view avatar URL");
                                                if image_response.clicked() {
                                                    self.show_avatar_url_popup = true;
                                                }
                                            } else {
                                                // Check if image data was loaded by async task from network
                                                let data_id =
                                                    format!("avatar_data_{}", profile.avatar_url);
                                                let bytes_id =
                                                    format!("avatar_bytes_{}", profile.avatar_url);
                                                let color_image = ui.ctx().data_mut(|data| {
                                                    data.get_temp::<ColorImage>(egui::Id::new(
                                                        &data_id,
                                                    ))
                                                });
                                                let fetched_bytes: Option<Vec<u8>> = ui.ctx().data_mut(|data| {
                                                    data.get_temp::<Vec<u8>>(egui::Id::new(
                                                        &bytes_id,
                                                    ))
                                                });

                                                if let Some(color_image) = color_image {
                                                    // Create texture from loaded image
                                                    let texture = ui.ctx().load_texture(
                                                        &texture_id,
                                                        color_image,
                                                        egui::TextureOptions::LINEAR,
                                                    );

                                                    // Display the image (clickable)
                                                    let image_response = ui.add(
                                                        egui::Image::new(&texture)
                                                            .fit_to_exact_size(egui::vec2(80.0, 80.0))
                                                            .corner_radius(8.0)
                                                            .sense(egui::Sense::click()),
                                                    ).on_hover_text("Click to view avatar URL");
                                                    if image_response.clicked() {
                                                        self.show_avatar_url_popup = true;
                                                    }

                                                    // Cache the texture in memory
                                                    self.avatar_textures
                                                        .insert(texture_id, texture);
                                                    self.avatar_loading = false;

                                                    // Save avatar bytes to database for caching
                                                    if let Some(bytes) = fetched_bytes
                                                        && let Some(ref identity) = self.selected_identity
                                                    {
                                                        let identity_id = identity.identity.id();
                                                        let network_str = self.app_context.network.to_string();
                                                        if let Err(e) = self.app_context.db.save_dashpay_profile_avatar_bytes(
                                                            &identity_id,
                                                            &network_str,
                                                            Some(&bytes),
                                                        ) {
                                                            tracing::error!("Failed to save avatar bytes to database: {}", e);
                                                        } else {
                                                            tracing::debug!("Saved avatar bytes to database ({} bytes)", bytes.len());
                                                        }
                                                        // Update the profile's avatar_bytes in memory
                                                        if let Some(ref mut p) = self.profile {
                                                            p.avatar_bytes = Some(bytes);
                                                        }
                                                    }

                                                    // Clear the temporary data
                                                    ui.ctx().data_mut(|data| {
                                                        data.remove::<ColorImage>(egui::Id::new(
                                                            &data_id,
                                                        ));
                                                        data.remove::<Vec<u8>>(egui::Id::new(
                                                            &bytes_id,
                                                        ));
                                                    });
                                                } else if !self.avatar_loading {
                                                    // Check if we have cached bytes from database
                                                    if let Some(ref avatar_bytes) = profile.avatar_bytes {
                                                        // Process cached bytes synchronously to avoid spinner
                                                        if let Some(color_image) = Self::process_avatar_bytes_sync(avatar_bytes) {
                                                            let texture = ui.ctx().load_texture(
                                                                &texture_id,
                                                                color_image,
                                                                egui::TextureOptions::LINEAR,
                                                            );
                                                            let image_response = ui.add(
                                                                egui::Image::new(&texture)
                                                                    .fit_to_exact_size(egui::vec2(80.0, 80.0))
                                                                    .corner_radius(8.0)
                                                                    .sense(egui::Sense::click()),
                                                            ).on_hover_text("Click to view avatar URL");
                                                            if image_response.clicked() {
                                                                self.show_avatar_url_popup = true;
                                                            }
                                                            self.avatar_textures.insert(texture_id, texture);
                                                        } else {
                                                            // Failed to process cached bytes, fetch from network
                                                            self.avatar_loading = true;
                                                            self.load_avatar_texture(
                                                                ui.ctx(),
                                                                &profile.avatar_url,
                                                            );
                                                            ui.add(
                                                                egui::Spinner::new()
                                                                    .color(DashColors::DASH_BLUE),
                                                            );
                                                        }
                                                    } else {
                                                        // No cached bytes, fetch from network
                                                        self.avatar_loading = true;
                                                        self.load_avatar_texture(
                                                            ui.ctx(),
                                                            &profile.avatar_url,
                                                        );
                                                        // Show spinner while loading
                                                        ui.add(
                                                            egui::Spinner::new()
                                                                .color(DashColors::DASH_BLUE),
                                                        );
                                                    }
                                                } else {
                                                    // Show loading indicator
                                                    ui.add(
                                                        egui::Spinner::new()
                                                            .color(DashColors::DASH_BLUE),
                                                    );
                                                }
                                            }
                                        } else {
                                            // No avatar URL, show default emoji
                                            ui.label(RichText::new("👤").size(80.0).color(DashColors::DEEP_BLUE));
                                        }
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
                                    if let Some(identity) = &self.selected_identity
                                        && !identity.dpns_names.is_empty()
                                    {
                                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                                        ui.label(
                                            RichText::new(format!(
                                                "@{}",
                                                identity.dpns_names[0].name
                                            ))
                                            .color(DashColors::text_secondary(dark_mode)),
                                        );
                                    }

                                    // Identity ID
                                    if let Some(identity) = &self.selected_identity {
                                        ui.label(
                                            RichText::new(format!(
                                                "ID: {}",
                                                identity.identity.id()
                                            ))
                                            .small()
                                            .weak(),
                                        );
                                    }
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::TOP),
                                    |ui| {
                                        let edit_button = egui::Button::new(
                                            RichText::new("Edit Profile")
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(egui::Color32::from_rgb(0, 141, 228)); // Dash blue

                                        if ui.add(edit_button).clicked() {
                                            self.start_editing();
                                        }
                                    },
                                );
                            });

                            ui.separator();

                            // Bio
                            let dark_mode = ui.ctx().style().visuals.dark_mode;
                            ui.label(
                                RichText::new("Bio:")
                                    .strong()
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            if !profile.bio.is_empty() {
                                ui.label(
                                    RichText::new(&profile.bio)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            } else {
                                ui.label(
                                    RichText::new("No bio set")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            }
                            ui.add_space(5.0);

                        });
                    } else if self.profile_load_attempted {
                        // No profile exists (only show after we've tried to load)
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
                                        RichText::new("No DashPay Profile")
                                            .strong()
                                            .size(20.0)
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    ui.add_space(5.0);
                                    ui.label(
                                        RichText::new(
                                            "This identity doesn't have a DashPay profile yet.",
                                        )
                                        .color(DashColors::text_secondary(dark_mode)),
                                    );
                                    ui.add_space(15.0);
                                    let create_button = egui::Button::new(
                                        RichText::new("Create Profile").color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(0, 141, 228)); // Dash blue

                                    if ui.add(create_button).clicked() {
                                        self.start_editing();
                                    }
                                    ui.add_space(10.0);
                                });
                            });
                    }
                }
            });
        }

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui.ctx(), |ui| {
                    let mut popup =
                        InfoPopup::new("Profile Guidelines", PROFILE_GUIDELINES_INFO_TEXT);
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        // Show avatar info popup if requested
        if self.show_avatar_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui.ctx(), |ui| {
                    let mut popup = InfoPopup::new("Avatar Image Guidelines", AVATAR_URL_INFO_TEXT);
                    if popup.show(ui).inner {
                        self.show_avatar_info_popup = false;
                    }
                });
        }

        // Show avatar URL popup when clicking on avatar image
        if self.show_avatar_url_popup {
            if let Some(profile) = &self.profile {
                let avatar_url = profile.avatar_url.clone();
                let texture_id = format!("avatar_{}", avatar_url);
                egui::Window::new("Avatar")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ui.ctx(), |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(5.0);

                            // Display larger avatar image
                            if let Some(texture) = self.avatar_textures.get(&texture_id) {
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(egui::vec2(200.0, 200.0))
                                        .corner_radius(10.0),
                                );
                            }

                            ui.add_space(10.0);

                            // Show URL in smaller, secondary text
                            let dark_mode = ui.ctx().style().visuals.dark_mode;
                            ui.label(
                                RichText::new(&avatar_url)
                                    .small()
                                    .color(DashColors::text_secondary(dark_mode)),
                            );

                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui.button("Copy URL").clicked() {
                                    ui.ctx().copy_text(avatar_url.clone());
                                    self.display_message(
                                        "Avatar URL copied to clipboard",
                                        MessageType::Info,
                                    );
                                    self.show_avatar_url_popup = false;
                                }
                                if ui.button("Close").clicked() {
                                    self.show_avatar_url_popup = false;
                                }
                            });
                        });
                    });
            } else {
                self.show_avatar_url_popup = false;
            }
        }

        // Show confirmation dialog for discarding unsaved changes
        if self.confirmation_dialog.is_some() {
            let dialog = self.confirmation_dialog.as_mut().unwrap();
            let response = dialog.show(ui);
            match response.inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) => {
                    self.confirmation_dialog = None;
                    self.cancel_editing();
                }
                Some(ConfirmationStatus::Canceled) => {
                    self.confirmation_dialog = None;
                }
                None => {}
            }
        }

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ui.ctx(), wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully, UI will update on next frame
            }
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
                    // Check if avatar URL changed - if so, we need to re-fetch the avatar
                    let old_avatar_url = self.profile.as_ref().map(|p| p.avatar_url.clone());
                    let avatar_url_changed = old_avatar_url.as_ref() != Some(&avatar_url);

                    // Preserve cached avatar bytes if URL hasn't changed
                    let avatar_bytes = if avatar_url_changed {
                        // URL changed, clear cached bytes and texture so new avatar is fetched
                        self.avatar_textures
                            .remove(&format!("avatar_{}", old_avatar_url.unwrap_or_default()));
                        self.avatar_loading = false;

                        // Clear old avatar bytes from database since URL changed
                        if let Some(ref identity) = self.selected_identity {
                            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                            let identity_id = identity.identity.id();
                            let network_str = self.app_context.network.to_string();
                            let _ = self.app_context.db.save_dashpay_profile_avatar_bytes(
                                &identity_id,
                                &network_str,
                                None,
                            );
                        }
                        None
                    } else {
                        // URL same, keep existing cached bytes
                        self.profile.as_ref().and_then(|p| p.avatar_bytes.clone())
                    };

                    self.profile = Some(DashPayProfile {
                        display_name: display_name.clone(),
                        bio: bio.clone(),
                        avatar_url: avatar_url.clone(),
                        avatar_bytes,
                    });

                    // Save profile to database for caching
                    if let Some(ref identity) = self.selected_identity {
                        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                        let identity_id = identity.identity.id();
                        let network_str = self.app_context.network.to_string();

                        if let Err(e) = self.app_context.db.save_dashpay_profile(
                            &identity_id,
                            &network_str,
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
                        let network_str = self.app_context.network.to_string();

                        // Save with all fields as None to indicate "no profile exists"
                        // This prevents unnecessary network queries on app restart
                        if let Err(e) = self.app_context.db.save_dashpay_profile(
                            &identity_id,
                            &network_str,
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
            BackendTaskSuccessResult::DashPayProfileUpdated(_identity_id) => {
                // Profile was successfully created/updated
                // Save the profile data to database BEFORE clearing edit fields
                if let Some(ref identity) = self.selected_identity {
                    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                    let identity_id = identity.identity.id();
                    let network_str = self.app_context.network.to_string();

                    let display_name = self.edit_display_name.trim();
                    let bio = self.edit_bio.trim();
                    let avatar_url = self.edit_avatar_url.trim();

                    tracing::info!(
                        "Saving profile to database: identity={}, network={}, display_name={:?}, bio={:?}, avatar_url={:?}",
                        identity_id,
                        network_str,
                        display_name,
                        bio,
                        avatar_url
                    );

                    // Save to database
                    match self.app_context.db.save_dashpay_profile(
                        &identity_id,
                        &network_str,
                        if display_name.is_empty() {
                            None
                        } else {
                            Some(display_name)
                        },
                        if bio.is_empty() { None } else { Some(bio) },
                        if avatar_url.is_empty() {
                            None
                        } else {
                            Some(avatar_url)
                        },
                        None,
                    ) {
                        Ok(_) => tracing::info!("Profile saved to database successfully"),
                        Err(e) => tracing::error!("Failed to save profile to database: {}", e),
                    }

                    // Update in-memory profile (preserve existing avatar_bytes if URL didn't change)
                    let existing_avatar_bytes = self.profile.as_ref().and_then(|p| {
                        if p.avatar_url == avatar_url {
                            p.avatar_bytes.clone()
                        } else {
                            None // URL changed, need to re-fetch
                        }
                    });
                    self.profile = Some(DashPayProfile {
                        display_name: display_name.to_string(),
                        bio: bio.to_string(),
                        avatar_url: avatar_url.to_string(),
                        avatar_bytes: existing_avatar_bytes,
                    });
                }

                self.cancel_editing(); // Exit edit mode (clears edit fields)
                self.show_success = true;
            }
            _ => {
                // Ignore other results - profile screen only handles DashPayProfile and DashPayProfileUpdated
            }
        }
    }
}
