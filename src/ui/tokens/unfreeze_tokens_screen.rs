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

/// The states for the unfreeze flow
#[derive(PartialEq)]
pub enum UnfreezeTokensStatus {
    NotStarted,
    WaitingForResult(u64),
    ErrorMessage(String),
    Complete,
}

/// A screen that allows unfreezing a previously frozen identity’s tokens for a specific contract
pub struct UnfreezeTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_balance: IdentityTokenBalance,
    selected_key: Option<IdentityPublicKey>,

    /// Identity that’s currently frozen and we want to unfreeze
    unfreeze_identity_id: String,

    status: UnfreezeTokensStatus,
    error_message: Option<String>,

    // Basic references
    pub app_context: Arc<AppContext>,

    // Confirmation popup
    show_confirmation_popup: bool,

    // If password-based wallet unlocking is needed
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl UnfreezeTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let identity = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|id| id.identity.id() == identity_token_balance.identity_id)
            .expect("No local qualified identity found for this token’s identity");

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

        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, possible_key.clone(), &mut error_message);

        Self {
            identity,
            identity_token_balance,
            selected_key: possible_key.cloned(),
            unfreeze_identity_id: String::new(),
            status: UnfreezeTokensStatus::NotStarted,
            error_message: None,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    fn render_key_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Select Key:");
            egui::ComboBox::from_id_salt("unfreeze_key_selector")
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

    fn render_unfreeze_identity_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Identity to Unfreeze:");
            ui.text_edit_singleline(&mut self.unfreeze_identity_id);
        });
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Unfreeze")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Validate user input
                let parsed = Identifier::from_string_try_encodings(
                    &self.unfreeze_identity_id,
                    &[
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                        dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex,
                    ],
                );
                if parsed.is_err() {
                    self.error_message = Some("Please enter a valid identity ID.".into());
                    self.status = UnfreezeTokensStatus::ErrorMessage("Invalid identity ID".into());
                    self.show_confirmation_popup = false;
                    return;
                }
                let unfreeze_id = parsed.unwrap();

                ui.label(format!(
                    "Are you sure you want to unfreeze identity {}?",
                    self.unfreeze_identity_id
                ));

                ui.add_space(10.0);

                // Confirm
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = UnfreezeTokensStatus::WaitingForResult(now);

                    let data_contract = self
                        .app_context
                        .get_contracts(None, None)
                        .expect("Contracts not loaded")
                        .iter()
                        .find(|c| c.contract.id() == self.identity_token_balance.data_contract_id)
                        .expect("Data contract not found")
                        .contract
                        .clone();

                    // Dispatch to backend
                    action =
                        AppAction::BackendTask(BackendTask::TokenTask(TokenTask::UnfreezeTokens {
                            actor_identity: self.identity.clone(),
                            data_contract,
                            token_position: self.identity_token_balance.token_position,
                            signing_key: self.selected_key.clone().expect("No key selected"),
                            unfreeze_identity: unfreeze_id,
                        }));
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

    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Unfroze Successfully!");

            ui.add_space(20.0);

            if ui.button("Back to Tokens").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }
}

impl ScreenLike for UnfreezeTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                // Possibly "UnfreezeTokens" or something else from your backend
                if message.contains("Successfully unfroze identity") || message == "UnfreezeTokens"
                {
                    self.status = UnfreezeTokensStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = UnfreezeTokensStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {}
        }
    }

    fn refresh(&mut self) {
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
                ("Unfreeze", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.status == UnfreezeTokensStatus::Complete {
                action = self.show_success_screen(ui);
                return;
            }

            ui.heading("Unfreeze a Frozen Identity’s Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_authentication_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    Color32::RED,
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

                // 1) Key selection
                ui.heading("1. Select the key to sign the Unfreeze transition");
                ui.add_space(5.0);
                self.render_key_selection(ui);

                ui.separator();
                ui.add_space(10.0);

                // 2) Identity to unfreeze
                ui.heading("2. Enter the identity ID to unfreeze");
                ui.add_space(5.0);
                self.render_unfreeze_identity_input(ui);

                ui.add_space(10.0);

                // Unfreeze button
                let button = egui::Button::new(RichText::new("Unfreeze").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 128))
                    .rounding(3.0);

                if ui.add(button).clicked() {
                    self.show_confirmation_popup = true;
                }

                // If user pressed "Unfreeze," show popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    UnfreezeTokensStatus::NotStarted => {
                        // no-op
                    }
                    UnfreezeTokensStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Unfreezing... elapsed: {}s", elapsed));
                    }
                    UnfreezeTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::RED, format!("Error: {}", msg));
                    }
                    UnfreezeTokensStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for UnfreezeTokensScreen {
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
