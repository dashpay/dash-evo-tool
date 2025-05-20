use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::contracts_documents::group_actions_screen::GroupActionsScreen;
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Internal states for the mint process.
#[derive(PartialEq)]
pub enum SetTokenPriceStatus {
    NotStarted,
    WaitingForResult(u64), // Use seconds or millis
    ErrorMessage(String),
    Complete,
}

/// A UI Screen for minting tokens from an existing token contract
pub struct SetTokenPriceScreen {
    pub identity_token_info: IdentityTokenInfo,
    selected_key: Option<IdentityPublicKey>,
    pub public_note: Option<String>,
    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,

    pub token_pricing_schedule: String,
    status: SetTokenPriceStatus,
    error_message: Option<String>,

    /// Basic references
    pub app_context: Arc<AppContext>,

    /// Confirmation popup
    show_confirmation_popup: bool,

    // If needed for password-based wallet unlocking:
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl SetTokenPriceScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let possible_key = identity_token_info
            .identity
            .identity
            .get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            );

        let mut error_message = None;

        let group = match identity_token_info
            .token_config
            .distribution_rules()
            .change_direct_purchase_pricing_rules()
            .authorized_to_make_change_action_takers()
        {
            AuthorizedActionTakers::NoOne => {
                error_message =
                    Some("Setting token price is not allowed on this token".to_string());
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != &identity_token_info.identity.identity.id()
                {
                    error_message = Some(
                        "You are not allowed to set token price on this token. Only the contract owner is."
                            .to_string(),
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    error_message =
                        Some("You are not allowed to set token price on this token".to_string());
                }
                None
            }
            AuthorizedActionTakers::MainGroup => {
                match identity_token_info.token_config.main_control_group() {
                    None => {
                        error_message = Some(
                            "Invalid contract: No main control group, though one should exist"
                                .to_string(),
                        );
                        None
                    }
                    Some(group_pos) => {
                        match identity_token_info
                            .data_contract
                            .contract
                            .expected_group(group_pos)
                        {
                            Ok(group) => Some((group_pos, group.clone())),
                            Err(e) => {
                                error_message = Some(format!("Invalid contract: {}", e));
                                None
                            }
                        }
                    }
                }
            }
            AuthorizedActionTakers::Group(group_pos) => {
                match identity_token_info
                    .data_contract
                    .contract
                    .expected_group(*group_pos)
                {
                    Ok(group) => Some((*group_pos, group.clone())),
                    Err(e) => {
                        error_message = Some(format!("Invalid contract: {}", e));
                        None
                    }
                }
            }
        };

        let mut is_unilateral_group_member = false;
        if group.is_some() {
            if let Some((_, group)) = group.clone() {
                let your_power = group
                    .members()
                    .get(&identity_token_info.identity.identity.id());

                if let Some(your_power) = your_power {
                    if your_power >= &group.required_power() {
                        is_unilateral_group_member = true;
                    }
                }
            }
        };

        // Attempt to get an unlocked wallet reference
        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key,
            &mut error_message,
        );

        Self {
            identity_token_info: identity_token_info.clone(),
            selected_key: possible_key.cloned(),
            public_note: None,
            group,
            is_unilateral_group_member,
            group_action_id: None,
            token_pricing_schedule: "".to_string(),
            status: SetTokenPriceStatus::NotStarted,
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
            egui::ComboBox::from_id_salt("set_price_key_selector")
                .selected_text(match &self.selected_key {
                    Some(key) => format!("Key ID: {}", key.id()),
                    None => "Select a key".to_string(),
                })
                .show_ui(ui, |ui| {
                    if self.app_context.developer_mode {
                        // Show all loaded public keys
                        for key in self
                            .identity_token_info
                            .identity
                            .identity
                            .public_keys()
                            .values()
                        {
                            let is_valid = key.purpose() == Purpose::AUTHENTICATION
                                && key.security_level() == SecurityLevel::CRITICAL;

                            let label = format!(
                                "Key ID: {} (Info: {}/{}/{})",
                                key.id(),
                                key.purpose(),
                                key.security_level(),
                                key.key_type()
                            );
                            let styled_label = if is_valid {
                                RichText::new(label.clone())
                            } else {
                                RichText::new(label.clone()).color(Color32::RED)
                            };

                            ui.selectable_value(
                                &mut self.selected_key,
                                Some(key.clone()),
                                styled_label,
                            );
                        }
                    } else {
                        // Show only "available" auth keys
                        for key_wrapper in self
                            .identity_token_info
                            .identity
                            .available_authentication_keys_with_critical_security_level()
                        {
                            let key = &key_wrapper.identity_public_key;
                            let label = format!(
                                "Key ID: {} (Info: {}/{}/{})",
                                key.id(),
                                key.purpose(),
                                key.security_level(),
                                key.key_type()
                            );
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    }
                });
        });
    }

    /// Renders a text input for the user to specify an amount to mint
    fn render_pricing_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Token pricing schedule:");
            ui.text_edit_singleline(&mut self.token_pricing_schedule);
        });
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm SetPricingSchedule")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Validate user input
                let token_pricing_schedule_opt = if self.token_pricing_schedule.trim().is_empty() {
                    None
                } else {
                    // Try to parse as a single price (u64)
                    if let Ok(single_price) = self.token_pricing_schedule.trim().parse::<u64>() {
                        Some(TokenPricingSchedule::SinglePrice(single_price))
                    } else {
                        // Try to parse as a tiered pricing schedule: "amount1:price1,amount2:price2"
                        let mut map = std::collections::BTreeMap::new();
                        let mut parse_error = None;
                        for pair in self.token_pricing_schedule.trim().split(',') {
                            let parts: Vec<_> = pair.split(':').collect();
                            if parts.len() != 2 {
                                parse_error = Some(format!(
                                    "Invalid pair '{}', expected format amount:price",
                                    pair
                                ));
                                break;
                            }
                            let amount = match parts[0].trim().parse::<u64>() {
                                Ok(a) => a,
                                Err(_) => {
                                    parse_error =
                                        Some(format!("Invalid amount '{}'", parts[0].trim()));
                                    break;
                                }
                            };
                            let price = match parts[1].trim().parse::<u64>() {
                                Ok(p) => p,
                                Err(_) => {
                                    parse_error =
                                        Some(format!("Invalid price '{}'", parts[1].trim()));
                                    break;
                                }
                            };
                            map.insert(amount, price);
                        }
                        if let Some(e) = parse_error {
                            self.error_message = Some(format!("Invalid pricing schedule: {}", e));
                            self.status = SetTokenPriceStatus::ErrorMessage(
                                "Invalid pricing schedule".into(),
                            );
                            self.show_confirmation_popup = false;
                            return;
                        }
                        if map.is_empty() {
                            self.error_message =
                                Some("Pricing schedule cannot be empty".to_string());
                            self.status = SetTokenPriceStatus::ErrorMessage(
                                "Invalid pricing schedule".into(),
                            );
                            self.show_confirmation_popup = false;
                            return;
                        }
                        Some(TokenPricingSchedule::SetPrices(map))
                    }
                };

                ui.label(format!(
                    "Are you sure you want to set the pricing schedule to \"{}\"?",
                    self.token_pricing_schedule
                ));

                ui.add_space(10.0);

                // Confirm button
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = SetTokenPriceStatus::WaitingForResult(now);

                    let group_info;
                    if self.group_action_id.is_some() {
                        group_info = self.group.as_ref().map(|(pos, _)| {
                            GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                                GroupStateTransitionInfo {
                                    group_contract_position: *pos,
                                    action_id: self.group_action_id.unwrap(),
                                    action_is_proposer: false,
                                },
                            )
                        });
                    } else {
                        group_info = self.group.as_ref().map(|(pos, _)| {
                            GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
                        });
                    }

                    // Dispatch the actual backend mint action
                    action = AppAction::BackendTask(BackendTask::TokenTask(
                        TokenTask::SetDirectPurchasePrice {
                            identity: self.identity_token_info.identity.clone(),
                            data_contract: self.identity_token_info.data_contract.contract.clone(),
                            token_position: self.identity_token_info.token_position,
                            signing_key: self.selected_key.clone().expect("Expected a key"),
                            public_note: self.public_note.clone(),
                            token_pricing_schedule: token_pricing_schedule_opt,
                            group_info,
                        },
                    ));
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
            if self.group_action_id.is_some() {
                // This is already initiated by the group, we are just signing it
                ui.heading("Group SetPrice Signing Successful.");
            } else {
                if !self.is_unilateral_group_member {
                    ui.heading("Group SetPrice Initiated.");
                } else {
                    ui.heading("SetPrice Successful.");
                }
            }

            ui.add_space(20.0);

            if self.group_action_id.is_some() {
                if ui.button("Back to Group Actions").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenMyTokenBalances,
                    );
                }
            } else {
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }

                if !self.is_unilateral_group_member {
                    if ui.button("Go to Group Actions").clicked() {
                        action = AppAction::PopThenAddScreenToMainScreen(
                            RootScreenType::RootScreenDocumentQuery,
                            Screen::GroupActionsScreen(GroupActionsScreen::new(
                                &self.app_context.clone(),
                            )),
                        );
                    }
                }
            }
        });
        action
    }
}

impl ScreenLike for SetTokenPriceScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Successfully set token pricing schedule")
                    || message == "SetDirectPurchasePrice"
                {
                    self.status = SetTokenPriceStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = SetTokenPriceStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {
        // If you need to reload local identity data or re-check keys:
        if let Ok(all_identities) = self.app_context.load_local_qualified_identities() {
            if let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity_token_info.identity.identity.id())
            {
                self.identity_token_info.identity = updated_identity;
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action;

        // Build a top panel
        if self.group_action_id.is_some() {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Contracts", AppAction::GoToMainScreen),
                    ("Group Actions", AppAction::PopScreen),
                    ("SetPrice", AppAction::None),
                ],
                vec![],
            );
        } else {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Tokens", AppAction::GoToMainScreen),
                    (&self.identity_token_info.token_alias, AppAction::PopScreen),
                    ("SetPrice", AppAction::None),
                ],
                vec![],
            );
        }

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
            if self.status == SetTokenPriceStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Set Token Pricing Schedule");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode {
                !self.identity_token_info.identity.identity.public_keys().is_empty()
            } else {
                !self.identity_token_info.identity.available_authentication_keys_with_critical_security_level().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    Color32::DARK_RED,
                    format!(
                        "No authentication keys found for this {} identity.",
                        self.identity_token_info.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                // Show "Add key" or "Check keys" option
                let first_key = self.identity_token_info.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([
                        SecurityLevel::CRITICAL,
                    ]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity_token_info.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity_token_info.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Possibly handle locked wallet scenario (similar to TransferTokens)
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        // Must unlock before we can proceed
                        return;
                    }
                }

                // 1) Key selection
                ui.heading("1. Select the key to sign the SetPrice transaction");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    self.render_key_selection(ui);
                    ui.add_space(5.0);
                    let identity_id_string =
                        self.identity_token_info.identity.identity.id().to_string(Encoding::Base58);
                    let identity_display = self.identity_token_info.identity
                        .alias
                        .as_deref()
                        .unwrap_or_else(|| &identity_id_string);
                    ui.label(format!("Identity: {}", identity_display));
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // 2) Pricing schedule
                ui.heading("2. Pricing schedule");
                ui.add_space(5.0);
                                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group SetPrice so you are not allowed to choose the pricing schedule.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!(
                        "Schedule: {}",
                        self.token_pricing_schedule
                    ));
                } else {
                    self.render_pricing_input(ui);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("3. Public note (optional)");
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group SetPrice so you are not allowed to put a note.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!(
                        "Note: {}",
                        self.public_note.clone().unwrap_or("None".to_string())
                    ));
                } else {
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
                            self.public_note = if txt.len() > 0 {
                                Some(txt)
                            } else {
                                None
                            };
                        }
                    });
                }

                let set_price_text = if let Some((_, group)) = self.group.as_ref() {
                    let your_power = group.members().get(&self.identity_token_info.identity.identity.id());
                    if your_power.is_none() {
                        self.error_message = Some("Only group members can set price on this token".to_string());
                    }
                    ui.heading("This is a group action, it is not immediate.");
                    ui.label(format!("Members are : \n{}", group.members().iter().map(|(member, power)| {
                        if member == &self.identity_token_info.identity.identity.id() {
                            format!("{} (You) with power {}", member, power)
                        } else {
                            format!("{} with power {}", member, power)
                        }
                    }).collect::<Vec<_>>().join(", \n")));
                    ui.add_space(10.0);
                    if let Some(your_power) = your_power {
                        if *your_power >= group.required_power() {
                            ui.label(format!("Even though this is a group action, you are able to unilaterally approve it because your power ({}) in the group exceeds the required amount : {}", *your_power,  group.required_power()));
                            "Set Price"
                        } else {
                            ui.label(format!("You will need at least {} voting power for this action to go through. Contact other group members to let them know to authorize this action after you have initiated it.", group.required_power()));
                            "Initiate Group SetPrice"
                        }
                    } else {
                        "Test SetPrice (It should fail)"
                    }
                } else {
                    "Set Price"
                };

                // Set price button
                let button = egui::Button::new(RichText::new(set_price_text).color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .corner_radius(3.0);

                if ui.add(button).clicked() {
                    self.show_confirmation_popup = true;
                }

                // If the user pressed "Set Price," show a popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    SetTokenPriceStatus::NotStarted => {
                        // no-op
                    }
                    SetTokenPriceStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Setting price... elapsed: {} seconds", elapsed));
                    }
                    SetTokenPriceStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
                    }
                    SetTokenPriceStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for SetTokenPriceScreen {
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
