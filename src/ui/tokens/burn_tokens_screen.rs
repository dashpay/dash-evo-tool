use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;

use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::IdentityPublicKey;

use crate::app::{AppAction, BackendTasksExecutionMode};
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

/// Internal states for the burn process.
#[derive(PartialEq)]
pub enum BurnTokensStatus {
    NotStarted,
    WaitingForResult(u64),
    ErrorMessage(String),
    Complete,
}

pub struct BurnTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_balance: IdentityTokenBalance, // Info on which token/contract to burn from
    selected_key: Option<IdentityPublicKey>,

    // The user chooses how many tokens to burn
    amount_to_burn: String,
    public_note: Option<String>,

    status: BurnTokensStatus,
    error_message: Option<String>,

    // Basic references
    pub app_context: Arc<AppContext>,

    // Confirmation popup
    show_confirmation_popup: bool,

    // For password-based wallet unlocking, if needed
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl BurnTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        // Find the local qualified identity that corresponds to `identity_token_balance.identity_id`
        let identity = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|id| id.identity.id() == identity_token_balance.identity_id)
            .expect("No local qualified identity found matching this token's identity.");

        // Grab a default signing key if possible
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

        // Attempt to get an unlocked wallet reference
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, possible_key.clone(), &mut error_message);

        Self {
            identity,
            identity_token_balance,
            selected_key: possible_key.cloned(),
            amount_to_burn: String::new(),
            public_note: None,
            status: BurnTokensStatus::NotStarted,
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
            egui::ComboBox::from_id_salt("burn_key_selector")
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

    /// Renders a text input for the user to specify an amount to burn
    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Amount to Burn:");
            ui.text_edit_singleline(&mut self.amount_to_burn);
        });
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Burn")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Validate user input
                let amount_ok = self.amount_to_burn.parse::<u64>().ok();
                if amount_ok.is_none() {
                    self.error_message = Some("Please enter a valid integer amount.".into());
                    self.status = BurnTokensStatus::ErrorMessage("Invalid amount".into());
                    self.show_confirmation_popup = false;
                    return;
                }

                ui.label(format!(
                    "Are you sure you want to burn {} tokens?",
                    self.amount_to_burn
                ));

                ui.add_space(10.0);

                // Confirm button
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = BurnTokensStatus::WaitingForResult(now);

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

                    // Dispatch the actual backend burn action
                    action = AppAction::BackendTasks(
                        vec![
                            BackendTask::TokenTask(TokenTask::BurnTokens {
                                owner_identity: self.identity.clone(),
                                data_contract,
                                token_position: self.identity_token_balance.token_position,
                                signing_key: self.selected_key.clone().expect("Expected a key"),
                                public_note: self.public_note.clone(),
                                amount: amount_ok.unwrap(),
                            }),
                            BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
                        ],
                        BackendTasksExecutionMode::Sequential,
                    );
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.show_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_confirmation_popup = false;
        }
        action
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Burn Successful!");

            ui.add_space(20.0);

            if ui.button("Back to Tokens").clicked() {
                // Pop this screen and refresh
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }
}

impl ScreenLike for BurnTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Successfully burned tokens") || message == "BurnTokens" {
                    self.status = BurnTokensStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = BurnTokensStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {
        // If you need to reload local identity data or re-check keys
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
                (
                    &self.identity_token_balance.token_alias,
                    AppAction::PopScreen,
                ),
                ("Burn", AppAction::None),
            ],
            vec![],
        );

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        egui::CentralPanel::default().show(ctx, |ui| {
            // If we are in the "Complete" status, just show success screen
            if self.status == BurnTokensStatus::Complete {
                action = self.show_success_screen(ui);
                return;
            }

            ui.heading("Burn Tokens");
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
                        // Must unlock before we can proceed
                        return;
                    }
                }

                // 1) Key selection
                ui.heading("1. Select the key to sign the Burn transaction");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    self.render_key_selection(ui);
                    ui.add_space(5.0);
                    let identity_id_string =
                        self.identity.identity.id().to_string(Encoding::Base58);
                    let identity_display = self
                        .identity
                        .alias
                        .as_deref()
                        .unwrap_or_else(|| &identity_id_string);
                    ui.label(format!("Identity: {}", identity_display));
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // 2) Amount to burn
                ui.heading("2. Amount to burn");
                ui.add_space(5.0);
                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("3. Public note (optional)");
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Public note (optional):");
                    ui.add_space(10.0);
                    let mut txt = self.public_note.clone().unwrap_or_default();
                    if ui
                        .text_edit_singleline(&mut txt)
                        .on_hover_text(
                            "A note about the transaction that can be seen by the public.",
                        )
                        .changed()
                    {
                        self.public_note = Some(txt);
                    }
                });
                ui.add_space(10.0);

                // Render text input for the public note
                ui.horizontal(|ui| {
                    ui.label("Public note (optional):");
                    ui.add_space(10.0);
                    let mut txt = self.public_note.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut txt).changed() {
                        self.public_note = Some(txt);
                    }
                });
                ui.add_space(10.0);

                // Burn button
                let button = egui::Button::new(RichText::new("Burn").color(Color32::WHITE))
                    .fill(Color32::from_rgb(255, 0, 0))
                    .corner_radius(3.0);

                if ui.add(button).clicked() {
                    self.show_confirmation_popup = true;
                }

                // If user pressed "Burn," show a popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    BurnTokensStatus::NotStarted => {
                        // no-op
                    }
                    BurnTokensStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Burning... elapsed: {} seconds", elapsed));
                    }
                    BurnTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
                    }
                    BurnTokensStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for BurnTokensScreen {
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
