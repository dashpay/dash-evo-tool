use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::{IdentityInputToLoad, IdentityTask};
use crate::context::AppContext;
use crate::model::qualified_identity::IdentityType;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::{MessageType, ScreenLike};
use bip39::rand::{prelude::IteratorRandom, thread_rng};
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::Context;
use egui::{Color32, ComboBox, RichText, Ui};
use serde::Deserialize;
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
struct MasternodeInfo {
    #[serde(rename = "pro-tx-hash")]
    pro_tx_hash: String,
    owner: KeyInfo,
    voter: KeyInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct HPMasternodeInfo {
    #[serde(rename = "protx-tx-hash")]
    protx_tx_hash: String,
    owner: KeyInfo,
    voter: KeyInfo,
    payout: KeyInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct KeyInfo {
    #[serde(rename = "private_key")]
    private_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TestnetNodes {
    masternodes: std::collections::HashMap<String, MasternodeInfo>,
    hp_masternodes: std::collections::HashMap<String, HPMasternodeInfo>,
}

fn load_testnet_nodes_from_yml(file_path: &str) -> Option<TestnetNodes> {
    let file_content = fs::read_to_string(file_path).ok()?;
    serde_yaml::from_str(&file_content).expect("expected proper yaml")
}

#[derive(PartialEq)]
pub enum AddIdentityStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct AddExistingIdentityScreen {
    identity_id_input: String,
    pub identity_type: IdentityType,
    alias_input: String,
    voting_private_key_input: String,
    owner_private_key_input: String,
    payout_address_private_key_input: String,
    keys_input: Vec<String>,
    add_identity_status: AddIdentityStatus,
    testnet_loaded_nodes: Option<TestnetNodes>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    show_password: bool,
    wallet_password: String,
    error_message: Option<String>,
    pub identity_index_input: String,
    pub app_context: Arc<AppContext>,
    show_pop_up_info: Option<String>,
}

impl AddExistingIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = app_context.wallets.read().unwrap().values().next().cloned();
        let testnet_loaded_nodes = if app_context.network == Network::Testnet {
            load_testnet_nodes_from_yml(".testnet_nodes.yml")
        } else {
            None
        };
        Self {
            identity_id_input: String::new(),
            identity_type: IdentityType::User,
            alias_input: String::new(),
            voting_private_key_input: String::new(),
            owner_private_key_input: String::new(),
            payout_address_private_key_input: String::new(),
            keys_input: vec![String::new(), String::new(), String::new()],
            add_identity_status: AddIdentityStatus::NotStarted,
            testnet_loaded_nodes,
            selected_wallet,
            show_password: false,
            wallet_password: "".to_string(),
            error_message: None,
            identity_index_input: String::new(),
            app_context: app_context.clone(),
            show_pop_up_info: None,
        }
    }

    fn render_by_identity(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        if self.app_context.network == Network::Testnet && self.testnet_loaded_nodes.is_some() {
            if ui.button("Fill Random HPMN").clicked() {
                self.fill_random_hpmn();
            }
            if ui.button("Fill Random Masternode").clicked() {
                self.fill_random_masternode();
            }
            ui.add_space(10.0);
        }

        egui::Grid::new("add_existing_identity_grid")
            .num_columns(2)
            .spacing([10.0, 10.0])
            .striped(false)
            .show(ui, |ui| {
                ui.label("Identity ID / ProTxHash (Hex or Base58):");
                ui.text_edit_singleline(&mut self.identity_id_input);
                ui.label("");
                ui.end_row();

                ui.label("Identity Type:");
                egui::ComboBox::from_id_salt("identity_type_selector")
                    .selected_text(format!("{:?}", self.identity_type))
                    // .width(350.0) // This sets the entire row's width
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.identity_type, IdentityType::User, "User");
                        ui.selectable_value(
                            &mut self.identity_type,
                            IdentityType::Masternode,
                            "Masternode",
                        );
                        ui.selectable_value(
                            &mut self.identity_type,
                            IdentityType::Evonode,
                            "Evonode",
                        );
                    });
                ui.label("");
                ui.end_row();

                // Input for Alias
                ui.horizontal(|ui| {
                    ui.label("Alias (optional):");
                    let response = crate::ui::helpers::info_icon_button(ui, "Alias is optional. It is only used to help identify the identity in Dash Evo Tool. It isn't saved to Dash Platform.");
                    if response.clicked() {
                        self.show_pop_up_info = Some("Alias is optional. It is only used to help identify the identity in Dash Evo Tool. It isn't saved to Dash Platform.".to_string());
                    }
                });
                ui.text_edit_singleline(&mut self.alias_input);
                ui.label("");
                ui.end_row();

                // Render the keys input based on identity type
                match self.identity_type {
                    IdentityType::Masternode | IdentityType::Evonode => {
                        // Store the voting and owner private key references before borrowing `self` mutably
                        let voting_private_key_input = &mut self.voting_private_key_input;
                        let owner_private_key_input = &mut self.owner_private_key_input;
                        let payout_address_private_key_input =
                            &mut self.payout_address_private_key_input;

                        ui.label("Voting Private Key:");
                        ui.text_edit_singleline(voting_private_key_input);
                        ui.end_row();

                        ui.label("Owner Private Key:");
                        ui.text_edit_singleline(owner_private_key_input);
                        ui.end_row();

                        ui.label("Payout Address Private Key:");
                        ui.text_edit_singleline(payout_address_private_key_input);
                        ui.end_row();
                    }
                    IdentityType::User => {
                        // A temporary vector to store indices of keys to be removed
                        let mut keys_to_remove = vec![];

                        for (i, key) in self.keys_input.iter_mut().enumerate() {
                            // First column: the label & info icon, combined horizontally
                            ui.horizontal(|ui| {
                                ui.label(format!("Private Key {} (Hex or WIF):", i + 1));

                                let response = crate::ui::helpers::info_icon_button(ui, "You don't need to add all or even any private keys here. \
                                                    Private keys can be added later. However, without private keys, \
                                                    you won't be able to sign any transactions.");

                                if response.clicked() {
                                    self.show_pop_up_info = Some(
                                        "You don't need to add all or even any private keys here. \
                                         Private keys can be added later. However, without private keys, \
                                         you won't be able to sign any transactions."
                                            .to_string(),
                                    );
                                }
                            });

                            // Second column: the text field
                            ui.text_edit_singleline(key);

                            // Third column: the remove button
                            if ui.button("-").clicked() {
                                keys_to_remove.push(i);
                            }

                            ui.end_row();
                        }

                        // Remove the keys after the loop to avoid borrowing conflicts
                        for i in keys_to_remove.iter().rev() {
                            self.keys_input.remove(*i);
                        }
                    }
                }
            });
        ui.add_space(10.0);

        // Add button to add more keys
        if ui.button("+ Add Key").clicked() {
            self.keys_input.push(String::new());
        }
        ui.add_space(10.0);

        // Load Identity button
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);
        let button = egui::Button::new(RichText::new("Load Identity").color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            // Set the status to waiting and capture the current time
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();
            self.add_identity_status = AddIdentityStatus::WaitingForResult(now);
            action = self.load_identity_clicked();
        }
        action
    }

    fn _render_wallet_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if self.app_context.has_wallet.load(Ordering::Relaxed) {
                let wallets = &self.app_context.wallets.read().unwrap();
                let wallet_aliases: Vec<String> = wallets
                    .values()
                    .map(|wallet| {
                        wallet
                            .read()
                            .unwrap()
                            .alias
                            .clone()
                            .unwrap_or_else(|| "Unnamed Wallet".to_string())
                    })
                    .collect();

                let selected_wallet_alias = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|wallet| wallet.read().ok()?.alias.clone())
                    .unwrap_or_else(|| "Select".to_string());

                // Display the ComboBox for wallet selection
                ComboBox::from_label("")
                    .selected_text(selected_wallet_alias.clone())
                    .show_ui(ui, |ui| {
                        for (idx, wallet) in wallets.values().enumerate() {
                            let wallet_alias = wallet_aliases[idx].clone();

                            let is_selected = self
                                .selected_wallet
                                .as_ref()
                                .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                            if ui
                                .selectable_label(is_selected, wallet_alias.clone())
                                .clicked()
                            {
                                // Update the selected wallet
                                self.selected_wallet = Some(wallet.clone());
                            }
                        }
                    });

                ui.add_space(20.0);
            } else {
                ui.label("No wallets available.");
            }
        });
    }

    fn _render_from_wallet(&mut self, ui: &mut egui::Ui, wallets_len: usize) -> AppAction {
        let mut action = AppAction::None;

        // Wallet selection
        if wallets_len > 1 {
            self._render_wallet_selection(ui);
        }

        if self.selected_wallet.is_none() {
            return action;
        };

        let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

        if needed_unlock && !just_unlocked {
            return action;
        }

        // Identity index input
        ui.horizontal(|ui| {
            ui.label("Identity Index:");
            ui.text_edit_singleline(&mut self.identity_index_input);
        });

        if ui.button("Search For Identity").clicked() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();
            self.add_identity_status = AddIdentityStatus::WaitingForResult(now);

            // Parse identity index input
            if let Ok(identity_index) = self.identity_index_input.trim().parse::<u32>() {
                action = AppAction::BackendTask(BackendTask::IdentityTask(
                    IdentityTask::SearchIdentityFromWallet(
                        self.selected_wallet.as_ref().unwrap().clone().into(),
                        identity_index,
                    ),
                ));
            } else {
                // Handle invalid index input (optional)
                self.add_identity_status =
                    AddIdentityStatus::ErrorMessage("Invalid identity index".to_string());
            }
        }
        action
    }

    fn load_identity_clicked(&mut self) -> AppAction {
        let identity_input = IdentityInputToLoad {
            identity_id_input: self.identity_id_input.trim().to_string(),
            identity_type: self.identity_type,
            alias_input: self.alias_input.clone(),
            voting_private_key_input: self.voting_private_key_input.clone(),
            owner_private_key_input: self.owner_private_key_input.clone(),
            payout_address_private_key_input: self.payout_address_private_key_input.clone(),
            keys_input: self.keys_input.clone(),
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::LoadIdentity(
            identity_input,
        )))
    }
    fn fill_random_hpmn(&mut self) {
        if let Some((name, hpmn)) = self
            .testnet_loaded_nodes
            .as_ref()
            .unwrap()
            .hp_masternodes
            .iter()
            .choose(&mut thread_rng())
        {
            self.identity_id_input = hpmn.protx_tx_hash.clone();
            self.identity_type = IdentityType::Evonode;
            self.alias_input = name.clone();
            self.voting_private_key_input = hpmn.voter.private_key.clone();
            self.owner_private_key_input = hpmn.owner.private_key.clone();
            self.payout_address_private_key_input = hpmn.payout.private_key.clone();
        }
    }

    fn fill_random_masternode(&mut self) {
        if let Some((name, masternode)) = self
            .testnet_loaded_nodes
            .as_ref()
            .unwrap()
            .masternodes
            .iter()
            .choose(&mut thread_rng())
        {
            self.identity_id_input = masternode.pro_tx_hash.clone();
            self.identity_type = IdentityType::Masternode;
            self.alias_input = name.clone();
            self.voting_private_key_input = masternode.voter.private_key.clone();
            self.owner_private_key_input = masternode.owner.private_key.clone();
        }
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully loaded identity.");

            ui.add_space(20.0);

            if ui.button("Load Another").clicked() {
                self.identity_id_input.clear();
                self.alias_input.clear();
                self.voting_private_key_input.clear();
                self.owner_private_key_input.clear();
                self.payout_address_private_key_input.clear();
                self.keys_input = vec![String::new(), String::new(), String::new()];
                self.identity_index_input.clear();
                self.error_message = None;
                self.show_pop_up_info = None;
                self.add_identity_status = AddIdentityStatus::NotStarted;
            }
            ui.add_space(5.0);

            if ui.button("Back to Identities Screen").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            ui.add_space(5.0);
        });

        action
    }
}

impl ScreenWithWalletUnlock for AddExistingIdentityScreen {
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

impl ScreenLike for AddExistingIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "Successfully loaded identity" {
                    self.add_identity_status = AddIdentityStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.add_identity_status = AddIdentityStatus::ErrorMessage(message.to_string());
            }
        }
    }

    fn pop_on_success(&mut self) {
        self.add_identity_status = AddIdentityStatus::Complete;
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Load Identity", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.heading("Load Existing Identity");
                    ui.add_space(10.0);

                    if self.add_identity_status == AddIdentityStatus::Complete {
                        inner_action |= self.show_success(ui);
                        return;
                    }

                    inner_action |= self.render_by_identity(ui);

                    ui.add_space(10.0);

                    match &self.add_identity_status {
                        AddIdentityStatus::NotStarted => {
                            // Do nothing
                        }
                        AddIdentityStatus::WaitingForResult(start_time) => {
                            let now = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .expect("Time went backwards")
                                .as_secs();
                            let elapsed_seconds = now - start_time;

                            let display_time = if elapsed_seconds < 60 {
                                format!(
                                    "{} second{}",
                                    elapsed_seconds,
                                    if elapsed_seconds == 1 { "" } else { "s" }
                                )
                            } else {
                                let minutes = elapsed_seconds / 60;
                                let seconds = elapsed_seconds % 60;
                                format!(
                                    "{} minute{} and {} second{}",
                                    minutes,
                                    if minutes == 1 { "" } else { "s" },
                                    seconds,
                                    if seconds == 1 { "" } else { "s" }
                                )
                            };

                            ui.label(format!("Loading... Time taken so far: {}", display_time));
                        }
                        AddIdentityStatus::ErrorMessage(msg) => {
                            ui.colored_label(egui::Color32::DARK_RED, format!("Error: {}", msg));
                        }
                        AddIdentityStatus::Complete => {
                            // handled above
                        }
                    }
                });

            inner_action
        });

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::Window::new("Load Identity Information")
                .collapsible(false) // Prevent collapsing
                .resizable(false) // Prevent resizing
                .show(ctx, |ui| {
                    ui.label(show_pop_up_info_text);

                    // Add a close button to dismiss the popup
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.show_pop_up_info = None
                    }
                });
        }

        action
    }
}
