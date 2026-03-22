use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};

use dash_sdk::platform::{Document, Identifier};
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

const PROFILE_SEARCH_INFO_TEXT: &str = "About Profile Search:\n\n\
    Search for users by their DPNS username.\n\n\
    Usernames are unique, verified identifiers on Dash Platform.\n\n\
    Results show the username along with profile info (if available).\n\n\
    Add contacts directly from search results.";

#[derive(Debug, Clone)]
pub struct ProfileSearchResult {
    pub identity_id: Identifier,
    pub display_name: Option<String>,
    pub public_message: Option<String>,
    pub avatar_url: Option<String>,
    pub username: Option<String>, // From DPNS if available
}

pub struct ProfileSearchScreen {
    pub app_context: Arc<AppContext>,
    search_query: String,
    search_results: Vec<ProfileSearchResult>,
    loading: bool,
    has_searched: bool, // Track if a search has been performed
    show_info_popup: bool,
}

impl ProfileSearchScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            search_query: String::new(),
            search_results: Vec::new(),
            loading: false,
            has_searched: false,
            show_info_popup: false,
        }
    }

    fn search_profiles(&mut self, ctx: &egui::Context) -> AppAction {
        if self.search_query.trim().is_empty() {
            self.loading = false;
            crate::ui::components::MessageBanner::set_global(
                ctx,
                "Please enter a search term",
                MessageType::Error,
            );
            return AppAction::None;
        }

        self.loading = true;
        self.search_results.clear();
        self.has_searched = true; // Mark that a search has been performed

        let task = BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles {
            search_query: self.search_query.trim().to_string(),
        }));

        AppAction::BackendTask(task)
    }

    fn view_profile(&mut self, ctx: &egui::Context, identity_id: Identifier) -> AppAction {
        // Use any available identity for viewing (just needed for context)
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();
        if identities.is_empty() {
            crate::ui::components::MessageBanner::set_global(
                ctx,
                "No identities available. Please load an identity first.",
                MessageType::Error,
            );
            return AppAction::None;
        }

        AppAction::AddScreen(
            ScreenType::DashPayContactProfileViewer(identities[0].clone(), identity_id)
                .create_screen(&self.app_context),
        )
    }

    fn add_contact(&mut self, identity_id: Identifier) -> AppAction {
        // Convert the identity ID to a base58 string and navigate to the Add Contact screen
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let identity_id_string = identity_id.to_string(Encoding::Base58);

        // Navigate to the Add Contact screen with the pre-populated identity ID
        AppAction::AddScreen(
            ScreenType::DashPayAddContactWithId(identity_id_string)
                .create_screen(&self.app_context),
        )
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Header
        ui.horizontal(|ui| {
            ui.heading("Search Public Profiles");
            ui.add_space(5.0);
            if crate::ui::helpers::info_icon_button(ui, PROFILE_SEARCH_INFO_TEXT).clicked() {
                self.show_info_popup = true;
            }
        });

        ui.separator();

        ScrollArea::vertical().show(ui, |ui| {
            // Search section
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    let response = ui.add(
                        TextEdit::singleline(&mut self.search_query)
                            .hint_text("Enter DPNS username...")
                            .desired_width(400.0),
                    );

                    // Trigger search on Enter key
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = self.search_profiles(ui.ctx());
                    }
                });

                if ui.button("Search").clicked() {
                    action = self.search_profiles(ui.ctx());
                }
            });

            ui.label(
                RichText::new("Tip: Search by DPNS username prefix (e.g., \"john\" finds \"john.dash\", \"johnny.dash\", etc.)")
                    .small()
                    .color(DashColors::text_secondary(dark_mode)),
            );

            ui.add_space(10.0);

            // Loading indicator
            if self.loading {
                ui.horizontal(|ui| {
                    ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                    ui.label("Searching...");
                });
                return;
            }

            // Search results
            if !self.search_results.is_empty() {
                ui.group(|ui| {
                    ui.label(
                        RichText::new(format!("Search Results ({})", self.search_results.len()))
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.separator();

                    let search_results = self.search_results.clone();
                    for result in &search_results {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                // No avatar display in search results
                                ui.vertical(|ui| {
                                    // Username (primary identifier per DIP-15)
                                    if let Some(username) = &result.username {
                                        ui.label(
                                            RichText::new(username)
                                                .strong()
                                                .size(16.0)
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }

                                    // Display name (complementary info)
                                    if let Some(display_name) = &result.display_name {
                                        ui.label(
                                            RichText::new(display_name)
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
                                    }

                                    // Public message preview
                                    if let Some(public_message) = &result.public_message {
                                        let preview = if public_message.len() > 60 {
                                            format!("{}...", &public_message[..60])
                                        } else {
                                            public_message.clone()
                                        };
                                        ui.label(
                                            RichText::new(preview)
                                                .small()
                                                .italics()
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
                                    }

                                    // Identity ID
                                    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
                                    ui.label(
                                        RichText::new(format!(
                                            "ID: {}",
                                            result.identity_id.to_string(Encoding::Base58)
                                        ))
                                        .small()
                                        .color(DashColors::text_secondary(dark_mode)),
                                    );
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("View Profile").clicked() {
                                            action = self.view_profile(ui.ctx(), result.identity_id);
                                        }
                                        if ui.button("Add Contact").clicked() {
                                            action = self.add_contact(result.identity_id);
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
            } else if self.has_searched && !self.loading {
                // Only show "No users found" if we've actually performed a search
                ui.group(|ui| {
                    ui.label("No users found");
                    ui.separator();
                    ui.label("Try searching with a different username prefix.");
                });
            }
        });

        action
    }

    pub fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Don't touch loading state here — display_message can be called for
        // unrelated task completions while we're waiting for search results.
        // Loading state is managed by search_profiles() and display_task_result().
    }
}

impl ScreenLike for ProfileSearchScreen {
    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel - consistent with other DashPay subscreens
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Profile Search", AppAction::None),
            ],
            vec![(
                "Clear Results",
                DesiredAppAction::Custom("clear_search".to_string()),
            )],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ctx, &self.app_context, RootScreenType::RootScreenDashpay);

        // Add DashPay subscreen chooser panel
        action |= add_dashpay_subscreen_chooser_panel(
            ctx,
            &self.app_context,
            DashPaySubscreen::ProfileSearch, // Use ProfileSearch as the active subscreen
        );

        // Main content area with island styling
        action |= island_central_panel(ctx, |ui| self.render(ui));

        // Handle custom action from top panel button
        if let AppAction::Custom(command) = &action
            && command == "clear_search"
        {
            self.search_query.clear();
            self.search_results.clear();
            self.has_searched = false;
            action = AppAction::None; // Consume the action
        }

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut popup =
                        InfoPopup::new("About Profile Search", PROFILE_SEARCH_INFO_TEXT);
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
        match result {
            BackendTaskSuccessResult::DashPayProfileSearchResults(results) => {
                self.loading = false;
                self.search_results.clear();

                // Convert backend results to UI results
                for (identity_id, profile_doc, username) in results {
                    // Extract profile data from document if available
                    use dash_sdk::dpp::document::DocumentV0Getters;
                    let (display_name, public_message, avatar_url) =
                        if let Some(document) = &profile_doc {
                            let properties = match document {
                                Document::V0(doc_v0) => doc_v0.properties(),
                            };
                            (
                                properties
                                    .get("displayName")
                                    .and_then(|v| v.as_text())
                                    .map(|s| s.to_string()),
                                properties
                                    .get("publicMessage")
                                    .and_then(|v| v.as_text())
                                    .map(|s| s.to_string()),
                                properties
                                    .get("avatarUrl")
                                    .and_then(|v| v.as_text())
                                    .map(|s| s.to_string()),
                            )
                        } else {
                            (None, None, None)
                        };

                    let search_result = ProfileSearchResult {
                        identity_id,
                        display_name,
                        public_message,
                        avatar_url,
                        username: Some(username), // DPNS username from search
                    };

                    self.search_results.push(search_result);
                }
            }
            BackendTaskSuccessResult::Message(_msg) => {
                // Message display is handled globally by AppState
            }
            _ => {
                // Ignore other results — don't touch loading state for
                // results from unrelated tasks
            }
        }
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        // Clear loading state on error so the spinner stops.
        // Return false to let AppState show the default error banner.
        self.loading = false;
        false
    }
}
