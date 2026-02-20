use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;

use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::wallet_unlock_popup::WalletUnlockResult;
use crate::ui::dashpay::contact_requests::ContactRequests;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{ColorImage, Frame, Margin, RichText, ScrollArea, TextureHandle, Ui};
use std::collections::{BTreeMap, HashSet};
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
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchFilter {
    All,
    WithUsernames,    // Only contacts with usernames
    WithoutUsernames, // Only contacts without usernames
    WithBio,          // Contacts with bio
    Recent,           // Added within the last 7 days
    Hidden,           // Only hidden contacts
    Visible,          // Only visible contacts
}

#[derive(Debug, Clone, PartialEq)]
pub enum SortOrder {
    Name,       // Sort by display name/username
    Username,   // Sort by username specifically
    DateAdded,  // Sort by date added (from database timestamp)
    AccountRef, // Sort by account reference number
}

/// Tab for the combined Contacts screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactsTab {
    Contacts,
    Requests,
}

pub struct ContactsList {
    pub app_context: Arc<AppContext>,
    contacts: BTreeMap<Identifier, Contact>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    search_query: String,
    message: Option<(String, MessageType)>,
    loading: bool,
    has_loaded: bool, // Track if we've ever loaded contacts
    show_hidden: bool,
    search_filter: SearchFilter,
    sort_order: SortOrder,
    avatar_textures: BTreeMap<String, TextureHandle>, // Cache for avatar textures by URL
    avatars_loading: HashSet<String>,                 // Track which avatars are being loaded
    /// Current active tab
    active_tab: ContactsTab,
    /// Embedded contact requests component
    pub contact_requests: ContactRequests,
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
            has_loaded: false,
            show_hidden: false,
            search_filter: SearchFilter::All,
            sort_order: SortOrder::Name,
            avatar_textures: BTreeMap::new(),
            avatars_loading: HashSet::new(),
            active_tab: ContactsTab::Contacts,
            contact_requests: ContactRequests::new(app_context.clone()),
        };

        // Auto-select first identity on creation if available
        if let Ok(identities) = app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            new_self.selected_identity = Some(identities[0].clone());
            new_self.selected_identity_string =
                identities[0].identity.id().to_string(Encoding::Base58);

            // Load contacts from database for this identity
            new_self.load_contacts_from_database();
        }

        new_self
    }

    fn load_contacts_from_database(&mut self) {
        // Load saved contacts for the selected identity from database
        if let Some(identity) = &self.selected_identity {
            let identity_id = identity.identity.id();
            let network_str = self.app_context.network.to_string();

            // Load saved contacts from database
            if let Ok(stored_contacts) = self
                .app_context
                .db
                .load_dashpay_contacts(&identity_id, &network_str)
            {
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
                            created_at: Some(stored_contact.created_at),
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
                        if let Ok(contact_id) = Identifier::from_bytes(&info.contact_identity_id)
                            && let Some(contact) = self.contacts.get_mut(&contact_id)
                        {
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

    pub fn trigger_fetch_requests(&mut self) -> AppAction {
        self.contact_requests.trigger_fetch_requests()
    }

    /// Set the active tab
    pub fn set_active_tab(&mut self, tab: ContactsTab) {
        self.active_tab = tab;
    }

    pub fn refresh(&mut self) -> AppAction {
        // Don't clear contacts - preserve loaded state
        // Only clear temporary states
        self.message = None;
        self.loading = false;

        // Auto-select first identity if none selected
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_qualified_identities()
            && !identities.is_empty()
        {
            self.selected_identity = Some(identities[0].clone());
            self.selected_identity_string = identities[0].identity.id().to_string(Encoding::Base58);
        }

        // Load contacts from database if we have an identity selected and no contacts loaded
        if self.selected_identity.is_some() && self.contacts.is_empty() {
            self.load_contacts_from_database();
        }

        // Also refresh contact requests
        let _ = self.contact_requests.refresh();

        AppAction::None
    }

    /// Load an avatar image from a URL asynchronously
    fn load_avatar_texture(&mut self, ctx: &egui::Context, url: &str) {
        // Mark as loading
        self.avatars_loading.insert(url.to_string());

        let ctx_clone = ctx.clone();
        let url_clone = url.to_string();

        // Spawn async task to fetch and load the image
        tokio::spawn(async move {
            match crate::backend_task::dashpay::avatar_processing::fetch_image_bytes(&url_clone)
                .await
            {
                Ok(image_bytes) => {
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
                            image::imageops::crop_imm(&rgba_image, x_offset, y_offset, size, size)
                                .to_image()
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
                        ctx_clone.request_repaint();

                        // Store the image data temporarily for the UI thread to pick up
                        ctx_clone.data_mut(|data| {
                            data.insert_temp(
                                egui::Id::new(format!("contact_avatar_data_{}", url_clone)),
                                color_image,
                            );
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch contact avatar image: {}", e);
                }
            }
        });
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Identity selector
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        // Header section with identity selector on the right
        ui.horizontal(|ui| {
            ui.heading("Contacts");

            if !identities.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        // Clear contacts and avatar caches when identity changes
                        self.contacts.clear();
                        self.avatar_textures.clear();
                        self.avatars_loading.clear();
                        self.message = None;
                        self.loading = false;

                        // Load contacts from database for the newly selected identity
                        self.load_contacts_from_database();

                        // Sync selected identity to contact_requests
                        self.contact_requests
                            .set_selected_identity(self.selected_identity.clone());
                    }
                });
            }
        });

        ui.separator();

        // Tab bar
        ui.horizontal(|ui| {
            let contacts_tab = egui::Button::new(RichText::new("My Contacts").color(
                if self.active_tab == ContactsTab::Contacts {
                    DashColors::WHITE
                } else {
                    DashColors::text_primary(dark_mode)
                },
            ))
            .fill(if self.active_tab == ContactsTab::Contacts {
                DashColors::DASH_BLUE
            } else {
                DashColors::glass_white(dark_mode)
            })
            .stroke(if self.active_tab == ContactsTab::Contacts {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0, DashColors::border(dark_mode))
            })
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::Vec2::new(120.0, 28.0));

            if ui.add(contacts_tab).clicked() {
                self.active_tab = ContactsTab::Contacts;
            }

            ui.add_space(8.0);

            // Get pending request count for badge
            let pending_count = self.contact_requests.pending_incoming_count();
            let requests_label = if pending_count > 0 {
                format!("Requests ({})", pending_count)
            } else {
                "Requests".to_string()
            };

            let requests_tab = egui::Button::new(RichText::new(requests_label).color(
                if self.active_tab == ContactsTab::Requests {
                    DashColors::WHITE
                } else {
                    DashColors::text_primary(dark_mode)
                },
            ))
            .fill(if self.active_tab == ContactsTab::Requests {
                DashColors::DASH_BLUE
            } else {
                DashColors::glass_white(dark_mode)
            })
            .stroke(if self.active_tab == ContactsTab::Requests {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0, DashColors::border(dark_mode))
            })
            .corner_radius(egui::CornerRadius::same(4))
            .min_size(egui::Vec2::new(120.0, 28.0));

            if ui.add(requests_tab).clicked() {
                self.active_tab = ContactsTab::Requests;
            }
        });

        ui.add_space(8.0);

        if identities.is_empty() {
            return super::render_no_identities_card(ui, &self.app_context);
        } else if self.active_tab == ContactsTab::Requests {
            // Sync identity before rendering (in case it wasn't synced yet)
            self.contact_requests
                .set_selected_identity(self.selected_identity.clone());
            // Render the contact requests tab without its own header
            action |= self.contact_requests.render_embedded(ui);

            // Show wallet unlock popup if open (needed because we're embedding contact_requests)
            if self.contact_requests.wallet_unlock_popup.is_open()
                && let Some(wallet) = &self.contact_requests.selected_wallet
            {
                let result = self.contact_requests.wallet_unlock_popup.show(
                    ui.ctx(),
                    wallet,
                    &self.app_context,
                );
                if result == WalletUnlockResult::Unlocked {
                    // Wallet unlocked successfully, UI will update on next frame
                }
            }

            return action;
        }

        // Contacts tab - show search/filter/sort controls if there are contacts
        {
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
                                    SearchFilter::Recent,
                                    "Recent",
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
                                    SortOrder::DateAdded,
                                    "Date added",
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
                MessageType::Warning => DashColors::WARNING,
                MessageType::Info => egui::Color32::LIGHT_BLUE,
            };
            ui.colored_label(color, message);
            ui.separator();
        }

        // Loading indicator
        if self.loading {
            ui.horizontal(|ui| {
                ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
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
                        let seven_days_ago = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64
                            - 7 * 24 * 60 * 60;
                        match contact.created_at {
                            Some(ts) if ts >= seven_days_ago => {}
                            _ => return false,
                        }
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
                if let Some(username) = &contact.username
                    && search_in_text(username)
                {
                    return true;
                }

                // Search in display name
                if let Some(display_name) = &contact.display_name
                    && search_in_text(display_name)
                {
                    return true;
                }

                // Search in nickname
                if let Some(nickname) = &contact.nickname
                    && search_in_text(nickname)
                {
                    return true;
                }

                // Search in bio
                if let Some(bio) = &contact.bio
                    && search_in_text(bio)
                {
                    return true;
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
                    // Sort by created_at descending (newest first)
                    // Contacts without timestamps sort last
                    b.created_at.unwrap_or(0).cmp(&a.created_at.unwrap_or(0))
                }
            }
        });

        // Contacts list
        ScrollArea::vertical()
            .id_salt("contacts_list_scroll")
            .show(ui, |ui| {
                if self.contacts.is_empty() {
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
                                    RichText::new("No Contacts")
                                        .strong()
                                        .size(20.0)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                ui.add_space(5.0);
                                ui.label(
                                    RichText::new("You haven't added any contacts yet.")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                ui.add_space(15.0);
                                let add_button = egui::Button::new(
                                    RichText::new("Add Contact").color(egui::Color32::WHITE),
                                )
                                .fill(egui::Color32::from_rgb(0, 141, 228));
                                if ui.add(add_button).clicked() {
                                    action = AppAction::AddScreen(
                                        ScreenType::DashPayAddContact
                                            .create_screen(&self.app_context),
                                    );
                                }
                                ui.add_space(10.0);
                            });
                        });
                } else if filtered_contacts.is_empty() {
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
                                    RichText::new("No Matches")
                                        .strong()
                                        .size(20.0)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                ui.add_space(5.0);
                                ui.label(
                                    RichText::new("No contacts match your search.")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                ui.add_space(10.0);
                            });
                        });
                } else {
                    // Collect avatar URLs that need to be loaded
                    let mut avatars_to_load: Vec<String> = Vec::new();

                    for contact in filtered_contacts {
                        let avatar_url_clone = contact.avatar_url.clone();
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                // Avatar display
                                ui.vertical(|ui| {
                                    ui.add_space(5.0);
                                    const AVATAR_SIZE: f32 = 40.0;

                                    if let Some(ref url) = avatar_url_clone {
                                        if !url.is_empty() {
                                            let texture_id = format!("contact_avatar_{}", url);

                                            // Check if texture is already cached
                                            if let Some(texture) =
                                                self.avatar_textures.get(&texture_id)
                                            {
                                                // Display the cached avatar image
                                                ui.add(
                                                    egui::Image::new(texture)
                                                        .fit_to_exact_size(egui::vec2(
                                                            AVATAR_SIZE,
                                                            AVATAR_SIZE,
                                                        ))
                                                        .corner_radius(AVATAR_SIZE / 2.0),
                                                );
                                            } else {
                                                // Check if image data was loaded by async task
                                                let data_id =
                                                    format!("contact_avatar_data_{}", url);
                                                let color_image = ui.ctx().data_mut(|data| {
                                                    data.get_temp::<ColorImage>(egui::Id::new(
                                                        &data_id,
                                                    ))
                                                });

                                                if let Some(color_image) = color_image {
                                                    // Create texture from loaded image
                                                    let texture = ui.ctx().load_texture(
                                                        &texture_id,
                                                        color_image,
                                                        egui::TextureOptions::LINEAR,
                                                    );

                                                    // Display the image
                                                    ui.add(
                                                        egui::Image::new(&texture)
                                                            .fit_to_exact_size(egui::vec2(
                                                                AVATAR_SIZE,
                                                                AVATAR_SIZE,
                                                            ))
                                                            .corner_radius(AVATAR_SIZE / 2.0),
                                                    );

                                                    // Cache the texture and clear loading state
                                                    self.avatar_textures
                                                        .insert(texture_id.clone(), texture);
                                                    self.avatars_loading.remove(url);

                                                    // Clear the temporary data
                                                    ui.ctx().data_mut(|data| {
                                                        data.remove::<ColorImage>(egui::Id::new(
                                                            &data_id,
                                                        ));
                                                    });
                                                } else if !self.avatars_loading.contains(url) {
                                                    // Queue for loading
                                                    avatars_to_load.push(url.clone());
                                                    // Show spinner while loading
                                                    ui.add(
                                                        egui::Spinner::new()
                                                            .size(AVATAR_SIZE)
                                                            .color(DashColors::DASH_BLUE),
                                                    );
                                                } else {
                                                    // Show loading indicator
                                                    ui.add(
                                                        egui::Spinner::new()
                                                            .size(AVATAR_SIZE)
                                                            .color(DashColors::DASH_BLUE),
                                                    );
                                                }
                                            }
                                        } else {
                                            // Empty URL, show default emoji
                                            ui.label(
                                                RichText::new("👤")
                                                    .size(AVATAR_SIZE)
                                                    .color(DashColors::DEEP_BLUE),
                                            );
                                        }
                                    } else {
                                        // No avatar URL, show default emoji
                                        ui.label(
                                            RichText::new("👤")
                                                .size(AVATAR_SIZE)
                                                .color(DashColors::DEEP_BLUE),
                                        );
                                    }
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
                                    if let Some(username) = &contact.username
                                        && (contact.display_name.is_some()
                                            || contact.nickname.is_some())
                                    {
                                        ui.label(
                                            RichText::new(format!("@{}", username))
                                                .small()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
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
                                        // Hide/Unhide button
                                        let hide_button_text =
                                            if contact.is_hidden { "Unhide" } else { "Hide" };
                                        if ui.button(hide_button_text).clicked() {
                                            let new_hidden = !contact.is_hidden;
                                            if let Some(identity) = &self.selected_identity {
                                                let owner_id = identity.identity.id();
                                                if let Err(e) =
                                                    self.app_context.db.set_contact_hidden(
                                                        &owner_id,
                                                        &contact.identity_id,
                                                        new_hidden,
                                                    )
                                                {
                                                    self.message = Some((
                                                        format!("Failed to update contact: {}", e),
                                                        MessageType::Error,
                                                    ));
                                                } else {
                                                    // Update the contact in memory
                                                    if let Some(c) =
                                                        self.contacts.get_mut(&contact.identity_id)
                                                    {
                                                        c.is_hidden = new_hidden;
                                                    }
                                                }
                                            }
                                        }

                                        // Pay button - requires SPV which is dev mode only
                                        if self.app_context.is_developer_mode()
                                            && ui.button("Pay").clicked()
                                        {
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

                    // Load any avatars that were queued
                    for url in avatars_to_load {
                        self.load_avatar_texture(ui.ctx(), &url);
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
                        created_at: None,
                    };
                    self.contacts.insert(contact_id, contact);
                }

                // Mark as loaded and clear message
                self.has_loaded = true;
                self.message = None;
            }
            BackendTaskSuccessResult::DashPayContactsWithInfo(contacts_data) => {
                // Clear existing contacts
                self.contacts.clear();

                // Save contacts to database if we have a selected identity
                if let Some(identity) = &self.selected_identity {
                    let owner_id = identity.identity.id();
                    let network_str = self.app_context.network.to_string();

                    // Clear all existing contacts for this identity from database first
                    // This prevents stale contacts from persisting
                    if let Err(e) = self
                        .app_context
                        .db
                        .clear_dashpay_contacts(&owner_id, &network_str)
                    {
                        tracing::warn!("Failed to clear dashpay contacts from database: {}", e);
                    }

                    // Convert ContactData to Contact structs and save to database
                    for contact_data in contacts_data {
                        // Skip self-contacts (where contact is the same as the owner)
                        if contact_data.identity_id == owner_id {
                            continue;
                        }
                        let contact = Contact {
                            identity_id: contact_data.identity_id,
                            username: contact_data.username.clone(),
                            display_name: contact_data.display_name.clone().or_else(|| {
                                Some(format!(
                                    "Contact ({})",
                                    &contact_data.identity_id.to_string(Encoding::Base58)[0..8]
                                ))
                            }),
                            avatar_url: contact_data.avatar_url.clone(),
                            bio: contact_data.bio.clone(),
                            nickname: contact_data.nickname.clone(),
                            is_hidden: contact_data.is_hidden,
                            account_reference: contact_data.account_reference,
                            created_at: Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64,
                            ), // Fallback to current time for filter/sort
                        };
                        self.contacts.insert(contact_data.identity_id, contact);

                        // Save to database
                        if let Err(e) = self.app_context.db.save_dashpay_contact(
                            &owner_id,
                            &contact_data.identity_id,
                            &network_str,
                            contact_data.username.as_deref(),
                            contact_data.display_name.as_deref(),
                            contact_data.avatar_url.as_deref(),
                            None,       // public_message - not yet fetched
                            "accepted", // Only accepted contacts are returned from load_contacts
                        ) {
                            tracing::warn!("Failed to save dashpay contact to database: {}", e);
                        }

                        // Save private info if present
                        if let Some(nickname) = &contact_data.nickname
                            && let Err(e) = self.app_context.db.save_contact_private_info(
                                &owner_id,
                                &contact_data.identity_id,
                                nickname,
                                &contact_data.note.unwrap_or_default(),
                                contact_data.is_hidden,
                            )
                        {
                            tracing::warn!(
                                "Failed to save contact private info to database: {}",
                                e
                            );
                        }
                    }
                } else {
                    // No selected identity, just populate in-memory
                    for contact_data in contacts_data {
                        let contact = Contact {
                            identity_id: contact_data.identity_id,
                            username: contact_data.username,
                            display_name: contact_data.display_name.or_else(|| {
                                Some(format!(
                                    "Contact ({})",
                                    &contact_data.identity_id.to_string(Encoding::Base58)[0..8]
                                ))
                            }),
                            avatar_url: contact_data.avatar_url,
                            bio: contact_data.bio,
                            nickname: contact_data.nickname,
                            is_hidden: contact_data.is_hidden,
                            account_reference: contact_data.account_reference,
                            created_at: Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64,
                            ), // Fallback to current time for filter/sort
                        };
                        self.contacts.insert(contact_data.identity_id, contact);
                    }
                }

                // Mark as loaded and clear message
                self.has_loaded = true;
                self.message = None;
            }
            BackendTaskSuccessResult::DashPayContactProfile(Some(doc)) => {
                // Extract profile information from the document
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
                        let network_str = self.app_context.network.to_string();
                        if let Err(e) = self.app_context.db.save_dashpay_contact(
                            &owner_id,
                            &contact_id,
                            &network_str,
                            contact.username.as_deref(),
                            contact.display_name.as_deref(),
                            contact.avatar_url.as_deref(),
                            public_message.as_deref(),
                            "accepted",
                        ) {
                            tracing::warn!(
                                "Failed to save updated contact profile to database: {}",
                                e
                            );
                        }
                    }
                }
            }
            _ => {
                // Ignore other results
            }
        }
    }
}
