use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contract::ContractTask;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_everyday_spec};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{MessageType, RootScreenType, ScreenLike};

use base64::{Engine, engine::general_purpose::STANDARD};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::serialization::PlatformDeserializable;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Color32, ScrollArea, TextEdit, Ui, Window};
use egui::RichText;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Successfully parsed transition plus the text shown in the visualizer pane.
///
/// Keeping both values in one struct makes the broadcast invariant type-enforced:
/// if a transition is displayed, a typed `StateTransition` is always available
/// for broadcast without re-parsing the rendered output.
struct ParsedTransition {
    state_transition: StateTransition,
    rendered: String,
}

/// Render a parsed state transition for display.
///
/// Prefer pretty JSON. When serialization fails (e.g. non-string map keys in
/// platform-address transitions), fall back to a plain-text header plus the
/// multi-line `Debug` representation so newlines stay readable in the UI.
fn state_transition_to_display(state_transition: &StateTransition) -> String {
    match serde_json::to_string_pretty(state_transition) {
        Ok(json) => json,
        Err(error) => {
            let error = error.to_string();
            let debug_output = format!("{state_transition:#?}");
            tracing::warn!(
                error = %error,
                "parsed state transition but failed to serialize it to JSON; using debug fallback"
            );
            // Full transition payload only at DEBUG — never at WARN.
            tracing::debug!(
                state_transition = %debug_output,
                "debug representation of state transition that failed JSON serialization"
            );

            format!(
                "State transition parsed successfully, but JSON serialization failed.\n\
                 Serialization error: {error}\n\n\
                 Debug representation:\n\
                 {debug_output}"
            )
        }
    }
}

#[derive(PartialEq)]
enum TransitionBroadcastStatus {
    NotStarted,
    Submitting,
    Complete(Instant),
}

pub struct TransitionVisualizerScreen {
    pub app_context: Arc<AppContext>,
    input_data: String,
    parsed: Option<ParsedTransition>,
    parse_error: Option<(String, Instant)>,
    broadcast_status: TransitionBroadcastStatus,
    submit_banner: Option<BannerHandle>,
    show_contract_dialog: bool,
    selected_contract_id: Option<String>,
    detected_contract_ids: Vec<String>,
    contract_fetch_message: Option<(String, Instant)>,
}

impl TransitionVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            input_data: String::new(),
            parsed: None,
            parse_error: None,
            broadcast_status: TransitionBroadcastStatus::NotStarted,
            submit_banner: None,
            show_contract_dialog: false,
            selected_contract_id: None,
            detected_contract_ids: Vec::new(),
            contract_fetch_message: None,
        }
    }

    fn extract_contract_ids(value: &Value, ids: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                // Check if this is a contractBounds object with an id
                if map.contains_key("type")
                    && map.contains_key("id")
                    && let (Some(Value::String(type_str)), Some(Value::String(id))) =
                        (map.get("type"), map.get("id"))
                    && type_str == "singleContract"
                {
                    ids.push(id.clone());
                }
                // Recursively check all values
                for val in map.values() {
                    Self::extract_contract_ids(val, ids);
                }
            }
            Value::Array(arr) => {
                for val in arr {
                    Self::extract_contract_ids(val, ids);
                }
            }
            _ => {}
        }
    }

    fn parse_input(&mut self) {
        // Clear previous parse results...
        self.parsed = None;
        self.parse_error = None;
        self.detected_contract_ids.clear();

        // Reset the broadcast status so we no longer show old states
        // from a previous parse/broadcast.
        self.broadcast_status = TransitionBroadcastStatus::NotStarted;

        // First, try to parse as comma-separated integers
        let decoded_bytes = if self.input_data.contains(',') {
            // Try parsing as comma-separated integers
            self.input_data
                .split(',')
                .filter(|s| !s.trim().is_empty()) // Skip empty segments
                .map(|s| s.trim().parse::<u8>())
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|error| {
                    tracing::debug!(?error, "Transition byte-list parsing failed");
                    format!(
                        "The comma-separated values are not valid bytes. Use numbers from 0 to 255. ({error})"
                    )
                })
        } else {
            // Try to decode the input as hex first
            hex::decode(self.input_data.trim()).or_else(|_| {
                STANDARD.decode(self.input_data.trim()).map_err(|error| {
                    tracing::debug!(?error, "Transition base64 decoding failed");
                    format!(
                        "The input is not valid hexadecimal or base64 data. Check it and try again. ({error})"
                    )
                })
            })
        };

        match decoded_bytes {
            Ok(bytes) => {
                // Try to deserialize into a StateTransition
                match StateTransition::deserialize_from_bytes(&bytes) {
                    Ok(state_transition) => {
                        // Convert to JSON, falling back to a readable debug view for
                        // transition variants whose map keys cannot currently be
                        // represented as JSON object keys.
                        let rendered = state_transition_to_display(&state_transition);

                        // Extract contract IDs when the rendered output is real JSON.
                        // Fallback plain-text rendering intentionally has none.
                        if let Ok(json_value) = serde_json::from_str::<Value>(&rendered) {
                            Self::extract_contract_ids(
                                &json_value,
                                &mut self.detected_contract_ids,
                            );
                        }

                        self.parsed = Some(ParsedTransition {
                            state_transition,
                            rendered,
                        });
                    }
                    Err(error) => {
                        // Live typing frequently produces incomplete bytes; keep this at
                        // DEBUG so interactive use does not spam WARN logs.
                        tracing::debug!(?error, "State-transition deserialization failed");
                        self.parse_error = Some((
                            format!(
                                "The state transition could not be read. Check the input format and try again. ({error})"
                            ),
                            Instant::now(),
                        ));
                    }
                }
            }
            Err(e) => {
                self.parse_error = Some((e, Instant::now()));
            }
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {
        ui.label("Enter hex, base64, or comma-separated integers for state transition:");
        ui.add_space(5.0);
        let dark_mode = ui.style().visuals.dark_mode;
        let response = ui.add(
            TextEdit::multiline(&mut self.input_data)
                .desired_rows(6)
                .desired_width(ui.available_width())
                .text_color(DashColors::text_primary(dark_mode))
                .background_color(DashColors::input_background(dark_mode))
                .code_editor(),
        );

        ui.add_space(10.0);

        if response.changed() {
            // Re-parse
            self.parse_input();
        }
    }

    fn show_output(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;

        ui.separator();
        ui.add_space(10.0);
        ui.label("Parsed State Transition:");

        // Show detected contract IDs if any
        if !self.detected_contract_ids.is_empty() {
            ui.add_space(5.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label("📄 Contract references found:");
                    ui.add_space(10.0);
                    for (i, contract_id) in self.detected_contract_ids.iter().enumerate() {
                        if i > 0 {
                            ui.label("•");
                        }
                        if ui
                            .link(contract_id)
                            .clickable_tooltip("Click to view contract")
                            .clicked()
                        {
                            self.selected_contract_id = Some(contract_id.clone());
                            self.show_contract_dialog = true;
                        }
                    }
                });
            });
            ui.add_space(5.0);
        }

        // Show the rendered transition if we have it
        ScrollArea::vertical().show(ui, |ui| {
            if let Some(ParsedTransition {
                state_transition,
                rendered,
            }) = &self.parsed
            {
                ui.add_space(5.0);
                let dark_mode = ui.style().visuals.dark_mode;
                ui.add(
                    TextEdit::multiline(&mut rendered.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .text_color(DashColors::text_primary(dark_mode))
                        .background_color(DashColors::input_background(dark_mode))
                        .font(egui::TextStyle::Monospace),
                );

                ui.add_space(10.0);

                // Show the button when not currently submitting or done.
                // `ParsedTransition` guarantees a typed transition is available.
                if matches!(self.broadcast_status, TransitionBroadcastStatus::NotStarted) {
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);

                    if ComponentStyles::add_primary_button(ui, "Broadcast Transition to Platform")
                        .clicked()
                    {
                        // Mark as submitting
                        self.submit_banner.take_and_clear();
                        let handle = MessageBanner::set_global(
                            ui.ctx(),
                            "Submitting transition...",
                            MessageType::Info,
                        );
                        handle.with_elapsed();
                        self.submit_banner = Some(handle);
                        self.broadcast_status = TransitionBroadcastStatus::Submitting;

                        // Broadcast the retained typed transition — never re-parse
                        // the rendered pane (which may be a debug fallback).
                        app_action = AppAction::BackendTask(BackendTask::BroadcastStateTransition(
                            state_transition.clone(),
                        ));
                    }
                }
            } else {
                // If parsed is None
                if matches!(self.broadcast_status, TransitionBroadcastStatus::NotStarted) {
                    ui.colored_label(Color32::GRAY, "No state transition parsed yet.");
                }
            }
        });

        // Show parse error if any (with fade-out)
        ui.add_space(5.0);
        let mut clear_parse_error = false;
        if let Some((msg, timestamp)) = &self.parse_error {
            let elapsed = timestamp.elapsed();
            if elapsed < Duration::from_secs(8) {
                let alpha = if elapsed > Duration::from_secs(6) {
                    let fade_progress = (8.0 - elapsed.as_secs_f32()) / 2.0;
                    (fade_progress * 255.0) as u8
                } else {
                    255
                };
                ui.colored_label(
                    Color32::from_rgba_premultiplied(139, 0, 0, alpha), // Dark red
                    msg,
                );
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            } else {
                clear_parse_error = true;
            }
        }
        if clear_parse_error {
            self.parse_error = None;
        }

        // Show broadcast status
        match &self.broadcast_status {
            TransitionBroadcastStatus::NotStarted => {}
            TransitionBroadcastStatus::Submitting => {
                // Elapsed time is shown in the global banner
            }
            TransitionBroadcastStatus::Complete(timestamp) => {
                let elapsed = timestamp.elapsed();
                if elapsed < Duration::from_secs(8) {
                    // Calculate fade effect for last 2 seconds
                    let alpha = if elapsed > Duration::from_secs(6) {
                        let fade_progress = (8.0 - elapsed.as_secs_f32()) / 2.0;
                        (fade_progress * 255.0) as u8
                    } else {
                        255
                    };
                    ui.colored_label(
                        Color32::from_rgba_premultiplied(0, 100, 0, alpha), // Dark green
                        "Successfully broadcasted state transition.",
                    );

                    // Request repaint to update the fade effect
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                } else {
                    // Clear the status after 8 seconds
                    self.broadcast_status = TransitionBroadcastStatus::NotStarted;
                }
            }
        }

        // Show contract fetch success message if any
        let mut clear_message = false;
        if let Some((message, timestamp)) = &self.contract_fetch_message {
            let elapsed = timestamp.elapsed();
            if elapsed < Duration::from_secs(8) {
                ui.add_space(10.0);
                let message_text = message.clone();
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        // Calculate fade effect for last 2 seconds
                        let alpha = if elapsed > Duration::from_secs(6) {
                            let fade_progress = (8.0 - elapsed.as_secs_f32()) / 2.0;
                            (fade_progress * 255.0) as u8
                        } else {
                            255
                        };

                        ui.colored_label(
                            Color32::from_rgba_premultiplied(0, 150, 0, alpha),
                            &message_text,
                        );

                        ui.add_space(20.0);

                        // Add button with same fade effect
                        let button_color = Color32::from_rgba_premultiplied(70, 130, 180, alpha); // Steel blue
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("View in Contracts").color(
                                        Color32::from_rgba_premultiplied(255, 255, 255, alpha),
                                    ),
                                )
                                .fill(button_color)
                                .frame(true)
                                .min_size(egui::vec2(140.0, 0.0)),
                            )
                            .clicked()
                        {
                            app_action |=
                                AppAction::SetMainScreen(RootScreenType::RootScreenDocumentQuery);
                            clear_message = true; // Mark for clearing after the UI block
                        }
                    });
                });

                // Request repaint to update the message timeout and fade effect
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            } else {
                // Clear the message after 8 seconds
                clear_message = true;
            }
        }

        if clear_message {
            self.contract_fetch_message = None;
        }

        app_action
    }
}

impl ScreenLike for TransitionVisualizerScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        match message_type {
            MessageType::Success => {
                if matches!(self.broadcast_status, TransitionBroadcastStatus::Submitting) {
                    self.submit_banner.take_and_clear();
                    self.broadcast_status = TransitionBroadcastStatus::Complete(Instant::now());
                }
            }
            MessageType::Error | MessageType::Warning => {
                self.submit_banner.take_and_clear();
                self.broadcast_status = TransitionBroadcastStatus::NotStarted;
            }
            MessageType::Info => {}
        }
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::ui::BackendTaskSuccessResult::FetchedContract(contract) => {
                let contract_id = contract.id().to_string(Encoding::Base58);
                self.contract_fetch_message = Some((
                    format!("✅ Contract {contract_id} fetched successfully"),
                    Instant::now(),
                ));
            }
            crate::ui::BackendTaskSuccessResult::FetchedContracts(contracts) => {
                let count = contracts.iter().filter(|c| c.is_some()).count();
                self.contract_fetch_message = Some((
                    format!("✅ {count} contract(s) fetched successfully"),
                    Instant::now(),
                ));
            }
            _ => {
                // Other results are handled globally
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel_with_global_nav(
            ui,
            &self.app_context,
            subdued_everyday_spec(
                "Tools",
                RootScreenType::RootScreenToolsTransitionVisualizerScreen,
            ),
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ui, self.app_context.as_ref());

        action |= island_central_panel(ui, |ui| {
            self.show_input_field(ui);
            self.show_output(ui)
        });

        // Show contract fetch dialog if needed
        if self.show_contract_dialog {
            let mut dialog_action = AppAction::None;
            let mut close_dialog = false;

            Window::new("Fetch Contract")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(10.0);

                        if let Some(ref contract_id) = self.selected_contract_id {
                            ui.label(format!("Contract ID: {contract_id}"));
                            ui.add_space(10.0);

                            // Check if contract already exists
                            let contract_exists = self
                                .app_context
                                .get_contracts()
                                .unwrap_or_default()
                                .iter()
                                .any(|c| {
                                    c.contract.id().to_string(Encoding::Base58) == *contract_id
                                });

                            if contract_exists {
                                ui.label("This contract already exists locally.");
                                ui.add_space(10.0);

                                if ui.button("Go to Contract").clicked() {
                                    // Navigate to contract screen
                                    action |= AppAction::SetMainScreen(
                                        RootScreenType::RootScreenDocumentQuery,
                                    );
                                    close_dialog = true;
                                }
                            } else {
                                ui.label("Would you like to fetch this contract from Platform?");
                                ui.add_space(10.0);

                                ui.horizontal(|ui| {
                                    if ui.button("Yes, Fetch").clicked() {
                                        // Parse the contract ID string to Identifier
                                        if let Ok(identifier) =
                                            Identifier::from_string(contract_id, Encoding::Base58)
                                        {
                                            dialog_action = AppAction::BackendTask(
                                                BackendTask::ContractTask(Box::new(
                                                    ContractTask::FetchContracts(vec![identifier]),
                                                )),
                                            );
                                        }
                                        close_dialog = true;
                                    }

                                    if ui.button("Cancel").clicked() {
                                        close_dialog = true;
                                    }
                                });
                            }
                        }

                        ui.add_space(10.0);
                    });
                });

            if close_dialog {
                self.show_contract_dialog = false;
                self.selected_contract_id = None;
            }

            action |= dialog_action;
        }

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_dir::{copy_env_file_if_not_exists, ensure_env_file};
    use crate::database::test_helpers::create_database_at_path;
    use dash_sdk::dpp::dashcore::Network;
    use std::sync::Once;

    /// Issue #573 AddressFundsTransfer payload. Historically this hit a
    /// `serde_json` non-string map-key failure; on current platform pins it may
    /// serialize as JSON. Either path must still deserialize and remain visible.
    const ISSUE_573_BASE64: &str = "DAABAEZJ8lqSWSBK4JRgM6A4o7EoAM/IDPwAD0JAAQA2I7Ax2quQXVnyAKnezuNPE75q7vwAD0JAAQAAAAEAQR/vseNQooUa8QVJbujMgP22M8EzL3C9AAEibx6iMtmWHA1ou89lGCCtHgJmKcieCtdhZMhqcnU9O+fc4Y575ryF";

    /// True when the rendered pane identifies the issue #573 variant.
    /// JSON uses `$type: "addressFundsTransfer"`; Debug uses `AddressFundsTransfer(...)`.
    fn renders_address_funds_transfer(rendered: &str) -> bool {
        rendered.contains("addressFundsTransfer") || rendered.contains("AddressFundsTransfer")
    }

    fn ensure_test_env() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            copy_env_file_if_not_exists();

            // Safety: tests set env vars once for deterministic config.
            unsafe {
                std::env::set_var("MAINNET_dapi_addresses", "http://127.0.0.1:1443");
                std::env::set_var("MAINNET_core_host", "127.0.0.1");
                std::env::set_var("MAINNET_core_rpc_port", "9998");
                std::env::set_var("MAINNET_core_rpc_user", "dashrpc");
                std::env::set_var("MAINNET_core_rpc_password", "password");

                std::env::set_var("LOCAL_dapi_addresses", "http://127.0.0.1:2443");
                std::env::set_var("LOCAL_core_host", "127.0.0.1");
                std::env::set_var("LOCAL_core_rpc_port", "20302");
                std::env::set_var("LOCAL_core_rpc_user", "dashmate");
                std::env::set_var("LOCAL_core_rpc_password", "password");
            }
        });
    }

    /// Hold the tempdir for the lifetime of the screen so the app k/v and
    /// secret store files stay available for the duration of the test.
    struct TestScreen {
        screen: TransitionVisualizerScreen,
        _dir: tempfile::TempDir,
    }

    fn make_test_screen() -> TestScreen {
        ensure_test_env();

        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().to_path_buf();
        ensure_env_file(&data_dir);

        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("test db"));
        // Each test gets its own scratch directory for app k/v + secret store
        // so concurrent tests do not fight over the same SQLite file.
        let app_kv = AppContext::open_app_kv(&data_dir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("open secret store");

        let app_context = AppContext::new(
            data_dir,
            Network::Regtest,
            db,
            Default::default(),
            Default::default(),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("Expected to create AppContext");

        TestScreen {
            screen: TransitionVisualizerScreen::new(&app_context),
            _dir: dir,
        }
    }

    #[test]
    fn issue_573_platform_address_transition_deserializes_and_renders() {
        let bytes = STANDARD
            .decode(ISSUE_573_BASE64)
            .expect("issue #573 base64 should decode");
        let state_transition = StateTransition::deserialize_from_bytes(&bytes)
            .expect("issue #573 transition should deserialize");

        let rendered = state_transition_to_display(&state_transition);

        assert!(
            !rendered.trim().is_empty(),
            "parsed transition must produce visible output"
        );
        assert!(
            renders_address_funds_transfer(&rendered),
            "rendered output should identify the AddressFundsTransfer variant"
        );

        let json_ok = serde_json::to_string_pretty(&state_transition).is_ok();
        if json_ok {
            // Prefer the JSON path when the current pin can serialize it.
            assert!(
                rendered.trim_start().starts_with('{'),
                "successful JSON serialization should yield pretty JSON"
            );
            assert!(
                serde_json::from_str::<Value>(&rendered).is_ok(),
                "JSON path must produce parseable JSON"
            );
        } else {
            // Fallback path: plain-text header + multi-line Debug body (not an
            // escaped JSON string blob). Do not assert upstream error wording.
            assert!(
                rendered.contains("parsed successfully"),
                "fallback should explain that parsing succeeded"
            );
            assert!(
                rendered.contains("Debug representation:"),
                "fallback should include a debug section"
            );
            assert!(
                rendered.lines().count() > 3,
                "fallback should preserve multi-line debug output"
            );
        }
    }

    #[test]
    fn parse_input_retains_typed_transition_for_issue_573_payload() {
        let mut fixture = make_test_screen();
        fixture.screen.input_data = ISSUE_573_BASE64.to_owned();
        fixture.screen.parse_input();

        let parsed = fixture
            .screen
            .parsed
            .as_ref()
            .expect("issue #573 payload should parse into a retained transition");

        assert!(
            fixture.screen.parse_error.is_none(),
            "successful deserialize must not set a parse error regardless of JSON serialization"
        );
        assert!(
            !parsed.rendered.trim().is_empty(),
            "rendered pane must show the parsed transition"
        );
        assert!(
            renders_address_funds_transfer(&parsed.rendered),
            "rendered pane should identify the AddressFundsTransfer variant"
        );
        // Typed transition is retained — broadcast must not need to re-parse rendered text.
        // Touch the field so a future refactor that drops it fails this test.
        let _retained: &StateTransition = &parsed.state_transition;
    }
}
