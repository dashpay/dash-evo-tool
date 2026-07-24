use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;

use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::avatar::Avatar;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::wallet_unlock_popup::WalletUnlockResult;
use crate::ui::dashpay::contact_requests::ContactRequests;
use crate::ui::dashpay::persist_contact_private_info;
use crate::ui::state::AvatarCache;
use crate::ui::state::contacts_view::{ContactSearchFields, matches_contact_search};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{Frame, Margin, RichText, ScrollArea, Ui};
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
    pub created_at: Option<i64>,
}

impl<'a> From<&'a Contact> for ContactSearchFields<'a> {
    fn from(contact: &'a Contact) -> Self {
        Self {
            nickname: contact.nickname.as_deref(),
            display_name: contact.display_name.as_deref(),
            username: contact.username.as_deref(),
            bio: contact.bio.as_deref(),
            identity_id: contact.identity_id,
        }
    }
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
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    search_query: String,
    message: Option<(String, MessageType)>,
    loading: bool,
    has_loaded: bool, // Track if we've ever loaded contacts
    show_hidden: bool,
    search_filter: SearchFilter,
    sort_order: SortOrder,
    avatar_cache: AvatarCache,
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
            avatar_cache: AvatarCache::new(),
            active_tab: ContactsTab::Contacts,
            contact_requests: ContactRequests::new(app_context.clone()),
        };

        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        if let Ok(identities) = app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            let selected_id = app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());
            new_self.selected_identity = Some(preferred.clone());
            new_self.selected_identity_string = preferred.identity.id().to_string(Encoding::Base58);
        }

        new_self
    }

    /// Auto-fetch on view: reads contacts from offline rehydrated state plus
    /// the DET contact-profile cache, so the list paints without a network
    /// round-trip. An explicit refresh uses
    /// [`Self::trigger_refresh_contacts`] to re-fetch from the network.
    pub fn trigger_fetch_contacts(&mut self) -> AppAction {
        // Only fetch if we have a selected identity
        if let Some(identity) = &self.selected_identity {
            self.loading = true;
            self.message = None; // Clear any existing message
            // Mark the attempt at dispatch time, not on success. The error
            // handlers reset `loading` but leave this flag set, so a failed
            // load fires the auto-fetch gate exactly once instead of
            // re-dispatching every frame. `refresh()` / an identity change
            // opts back into a fresh attempt.
            self.has_loaded = true;

            let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContactsOffline {
                identity: identity.clone(),
            }));

            return AppAction::BackendTask(task);
        }

        AppAction::None
    }

    /// Re-fetch contacts and their profiles from the network, refreshing the
    /// DET caches. Triggered by an explicit user refresh, not by entering the
    /// view — the view itself serves the offline cache.
    ///
    /// Clears the in-memory rendered-texture cache so a contact whose avatar
    /// changed at the same URL repaints in-session: the textures are keyed by
    /// URL and never expire on their own, so without this an already-rendered
    /// avatar would keep showing the stale image even after the network read
    /// re-fetches and re-caches the new bytes.
    pub fn trigger_refresh_contacts(&mut self) -> AppAction {
        let Some(identity) = self.selected_identity.clone() else {
            return AppAction::None;
        };

        self.loading = true;
        self.message = None;
        self.has_loaded = true;
        self.clear_avatar_render_state();

        // An explicit refresh means "give me the latest", so drop the DET
        // avatar disk cache too: otherwise a within-TTL hit would short-circuit
        // the re-fetch and keep serving the old bytes for an unchanged URL.
        // Best-effort — a clear miss only costs one re-fetch.
        if let Ok(backend) = self.app_context.wallet_backend()
            && let Err(e) = backend.avatar_cache().clear()
        {
            tracing::debug!(error = ?e, "Failed to clear avatar cache on refresh");
        }

        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::LoadContacts { identity },
        )))
    }

    /// Drop the in-memory avatar render state so the next render re-derives each
    /// avatar from the (re-fetched) cache. Shared by the explicit Refresh path
    /// and the identity-change reset.
    fn clear_avatar_render_state(&mut self) {
        self.avatar_cache.invalidate();
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

        // Seed from the app-scoped selected identity if none yet selected (W3 SYNC).
        if self.selected_identity.is_none()
            && let Ok(identities) = self.app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            let selected_id = self.app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());
            self.selected_identity = Some(preferred.clone());
            self.selected_identity_string = preferred.identity.id().to_string(Encoding::Base58);
        }

        // Trigger backend fetch if we have an identity selected and no contacts loaded.
        // The result populates `self.contacts` via `display_task_result`.
        let action = if self.selected_identity.is_some() && self.contacts.is_empty() {
            self.trigger_fetch_contacts()
        } else {
            AppAction::None
        };

        // Also refresh contact requests
        let _ = self.contact_requests.refresh();

        action
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

        // Auto-fetch contacts on first render or after identity change.
        if !self.has_loaded && !self.loading && self.selected_identity.is_some() {
            action = self.trigger_fetch_contacts();
        }

        // Identity selector
        let identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();

        // Header section with identity selector on the right
        let mut refresh_action = AppAction::None;
        ui.horizontal(|ui| {
            ui.heading("Contacts");

            if !identities.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // SYNC: write-back via syncing_global on user pick (FR-6: the source list is
                    // User-only, so a masternode/evonode can never leak to the app-global identity).
                    let response = ui.add(
                        IdentitySelector::new(
                            "contacts_identity_selector",
                            &mut self.selected_identity_string,
                            &identities,
                        )
                        .selected_identity(&mut self.selected_identity)
                        .unwrap()
                        .width(300.0)
                        .other_option(false)
                        .syncing_global(self.app_context.clone()),
                    );

                    if response.changed() {
                        // Clear contacts and avatar caches when identity changes.
                        // The next render dispatches the offline read via
                        // `has_loaded == false`.
                        self.contacts.clear();
                        self.clear_avatar_render_state();
                        self.message = None;
                        self.loading = false;
                        self.has_loaded = false;

                        // Sync selected identity to contact_requests
                        self.contact_requests
                            .set_selected_identity(self.selected_identity.clone());
                    }

                    // Explicit refresh: re-fetch contacts and profiles from the
                    // network (the view itself serves the offline cache).
                    if ui
                        .add_enabled(!self.loading, egui::Button::new("Refresh"))
                        .on_hover_text("Fetch the latest contacts and profiles from the network.")
                        .clicked()
                    {
                        refresh_action = self.trigger_refresh_contacts();
                    }
                });
            }
        });

        if !matches!(refresh_action, AppAction::None) {
            action = refresh_action;
        }

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
        let query = self.search_query.clone();

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

                matches_contact_search(ContactSearchFields::from(*contact), &query)
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
                    for contact in filtered_contacts {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                // Avatar display
                                ui.vertical(|ui| {
                                    ui.add_space(5.0);
                                    action |= Avatar::new(contact.avatar_url.as_deref(), 40.0)
                                        .show(ui, &mut self.avatar_cache)
                                        .into_action();
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

                                    let dark_mode = ui.style().visuals.dark_mode;

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
                                                // Preserve the existing memo, flipping only the
                                                // hidden flag.
                                                let existing = self
                                                    .app_context
                                                    .wallet_backend()
                                                    .ok()
                                                    .and_then(|b| {
                                                        b.dashpay_get_private_info(
                                                            &owner_id,
                                                            &contact.identity_id,
                                                        )
                                                        .ok()
                                                        .flatten()
                                                    })
                                                    .unwrap_or_default();
                                                let sidecar_result = persist_contact_private_info(
                                                    &self.app_context,
                                                    &owner_id,
                                                    &contact.identity_id,
                                                    existing.nickname,
                                                    existing.notes,
                                                    new_hidden,
                                                );
                                                if let Err(error) = sidecar_result {
                                                    tracing::warn!(
                                                        ?error,
                                                        "Failed to update private contact information"
                                                    );
                                                    self.message = Some((
                                                        "The contact could not be updated. Check available disk space and try again."
                                                            .to_string(),
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

                                        // Pay is an experimental DashPay feature.
                                        if FeatureGate::DashPayOperations
                                            .is_available(&self.app_context)
                                            && ui.button("Pay").clicked()
                                        {
                                            if let Some(identity) = self.selected_identity.clone() {
                                                action = AppAction::AddScreen(
                                                    ScreenType::DashPaySendPayment(
                                                        identity,
                                                        contact.identity_id,
                                                    )
                                                    .create_screen(&self.app_context),
                                                );
                                            } else {
                                                tracing::warn!(
                                                    "Pay clicked with no selected identity; ignoring"
                                                );
                                            }
                                        }

                                        if ui.button("View Profile").clicked() {
                                            if let Some(identity) = self.selected_identity.clone() {
                                                action = AppAction::AddScreen(
                                                    ScreenType::DashPayContactProfileViewer(
                                                        identity,
                                                        contact.identity_id,
                                                    )
                                                    .create_screen(&self.app_context),
                                                );
                                            } else {
                                                tracing::warn!(
                                                    "View Profile clicked with no selected identity; ignoring"
                                                );
                                            }
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
        // Trigger a fresh `LoadContacts` dispatch via the auto-fetch in `render()`.
        if self.selected_identity.is_some() && self.contacts.is_empty() {
            self.has_loaded = false;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ui, |ui| {
            action = self.render(ui);
        });
        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.loading = false;
        self.message = Some((message.to_string(), message_type));
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // Avatar results arrive independently of the contact-list load; route
        // them without disturbing the list's loading state.
        if let BackendTaskSuccessResult::DashPayAvatar { url, bytes } = result {
            self.avatar_cache.store(url, bytes);
            return;
        }

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
            BackendTaskSuccessResult::DashPayContactsWithInfo {
                identity,
                contacts: contacts_data,
            } => {
                let owner_id_opt = self.selected_identity.as_ref().map(|i| i.identity.id());

                // A load that outlived an identity switch belongs to the
                // identity we left — it must not repopulate this list.
                if owner_id_opt != Some(identity) {
                    tracing::debug!("Discarding contacts for a no-longer-selected identity");
                    return;
                }

                // Clear existing contacts and repopulate the in-memory map
                // from the adapter result. Upstream `ManagedIdentity` is
                // now the authoritative source for contact rows (D4d), so
                // the DET-local cache writes are gone.
                self.contacts.clear();
                for contact_data in contacts_data {
                    // Skip self-contacts (where contact is the same as the owner)
                    if owner_id_opt
                        .as_ref()
                        .is_some_and(|owner| *owner == contact_data.identity_id)
                    {
                        continue;
                    }
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
                    // `DashpayView::contacts` reads contact identities from the
                    // upstream wallet and cross-references the public
                    // DashPayProfile via the backend task on demand, so this
                    // message is not cached locally.
                    let _ = public_message;
                }
            }
            _ => {
                // Ignore other results
            }
        }
    }
}
