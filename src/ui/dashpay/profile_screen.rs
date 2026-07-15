use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dashpay::{MAX_AVATAR_URL_CHARS, ProfileFieldError};
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::MessageType;
use crate::ui::components::avatar::Avatar;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{MessageBanner, ResultBannerExt};
use crate::ui::helpers::{ModalOpeningGuard, clicked_outside_window_after_open};
use crate::ui::identities::get_selected_wallet;
use crate::ui::state::AvatarCache;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{Frame, Margin, RichText, ScrollArea, TextEdit, Ui};
use std::sync::{Arc, RwLock};

const PROFILE_GUIDELINES_INFO_TEXT: &str = "Profile Guidelines:\n\n\
    Display names can include any UTF-8 characters (emojis, symbols, etc.).\n\n\
    Display names are limited to 25 characters.\n\n\
    Bios are limited to 140 characters.\n\n\
    Avatar URLs should point to publicly accessible images (max 2048 chars).\n\n\
    Profiles are public and visible to all DashPay users.";

const AVATAR_URL_INFO_TEXT: &str = "Avatar Image Guidelines:\n\n\
    The URL must point to a publicly accessible image.\n\n\
    Recommended: Square images (e.g., 256x256 or 512x512 pixels).\n\n\
    Supported formats: JPEG, PNG, WebP, or GIF.\n\n\
    Maximum URL length: 2048 characters.\n\n\
    Example URL:\nhttps://example.com/images/avatar.jpg\n\n\
    Tip: Use image hosting services like Imgur, Cloudinary, or your own server.";

#[derive(Debug, Clone)]
pub struct DashPayProfile {
    pub display_name: String,
    pub bio: String,
    pub avatar_url: String,
    pub avatar_bytes: Option<Vec<u8>>,
}

pub struct ProfileScreen {
    pub app_context: Arc<AppContext>,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    profile: Option<DashPayProfile>,
    editing: bool,
    edit_display_name: String,
    edit_bio: String,
    edit_avatar_url: String,
    loading: bool,
    saving: bool, // Track if we're saving vs loading
    profile_load_attempted: bool,
    validation_errors: Vec<ProfileFieldError>,
    has_unsaved_changes: bool,
    original_display_name: String,
    original_bio: String,
    original_avatar_url: String,
    avatar_cache: AvatarCache,
    pending_action: Option<Box<AppAction>>, // Action to execute on next frame
    show_info_popup: bool,
    show_avatar_info_popup: bool,
    show_avatar_url_popup: bool, // Show avatar URL when clicking on avatar in view mode
    avatar_url_popup_opening_guard: ModalOpeningGuard,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
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
            loading: false,
            saving: false,
            profile_load_attempted: false,
            validation_errors: Vec::new(),
            has_unsaved_changes: false,
            original_display_name: String::new(),
            original_bio: String::new(),
            original_avatar_url: String::new(),
            avatar_cache: AvatarCache::new(),
            pending_action: None,
            show_info_popup: false,
            show_avatar_info_popup: false,
            show_avatar_url_popup: false,
            avatar_url_popup_opening_guard: ModalOpeningGuard::default(),
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            show_success: false,
            was_creating_new: false,
            confirmation_dialog: None,
        };

        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        // Profile is loaded asynchronously by `LoadProfile` dispatch in `render()`.
        if let Ok(identities) = app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

            let selected_id = app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());

            new_self.selected_identity = Some(preferred.clone());
            new_self.selected_identity_string = preferred
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

            tracing::info!(
                "ProfileScreen::new - selected identity {}",
                new_self.selected_identity_string
            );

            new_self.selected_wallet = get_selected_wallet(&preferred, Some(&app_context), None)
                .or_show_error(app_context.egui_ctx())
                .unwrap_or(None);
        }

        new_self
    }

    fn validate_profile(&mut self) {
        self.validation_errors = crate::model::dashpay::validate_profile_fields(
            &self.edit_display_name,
            &self.edit_bio,
            &self.edit_avatar_url,
        );
    }

    fn check_for_changes(&mut self) {
        self.has_unsaved_changes = self.edit_display_name != self.original_display_name
            || self.edit_bio != self.original_bio
            || self.edit_avatar_url != self.original_avatar_url;
    }

    fn is_valid(&self) -> bool {
        self.validation_errors.is_empty()
    }

    /// Reset the load-attempted flag so the next `render()` re-dispatches
    /// `LoadProfile`. The local DET profile cache is gone after D3 — the
    /// async result populates `self.profile` via `display_task_result`.
    fn load_profile_from_database(&mut self) {
        self.profile_load_attempted = false;
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

        // Seed from the app-scoped selected identity if none yet selected (W3 SYNC).
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            let selected_id = self.app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());
            self.selected_identity = Some(preferred.clone());
            self.selected_identity_string = preferred
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
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
    }

    fn save_profile(&mut self) -> AppAction {
        self.validate_profile();

        if !self.is_valid() {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                self.validation_errors[0].message(),
                MessageType::Error,
            );
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
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "No identity selected",
                MessageType::Error,
            );
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

        // Auto-dispatch `LoadProfile` on first render or after identity change.
        if !self.profile_load_attempted
            && !self.loading
            && self.selected_identity.is_some()
            && matches!(action, AppAction::None)
        {
            action = self.trigger_load_profile();
        }

        // Show success screen if profile was just created/updated
        if self.show_success {
            return self.show_success_screen(ui);
        }

        // Identity selector or no identities message
        let identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();

        // Header with identity selector on the right
        ui.horizontal(|ui| {
            ui.heading("My DashPay Profile");

            if !identities.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // SYNC: write-back via syncing_global on user pick (FR-6: the source list is
                    // User-only, so a masternode/evonode can never leak to the app-global identity).
                    let response = ui.add(
                        IdentitySelector::new(
                            "profile_identity_selector",
                            &mut self.selected_identity_string,
                            &identities,
                        )
                        .selected_identity(&mut self.selected_identity)
                        .unwrap()
                        .width(300.0)
                        .other_option(false) // Disable "Other" option
                        .syncing_global(self.app_context.clone()),
                    );

                    if response.changed() {
                        // Reset state when identity changes
                        self.profile = None;
                        self.profile_load_attempted = false;
                        self.loading = false;
                        self.editing = false;
                        self.validation_errors.clear();
                        self.has_unsaved_changes = false;
                        // Avatar cache is keyed by URL, so it is reused across
                        // identities without a reset.

                        // Update wallet for the newly selected identity
                        if let Some(identity) = &self.selected_identity {
                            self.selected_wallet =
                                get_selected_wallet(identity, Some(&self.app_context), None)
                                    .or_show_error(self.app_context.egui_ctx())
                                    .unwrap_or(None);
                        } else {
                            self.selected_wallet = None;
                        }
                        self.wallet_open_attempted = false;

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

        if self.selected_identity.is_none() {
            ui.label("Please select an identity to view or edit profile");
            return action;
        }

        // Profile loading status - styled card when no profile loaded
        if !self.profile_load_attempted && !self.loading {
            let dark_mode = ui.style().visuals.dark_mode;
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
                let dark_mode = ui.style().visuals.dark_mode;
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
                                let dark_mode = ui.style().visuals.dark_mode;
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
                                let url_count = self.edit_avatar_url.chars().count();
                                let url_count_color = if url_count > MAX_AVATAR_URL_CHARS {
                                    egui::Color32::RED
                                } else if url_count > MAX_AVATAR_URL_CHARS - 50 {
                                    egui::Color32::ORANGE
                                } else {
                                    DashColors::text_secondary(dark_mode)
                                };
                                if !self.edit_avatar_url.is_empty() {
                                    ui.label(
                                        RichText::new(format!("{url_count}/{MAX_AVATAR_URL_CHARS}"))
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
                                    if !self.wallet_open_attempted {
                                        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                                            MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                                                .disable_auto_dismiss();
                                        }
                                        self.wallet_open_attempted = true;
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

                                        if ui.add_enabled(can_save, save_button)
                                            .clickable_tooltip(&hover_text)
                                            .disabled_tooltip(&hover_text)
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
                                        // Seed the cache with locally-known bytes
                                        // so a stored avatar renders without a
                                        // network round-trip.
                                        if let Some(bytes) = profile.avatar_bytes.clone() {
                                            self.avatar_cache.seed(&profile.avatar_url, bytes);
                                        }
                                        let response = Avatar::new(Some(profile.avatar_url.as_str()), 80.0)
                                            .corner_radius(8.0)
                                            .clickable("Click to view avatar URL")
                                            .show(ui, &mut self.avatar_cache);
                                        if let Some(task) = response.fetch {
                                            action |= AppAction::BackendTask(task);
                                        }
                                        if response.clicked {
                                            self.show_avatar_url_popup = true;
                                            self.avatar_url_popup_opening_guard.arm();
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
                                        let dark_mode = ui.style().visuals.dark_mode;
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
                            let dark_mode = ui.style().visuals.dark_mode;
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
                        let dark_mode = ui.style().visuals.dark_mode;
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
                .show(ui, |ui| {
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
                .show(ui, |ui| {
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

                // Draw modal overlay
                let screen_rect = ui.ctx().content_rect();
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Background,
                    egui::Id::new("avatar_popup_overlay"),
                ));
                painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

                let window_response = egui::Window::new("Avatar")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ui.ctx(), |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(5.0);

                            // Display larger avatar image
                            if let Some(texture) = self.avatar_cache.ready_texture(&avatar_url) {
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(egui::vec2(200.0, 200.0))
                                        .corner_radius(10.0),
                                );
                            }

                            ui.add_space(10.0);

                            // Show URL in smaller, secondary text
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.label(
                                RichText::new(&avatar_url)
                                    .small()
                                    .color(DashColors::text_secondary(dark_mode)),
                            );

                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ComponentStyles::add_primary_button(ui, "Copy URL").clicked() {
                                    ui.ctx().copy_text(avatar_url.clone());
                                    MessageBanner::set_global(
                                        ui.ctx(),
                                        "Avatar URL copied to clipboard",
                                        MessageType::Info,
                                    );
                                    self.show_avatar_url_popup = false;
                                }
                                if ComponentStyles::add_secondary_button(ui, "Close", dark_mode)
                                    .clicked()
                                {
                                    self.show_avatar_url_popup = false;
                                }
                            });
                        });
                    });

                if let Some(ref resp) = window_response
                    && clicked_outside_window_after_open(
                        ui.ctx(),
                        resp.response.rect,
                        &mut self.avatar_url_popup_opening_guard,
                    )
                {
                    self.show_avatar_url_popup = false;
                }
            } else {
                self.show_avatar_url_popup = false;
            }
        }

        // Show confirmation dialog for discarding unsaved changes
        if let Some(dialog) = self.confirmation_dialog.as_mut() {
            match dialog.show(ui).inner.dialog_response {
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

    pub fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.loading = false;
            self.saving = false;
        }
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // Avatar results arrive independently of profile load/save; route them
        // without disturbing those loading states.
        if let BackendTaskSuccessResult::DashPayAvatar { url, bytes } = result {
            self.avatar_cache.store(url, bytes);
            return;
        }

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
                        // URL changed: drop the stale cached avatar so the new URL re-fetches.
                        self.avatar_cache.invalidate();
                        None
                    } else {
                        // URL same, keep existing in-memory bytes
                        self.profile.as_ref().and_then(|p| p.avatar_bytes.clone())
                    };

                    self.profile = Some(DashPayProfile {
                        display_name: display_name.clone(),
                        bio: bio.clone(),
                        avatar_url: avatar_url.clone(),
                        avatar_bytes,
                    });

                    // Profile cache write dropped — `load_profile` /
                    // `update_profile` already mirror through the D3 seam
                    // (`WalletBackend::dashpay_set_profile`), so DashpayView
                    // is the single source of truth.
                    // Profile loaded successfully - no need to show a message
                } else {
                    // No profile found - clear any existing profile and show create button
                    self.profile = None;
                    // The "no profile" sentinel write dropped: the next fetch
                    // re-resolves via Platform, and `DashpayView::profile`
                    // already returns None when the upstream wallet has no
                    // DashPayProfile bound to the owner.
                    // Don't show a message - let the UI show "Create Profile" button
                }
            }
            BackendTaskSuccessResult::DashPayProfileUpdated(_identity_id) => {
                // Profile was successfully created/updated; the upstream
                // mirror (`update_profile` → `dashpay_set_profile`) is the
                // authoritative write, so we only refresh local in-memory
                // state for instant UI feedback.
                if let Some(ref identity) = self.selected_identity {
                    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                    let identity_id = identity.identity.id();

                    let display_name = self.edit_display_name.trim();
                    let bio = self.edit_bio.trim();
                    let avatar_url = self.edit_avatar_url.trim();

                    tracing::debug!(
                        identity = %identity_id,
                        "DashPay profile updated; refreshing in-memory copy"
                    );

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
