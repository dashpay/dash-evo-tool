use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::{Document, Identifier};
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

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
    message: Option<(String, MessageType)>,
    loading: bool,
    has_searched: bool, // Track if a search has been performed
}

impl ProfileSearchScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            search_query: String::new(),
            search_results: Vec::new(),
            message: None,
            loading: false,
            has_searched: false,
        }
    }

    fn search_profiles(&mut self) -> AppAction {
        if self.search_query.trim().is_empty() {
            self.display_message("Please enter a search term", MessageType::Error);
            return AppAction::None;
        }

        // Use any available identity for the search (just needed for SDK context)
        // The identity doesn't affect what profiles can be searched - they're all public
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();
        if identities.is_empty() {
            self.display_message(
                "No identities available. Please load an identity first.",
                MessageType::Error,
            );
            return AppAction::None;
        }

        self.loading = true;
        self.search_results.clear();
        self.has_searched = true; // Mark that a search has been performed

        let task = BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles {
            identity: identities[0].clone(), // Just use any available identity
            search_query: self.search_query.trim().to_string(),
        }));

        AppAction::BackendTask(task)
    }

    fn view_profile(&mut self, identity_id: Identifier) -> AppAction {
        // Use any available identity for viewing (just needed for context)
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();
        if identities.is_empty() {
            self.display_message(
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
            crate::ui::helpers::info_icon_button(
                ui,
                "About Profile Search:\n\n\
                • Search for public DashPay profiles on the Platform\n\
                • Search by display name, username, or identity ID\n\
                • View anyone's public profile information\n\
                • Add contacts directly from search results",
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

        ScrollArea::vertical().show(ui, |ui| {
            // Search section
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    ui.add(
                        TextEdit::singleline(&mut self.search_query)
                            .hint_text("Enter display name, username, or identity ID...")
                            .desired_width(400.0),
                    );
                });

                if ui.button("Search").clicked() {
                    action = self.search_profiles();
                }

                if ui.button("Clear").clicked() {
                    self.search_query.clear();
                    self.search_results.clear();
                    self.message = None;
                    self.has_searched = false; // Reset search state
                }
            });

            ui.label(
                RichText::new(
                    "Tip: You can search by partial display name, @username, or full identity ID",
                )
                .small()
                .color(DashColors::text_secondary(dark_mode)),
            );

            ui.add_space(10.0);

            // Loading indicator
            if self.loading {
                ui.horizontal(|ui| {
                    let spinner_color = if dark_mode {
                        egui::Color32::from_gray(200)
                    } else {
                        egui::Color32::from_gray(60)
                    };
                    ui.add(egui::widgets::Spinner::default().color(spinner_color));
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
                                // Avatar placeholder
                                ui.add(egui::Label::new(RichText::new("👤").size(30.0)));

                                ui.vertical(|ui| {
                                    // Display name
                                    if let Some(display_name) = &result.display_name {
                                        ui.label(
                                            RichText::new(display_name)
                                                .strong()
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new("No display name")
                                                .color(DashColors::text_secondary(dark_mode))
                                                .italics(),
                                        );
                                    }

                                    // Username
                                    if let Some(username) = &result.username {
                                        ui.label(
                                            RichText::new(format!("@{}", username))
                                                .small()
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
                                            action = self.view_profile(result.identity_id);
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
                // Only show "No profiles found" if we've actually performed a search
                ui.group(|ui| {
                    ui.label("No profiles found");
                    ui.separator();
                    ui.label("Try searching with different terms or check the identity ID format.");
                });
            }
        });

        action
    }

    pub fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.loading = false;
        self.message = Some((message.to_string(), message_type));
    }
}

impl ScreenLike for ProfileSearchScreen {
    fn ui(&mut self, ctx: &egui::Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Profile Search", AppAction::None),
            ],
            vec![],
        );

        // Add left panel for DashPay navigation
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDashPayProfileSearch,
        );

        // Add DashPay subscreen chooser panel
        action |= add_dashpay_subscreen_chooser_panel(
            ctx,
            &self.app_context,
            DashPaySubscreen::ProfileSearch, // Use ProfileSearch as the active subscreen
        );

        // Main content area with island styling
        action |= island_central_panel(ctx, |ui| self.render(ui));

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.display_message(message, message_type);
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.loading = false;

        match result {
            BackendTaskSuccessResult::DashPayProfileSearchResults(results) => {
                self.search_results.clear();

                // Convert backend results to UI results
                for (identity_id, document) in results {
                    // Extract profile data from document
                    use dash_sdk::dpp::document::DocumentV0Getters;
                    let properties = match &document {
                        Document::V0(doc_v0) => doc_v0.properties(),
                    };

                    let search_result = ProfileSearchResult {
                        identity_id,
                        display_name: properties
                            .get("displayName")
                            .and_then(|v| v.as_text())
                            .map(|s| s.to_string()),
                        public_message: properties
                            .get("publicMessage")
                            .and_then(|v| v.as_text())
                            .map(|s| s.to_string()),
                        avatar_url: properties
                            .get("avatarUrl")
                            .and_then(|v| v.as_text())
                            .map(|s| s.to_string()),
                        username: None, // TODO: Resolve from DPNS if needed
                    };

                    self.search_results.push(search_result);
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
