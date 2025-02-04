use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};

use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};

use super::tokens_screen::IdentityTokenBalance;

/// Represents possible states in the “destroy frozen funds” flow
#[derive(PartialEq)]
pub enum DestroyFrozenFundsStatus {
    NotStarted,
    WaitingForResult(u64),
    ErrorMessage(String),
    Complete,
}

/// A screen for destroying frozen funds of a particular token contract
pub struct DestroyFrozenFundsScreen {
    /// Identity that is authorized to destroy
    pub identity: QualifiedIdentity,

    /// Info on which token contract we’re dealing with
    pub identity_token_balance: IdentityTokenBalance,

    /// The key used to sign the operation
    selected_key: Option<IdentityPublicKey>,

    /// The user must specify the identity ID whose frozen funds are to be destroyed
    /// Typically some Identity that has been frozen by the system or a group
    frozen_identity_id: String,

    status: DestroyFrozenFundsStatus,
    error_message: Option<String>,

    /// Basic references
    pub app_context: Arc<AppContext>,

    /// Confirmation popup
    show_confirmation_popup: bool,

    /// If password-based wallet unlocking is needed
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl DestroyFrozenFundsScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        // Locate the local identity that owns the token contract
        let identity = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|id| id.identity.id() == identity_token_balance.identity_id)
            .expect("No local qualified identity found matching this token's identity.");

        // Grab a suitable key
        let identity_clone = identity.identity.clone();
        let possible_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
                SecurityLevel::CRITICAL,
            ]),
            KeyType::all_key_types().into(),
            false,
        );

        // Possibly get an unlocked wallet
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, possible_key.clone(), &mut error_message);

        Self {
            identity,
            identity_token_balance,
            selected_key: possible_key.cloned(),
            frozen_identity_id: String::new(),
            status: DestroyFrozenFundsStatus::NotStarted,
            error_message: None,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    /// Renders a ComboBox or similar for selecting an authentication key
    fn render_key_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Select Key:");
            egui::ComboBox::from_id_salt("destroy_key_selector")
                .selected_text(match &self.selected_key {
                    Some(key) => format!("Key ID: {}", key.id()),
                    None => "Select a key".to_string(),
                })
                .show_ui(ui, |ui| {
                    if self.app_context.developer_mode {
                        // Show all loaded public keys
                        for key in self.identity.identity.public_keys().values() {
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    } else {
                        // Show only "available" auth keys
                        for key_wrapper in self.identity.available_authentication_keys() {
                            let key = &key_wrapper.identity_public_key;
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    }
                });
        });
    }

    /// Renders the text input for specifying the “frozen identity”
    fn render_frozen_identity_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Frozen Identity ID:");
            ui.text_edit_singleline(&mut self.frozen_identity_id);
        });
    }

    /// Confirmation popup
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Destroy Frozen Funds")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Parse the user input into an Identifier
                let maybe_frozen_id = Identifier::from_string_try_encodings(
                    &self.frozen_identity_id,
                    &[
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex,
                    ],
                );

                if maybe_frozen_id.is_err() {
                    self.error_message = Some("Invalid frozen identity format".into());
                    self.status = DestroyFrozenFundsStatus::ErrorMessage("Invalid identity".into());
                    self.show_confirmation_popup = false;
                    return;
                }

                let frozen_id = maybe_frozen_id.unwrap();

                ui.label(format!(
                    "Are you sure you want to destroy the frozen funds of identity {}?",
                    self.frozen_identity_id
                ));

                ui.add_space(10.0);

                // Confirm button
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = DestroyFrozenFundsStatus::WaitingForResult(now);

                    // Grab the data contract for this token from the app context
                    let data_contract = self
                        .app_context
                        .get_contracts(None, None)
                        .expect("Contracts not loaded")
                        .iter()
                        .find(|c| c.contract.id() == self.identity_token_balance.data_contract_id)
                        .expect("Data contract not found")
                        .contract
                        .clone();

                    // Dispatch the actual backend destroy action
                    action = AppAction::BackendTask(BackendTask::TokenTask(
                        TokenTask::DestroyFrozenFunds {
                            actor_identity: self.identity.clone(),
                            data_contract,
                            token_position: self.identity_token_balance.token_position,
                            signing_key: self.selected_key.clone().expect("Expected a key"),
                            frozen_identity: frozen_id,
                        },
                    ));
                }

                // Cancel
                if ui.button("Cancel").clicked() {
                    self.show_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_confirmation_popup = false;
        }
        action
    }

    /// Simple “Success” screen
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully destroyed frozen funds!");

            ui.add_space(20.0);

            if ui.button("Back to Tokens").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }
}

impl ScreenLike for DestroyFrozenFundsScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                // If your backend returns "DestroyFrozenFunds" on success,
                // or if there's a more descriptive success message:
                if message.contains("Successfully destroyed frozen funds")
                    || message == "DestroyFrozenFunds"
                {
                    self.status = DestroyFrozenFundsStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = DestroyFrozenFundsStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {
        // Reload the identity data if needed
        if let Ok(all_identities) = self.app_context.load_local_qualified_identities() {
            if let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity.identity.id())
            {
                self.identity = updated_identity;
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                ("Destroy", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.status == DestroyFrozenFundsStatus::Complete {
                action = self.show_success_screen(ui);
                return;
            }

            ui.heading("Destroy Frozen Funds");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_authentication_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    Color32::DARK_RED,
                    format!(
                        "No authentication keys found for this {} identity.",
                        self.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                // Show "Add key" or "Check keys" option
                let first_key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([
                        SecurityLevel::HIGH,
                        SecurityLevel::MEDIUM,
                        SecurityLevel::CRITICAL,
                    ]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Possibly handle locked wallet scenario
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);
                    if needed_unlock && !just_unlocked {
                        return;
                    }
                }

                // Key selection
                ui.heading("1. Select the key to sign the Destroy operation");
                ui.add_space(5.0);
                self.render_key_selection(ui);

                ui.separator();
                ui.add_space(10.0);

                // Frozen identity
                ui.heading("2. Frozen identity to destroy funds from");
                ui.add_space(5.0);
                self.render_frozen_identity_input(ui);

                ui.add_space(10.0);

                // Destroy button
                let button = egui::Button::new(RichText::new("Destroy").color(Color32::WHITE))
                    .fill(Color32::DARK_RED)
                    .rounding(3.0);

                if ui.add(button).clicked() {
                    self.show_confirmation_popup = true;
                }

                // If user pressed "Destroy," show a popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    DestroyFrozenFundsStatus::NotStarted => {
                        // no-op
                    }
                    DestroyFrozenFundsStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!(
                            "Destroying frozen funds... elapsed: {} seconds",
                            elapsed
                        ));
                    }
                    DestroyFrozenFundsStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
                    }
                    DestroyFrozenFundsStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for DestroyFrozenFundsScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}
