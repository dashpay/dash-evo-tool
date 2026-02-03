//! State Transition Signing Screen
//!
//! Displays the state transition request details and allows the user to approve or reject
//! signing and broadcasting the state transition.

use crate::app::AppAction;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::state_transition_request::StateTransitionRequest;
use crate::model::wallet::Wallet;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::get_selected_wallet;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::IdentityPublicKey;
use egui::{Color32, Context, RichText, ScrollArea, TextEdit, Ui};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Status of the signing process
#[derive(Debug, Clone, PartialEq)]
enum SigningStatus {
    /// Waiting for user to approve or reject
    PendingApproval,
    /// Processing the request
    Processing,
    /// Successfully completed
    Success,
    /// Failed with error
    Error(String),
}

/// Screen for confirming a state transition signing request
pub struct StateTransitionSigningScreen {
    pub app_context: Arc<AppContext>,
    request: StateTransitionRequest,
    request_network: Network,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    status: SigningStatus,
    message: Option<(String, MessageType)>,
    json_preview: Option<String>,
}

impl StateTransitionSigningScreen {
    /// Create a new state transition signing screen
    pub fn new(
        app_context: Arc<AppContext>,
        request: StateTransitionRequest,
        request_network: Network,
    ) -> Self {
        // Pre-generate JSON preview
        let json_preview = request.to_json_pretty().ok();

        // Try to pre-select identity if hint is provided
        let mut selected_identity = None;
        let mut selected_identity_string = String::new();

        if let Some(identity_hint) = &request.identity_hint
            && let Ok(identities) = app_context.load_local_qualified_identities()
        {
            for identity in identities {
                if identity.identity.id() == *identity_hint {
                    selected_identity_string = identity.display_string();
                    selected_identity = Some(identity);
                    break;
                }
            }
        }

        // Also try to match by owner ID from the state transition
        if selected_identity.is_none()
            && let Some(owner_id) = request.owner_identity_id()
            && let Ok(identities) = app_context.load_local_qualified_identities()
        {
            for identity in identities {
                if identity.identity.id() == owner_id {
                    selected_identity_string = identity.display_string();
                    selected_identity = Some(identity);
                    break;
                }
            }
        }

        Self {
            app_context,
            request,
            request_network,
            selected_identity,
            selected_identity_string,
            selected_key: None,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            status: SigningStatus::PendingApproval,
            message: None,
            json_preview,
        }
    }

    /// Handle the approve action
    fn approve_request(&mut self) -> AppAction {
        let Some(identity) = self.selected_identity.clone() else {
            self.message = Some(("Please select an identity".to_string(), MessageType::Error));
            return AppAction::None;
        };

        let Some(signing_key) = self.selected_key.clone() else {
            self.message = Some((
                "Please select a signing key".to_string(),
                MessageType::Error,
            ));
            return AppAction::None;
        };

        // Validate identity hint if provided
        if let Some(identity_hint) = &self.request.identity_hint
            && identity.identity.id() != *identity_hint
        {
            self.message = Some((
                format!(
                    "Identity mismatch: request specifies {} but you selected {}",
                    identity_hint.to_string(Encoding::Base58),
                    identity.identity.id().to_string(Encoding::Base58)
                ),
                MessageType::Error,
            ));
            return AppAction::None;
        }

        // Validate key hint if provided
        if let Some(key_id_hint) = self.request.key_id_hint
            && signing_key.id() != key_id_hint
        {
            self.message = Some((
                format!(
                    "Key mismatch: request specifies key {} but you selected key {}",
                    key_id_hint,
                    signing_key.id()
                ),
                MessageType::Error,
            ));
            return AppAction::None;
        }

        self.status = SigningStatus::Processing;

        // Create the signing task
        let task = BackendTask::SignAndBroadcastStateTransition {
            identity,
            signing_key,
            state_transition: Box::new(self.request.state_transition.clone()),
        };

        AppAction::BackendTask(task)
    }

    /// Render the main content
    fn render_content(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Header
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Sign State Transition Request");
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
            ui.add_space(10.0);
        }

        match &self.status {
            SigningStatus::Success => {
                return self.render_success_screen(ui);
            }
            SigningStatus::Error(error) => {
                return self.render_error_screen(ui, error.clone());
            }
            SigningStatus::Processing => {
                ui.horizontal(|ui| {
                    ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                    ui.label(
                        RichText::new("Signing and broadcasting state transition...")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                return action;
            }
            SigningStatus::PendingApproval => {}
        }

        // Check network mismatch - this is a HARD block, not just a warning
        if self.request_network != self.app_context.network {
            self.render_network_mismatch_error(ui);
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {
            // Security warning banner (always show)
            self.render_security_warning(ui);

            // High-risk warning (if applicable)
            if self.request.is_high_risk() {
                ui.add_space(10.0);
                self.render_high_risk_warning(ui);
            }

            ui.add_space(15.0);

            // Request details
            self.render_request_details(ui);

            ui.add_space(15.0);

            // Identity selector
            action |= self.render_identity_selector(ui);

            ui.add_space(15.0);

            // Key selector
            self.render_key_selector(ui);

            ui.add_space(15.0);

            // JSON preview
            self.render_json_preview(ui);

            ui.add_space(15.0);

            // Action buttons
            action |= self.render_action_buttons(ui);
        });

        action
    }

    /// Render the network mismatch error (hard block - no approve button)
    fn render_network_mismatch_error(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.colored_label(
                DashColors::error_color(dark_mode),
                RichText::new("NETWORK MISMATCH - CANNOT PROCEED").strong(),
            );
            ui.separator();

            ui.label(format!(
                "This request is for {} but you are connected to {}.",
                network_display_name(self.request_network),
                network_display_name(self.app_context.network)
            ));

            ui.add_space(10.0);

            ui.colored_label(
                DashColors::error_color(dark_mode),
                "For security reasons, you cannot sign state transitions for a different network.",
            );

            ui.add_space(10.0);

            ui.label("Please either:");
            ui.label("  1. Switch to the correct network in Settings");
            ui.label("  2. Or request a new state transition URI for your current network");
        });
    }

    /// Render the security warning banner
    fn render_security_warning(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Security Notice")
                        .strong()
                        .color(Color32::from_rgb(255, 165, 0)),
                );
            });
            ui.separator();

            ui.label(
                RichText::new(
                    "An external application is requesting you to sign and broadcast a state transition. \
                     Review the details carefully before approving.",
                )
                .color(DashColors::text_primary(dark_mode)),
            );

            ui.add_space(5.0);

            ui.label(
                RichText::new(
                    "Only approve if you trust the application and understand what this transition does.",
                )
                .small()
                .color(DashColors::text_secondary(dark_mode)),
            );
        });
    }

    /// Render the high-risk warning for dangerous operations
    fn render_high_risk_warning(&self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("HIGH RISK OPERATION")
                        .strong()
                        .color(Color32::from_rgb(255, 0, 0)),
                );
            });
            ui.separator();

            ui.colored_label(
                Color32::from_rgb(255, 0, 0),
                RichText::new(
                    self.request
                        .high_risk_reason()
                        .unwrap_or("This is a high-risk operation. Verify the details carefully."),
                ),
            );

            ui.add_space(5.0);

            ui.colored_label(
                Color32::from_rgb(255, 100, 100),
                "Double-check the recipient addresses and amounts before approving.",
            );
        });
    }

    /// Render the request details
    fn render_request_details(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.label(
                RichText::new("Request Details")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.separator();

            egui::Grid::new("st_request_details")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    // Application name
                    ui.label(
                        RichText::new("Application:").color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(self.request.display_name())
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();

                    // Transition type
                    ui.label(
                        RichText::new("Transition Type:")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    let type_color = if self.request.is_high_risk() {
                        Color32::from_rgb(255, 100, 100)
                    } else {
                        DashColors::text_primary(dark_mode)
                    };
                    ui.label(
                        RichText::new(self.request.transition_type())
                            .strong()
                            .color(type_color),
                    );
                    ui.end_row();

                    // Network
                    ui.label(
                        RichText::new("Network:").color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(network_display_name(self.request_network))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();

                    // Owner identity (if available)
                    if let Some(owner_id) = self.request.owner_identity_id() {
                        ui.label(
                            RichText::new("Owner Identity:")
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        let owner_str = owner_id.to_string(Encoding::Base58);
                        let display_id = if owner_str.len() > 20 {
                            format!(
                                "{}...{}",
                                &owner_str[..10],
                                &owner_str[owner_str.len() - 10..]
                            )
                        } else {
                            owner_str.clone()
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&display_id)
                                    .monospace()
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            if ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(owner_str);
                            }
                        });
                        ui.end_row();
                    }

                    // Identity hint (if provided)
                    if let Some(identity_hint) = &self.request.identity_hint {
                        ui.label(
                            RichText::new("Required Identity:")
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        let hint_str = identity_hint.to_string(Encoding::Base58);
                        let display_id = if hint_str.len() > 20 {
                            format!("{}...{}", &hint_str[..10], &hint_str[hint_str.len() - 10..])
                        } else {
                            hint_str.clone()
                        };
                        ui.label(
                            RichText::new(&display_id)
                                .monospace()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.end_row();
                    }

                    // Key hint (if provided)
                    if let Some(key_id_hint) = self.request.key_id_hint {
                        ui.label(
                            RichText::new("Required Key ID:")
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        ui.label(
                            RichText::new(format!("{}", key_id_hint))
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.end_row();
                    }

                    // Protocol version
                    ui.label(
                        RichText::new("Protocol Version:")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format!("{}", self.request.version))
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.end_row();
                });
        });
    }

    /// Render the identity selector
    fn render_identity_selector(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            ui.group(|ui| {
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    "No identities found. Please add an identity first.",
                );
            });
            return action;
        }

        ui.group(|ui| {
            ui.label(
                RichText::new("Select Identity")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.separator();

            // Show hint constraint if present
            if let Some(identity_hint) = &self.request.identity_hint {
                ui.colored_label(
                    Color32::from_rgb(255, 165, 0),
                    format!(
                        "Request requires identity: {}",
                        identity_hint.to_string(Encoding::Base58)
                    ),
                );
                ui.add_space(5.0);
            }

            ui.label(
                RichText::new("Choose which identity will sign this transition:")
                    .color(DashColors::text_secondary(dark_mode)),
            );

            ui.add_space(5.0);

            // Track previous identity to detect changes
            let prev_identity_id = self.selected_identity.as_ref().map(|i| i.identity.id());

            ui.horizontal(|ui| {
                ui.label("Identity:");
                ui.add(
                    IdentitySelector::new(
                        "st_signing_identity_selector",
                        &mut self.selected_identity_string,
                        &identities,
                    )
                    .selected_identity(&mut self.selected_identity)
                    .unwrap()
                    .width(300.0)
                    .other_option(false),
                );
            });

            // Update wallet and key if identity changed
            let new_identity_id = self.selected_identity.as_ref().map(|i| i.identity.id());

            if prev_identity_id != new_identity_id {
                if let Some(identity) = &self.selected_identity {
                    let mut error_message = None;
                    self.selected_wallet = get_selected_wallet(
                        identity,
                        Some(&self.app_context),
                        None,
                        &mut error_message,
                    );
                    if let Some(error) = error_message {
                        self.message = Some((error, MessageType::Error));
                    }

                    // Auto-select key if hint is provided
                    if let Some(key_id_hint) = self.request.key_id_hint {
                        self.selected_key = identity
                            .identity
                            .public_keys()
                            .values()
                            .find(|k| k.id() == key_id_hint)
                            .cloned();
                    } else {
                        // Auto-select first suitable authentication key
                        self.selected_key = identity
                            .identity
                            .get_first_public_key_matching(
                                Purpose::AUTHENTICATION,
                                HashSet::from([
                                    SecurityLevel::CRITICAL,
                                    SecurityLevel::HIGH,
                                    SecurityLevel::MEDIUM,
                                ]),
                                HashSet::from([KeyType::ECDSA_SECP256K1, KeyType::ECDSA_HASH160]),
                                false,
                            )
                            .cloned();
                    }
                } else {
                    self.selected_wallet = None;
                    self.selected_key = None;
                }
            }
        });

        action
    }

    /// Render the key selector
    fn render_key_selector(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.label(
                RichText::new("Select Signing Key")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.separator();

            // Show hint constraint if present
            if let Some(key_id_hint) = self.request.key_id_hint {
                ui.colored_label(
                    Color32::from_rgb(255, 165, 0),
                    format!("Request requires key ID: {}", key_id_hint),
                );
                ui.add_space(5.0);
            }

            if let Some(identity) = &self.selected_identity {
                let keys: Vec<_> = identity
                    .identity
                    .public_keys()
                    .values()
                    .filter(|k| {
                        k.purpose() == Purpose::AUTHENTICATION
                            && identity.private_keys.has(&(k.purpose().into(), k.id()))
                    })
                    .collect();

                if keys.is_empty() {
                    ui.colored_label(
                        DashColors::error_color(dark_mode),
                        "No suitable signing keys found for this identity.",
                    );
                } else {
                    egui::ComboBox::from_label("Signing Key")
                        .selected_text(
                            self.selected_key
                                .as_ref()
                                .map(|k| format!("Key {} ({:?})", k.id(), k.key_type()))
                                .unwrap_or_else(|| "Select a key".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for key in keys {
                                let label = format!(
                                    "Key {} - {:?} ({:?})",
                                    key.id(),
                                    key.security_level(),
                                    key.key_type()
                                );
                                let is_selected = self
                                    .selected_key
                                    .as_ref()
                                    .is_some_and(|k| k.id() == key.id());
                                if ui.selectable_label(is_selected, label).clicked() {
                                    self.selected_key = Some(key.clone());
                                }
                            }
                        });
                }
            } else {
                ui.label(
                    RichText::new("Please select an identity first")
                        .color(DashColors::text_secondary(dark_mode)),
                );
            }
        });
    }

    /// Render the JSON preview
    fn render_json_preview(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.label(
                RichText::new("State Transition Preview (JSON)")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.separator();

            if let Some(json) = &self.json_preview {
                let mut json_text = json.clone();
                ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut json_text)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false),
                    );
                });

                if ui.button("Copy JSON").clicked() {
                    ui.ctx().copy_text(json.clone());
                }
            } else {
                ui.label(
                    RichText::new("Failed to serialize state transition to JSON")
                        .color(DashColors::error_color(dark_mode)),
                );
            }
        });
    }

    /// Render the action buttons
    fn render_action_buttons(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.group(|ui| {
            // Check wallet lock status
            let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                if let Err(e) = try_open_wallet_no_password(wallet) {
                    self.message = Some((e, MessageType::Error));
                }
                wallet_needs_unlock(wallet)
            } else {
                false
            };

            if wallet_locked {
                ui.colored_label(
                    Color32::from_rgb(200, 150, 50),
                    "Wallet is locked. Please unlock to proceed.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reject").clicked() {
                        action = AppAction::PopScreen;
                    }
                    ui.add_space(10.0);
                    if ui.button("Unlock Wallet").clicked() {
                        self.wallet_unlock_popup.open();
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    // Reject button
                    if ui.button("Reject").clicked() {
                        action = AppAction::PopScreen;
                    }

                    ui.add_space(20.0);

                    // Approve button
                    let can_approve = self.selected_identity.is_some()
                        && self.selected_key.is_some()
                        && self.request_network == self.app_context.network;

                    let approve_color = if self.request.is_high_risk() {
                        Color32::from_rgb(200, 50, 50) // Red for high-risk
                    } else if can_approve {
                        DashColors::DASH_BLUE
                    } else {
                        Color32::GRAY
                    };

                    let approve_text = if self.request.is_high_risk() {
                        "I Understand - Sign & Broadcast"
                    } else {
                        "Approve & Broadcast"
                    };

                    let approve_button =
                        egui::Button::new(RichText::new(approve_text).color(Color32::WHITE))
                            .fill(approve_color);

                    if ui.add_enabled(can_approve, approve_button).clicked() {
                        action = self.approve_request();
                    }
                });
            }
        });

        action
    }

    /// Render the success screen
    fn render_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen(
            ui,
            format!(
                "State transition '{}' broadcasted successfully!",
                self.request.display_name()
            ),
            vec![("Done".to_string(), AppAction::PopScreen)],
        )
    }

    /// Render the error screen
    fn render_error_screen(&mut self, ui: &mut Ui, error: String) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.group(|ui| {
            ui.colored_label(
                DashColors::error_color(dark_mode),
                RichText::new("Broadcast Failed").strong(),
            );
            ui.separator();

            ui.label(RichText::new(&error).color(DashColors::error_color(dark_mode)));

            ui.add_space(15.0);

            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    action = AppAction::PopScreen;
                }

                ui.add_space(10.0);

                if ui.button("Try Again").clicked() {
                    self.status = SigningStatus::PendingApproval;
                    self.message = None;
                }
            });
        });

        action
    }
}

impl ScreenLike for StateTransitionSigningScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tools", AppAction::None),
                ("Sign State Transition", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        action |= island_central_panel(ctx, |ui| self.render_content(ui));

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked, UI will update on next frame
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type));
        if message_type == MessageType::Error {
            self.status = SigningStatus::Error(message.to_string());
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::BroadcastedStateTransition => {
                self.status = SigningStatus::Success;
            }
            BackendTaskSuccessResult::Message(message) => {
                if message.contains("Error") || message.contains("Failed") {
                    self.status = SigningStatus::Error(message);
                }
            }
            _ => {}
        }
    }
}

/// Get a user-friendly network name
fn network_display_name(network: Network) -> &'static str {
    match network {
        Network::Dash => "Mainnet",
        Network::Testnet => "Testnet",
        Network::Devnet => "Devnet",
        Network::Regtest => "Regtest",
        _ => "Unknown",
    }
}
