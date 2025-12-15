use crate::app::AppAction;
use crate::backend_task::identity::{IdentityInputToLoad, IdentityTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::IdentityType;
use crate::model::wallet::Wallet;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    wallet_needs_unlock, try_open_wallet_no_password, WalletUnlockPopup, WalletUnlockResult,
};
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadIdentityMode {
    ByIdentityId,
    ByWallet,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WalletIdentitySearchMode {
    SpecificIndex,
    UpToIndex,
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
    identity_associated_with_wallet: bool,
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    pub identity_index_input: String,
    pub app_context: Arc<AppContext>,
    show_pop_up_info: Option<String>,
    mode: LoadIdentityMode,
    backend_message: Option<String>,
    wallet_search_mode: WalletIdentitySearchMode,
    success_message: Option<String>,
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
            keys_input: vec![],
            add_identity_status: AddIdentityStatus::NotStarted,
            testnet_loaded_nodes,
            selected_wallet,
            identity_associated_with_wallet: true,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message: None,
            identity_index_input: String::new(),
            app_context: app_context.clone(),
            show_pop_up_info: None,
            mode: LoadIdentityMode::ByIdentityId,
            backend_message: None,
            wallet_search_mode: WalletIdentitySearchMode::SpecificIndex,
            success_message: None,
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
        }

        let wallets_snapshot: Vec<(String, Arc<RwLock<Wallet>>)> = {
            let wallets_guard = self.app_context.wallets.read().unwrap();
            wallets_guard
                .values()
                .map(|wallet| {
                    let alias = wallet
                        .read()
                        .unwrap()
                        .alias
                        .clone()
                        .unwrap_or_else(|| "Unnamed Wallet".to_string());
                    (alias, wallet.clone())
                })
                .collect()
        };
        let has_wallets = !wallets_snapshot.is_empty();
        let mut should_return_early = false;

        ui.add_space(10.0);

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let checkbox_response = ui.checkbox(
                    &mut self.identity_associated_with_wallet,
                    "Try to automatically derive private keys from loaded wallet",
                );
                let response = crate::ui::helpers::info_icon_button(
                    ui,
                    "When enabled, Dash Evo Tool scans the selected unlocked wallet (or all unlocked wallets) right now to find matching keys.",
                );
                if response.clicked() {
                    self.show_pop_up_info = Some(
                        "When enabled, Dash Evo Tool scans the selected unlocked wallet (or all unlocked wallets) right now to find matching keys."
                            .to_string(),
                    );
                }

                if checkbox_response.changed() && !self.identity_associated_with_wallet {
                    self.selected_wallet = None;
                }
            });

            if self.identity_associated_with_wallet {
                if has_wallets {
                    let selected_label = self
                        .selected_wallet
                        .as_ref()
                        .and_then(|selected| {
                            wallets_snapshot.iter().find_map(|(alias, wallet)| {
                                if Arc::ptr_eq(selected, wallet) {
                                    Some(alias.clone())
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap_or_else(|| "All unlocked wallets".to_string());

                    ComboBox::from_id_salt("identity_wallet_selector")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    self.selected_wallet.is_none(),
                                    "All unlocked wallets",
                                )
                                .clicked()
                            {
                                self.selected_wallet = None;
                            }

                            for (alias, wallet) in &wallets_snapshot {
                                let is_selected = self
                                    .selected_wallet
                                    .as_ref()
                                    .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                                if ui.selectable_label(is_selected, alias).clicked() {
                                    self.selected_wallet = Some(wallet.clone());
                                }
                            }
                        });

                    ui.add_space(10.0);
                    if let Some(selected_wallet) = &self.selected_wallet {
                        let wallet_still_loaded = wallets_snapshot
                            .iter()
                            .any(|(_, wallet)| Arc::ptr_eq(wallet, selected_wallet));

                        if wallet_still_loaded {
                            // Try to open wallet without password if it doesn't use one
                            if let Err(e) = try_open_wallet_no_password(selected_wallet) {
                                self.error_message = Some(e);
                            }

                            if wallet_needs_unlock(selected_wallet) {
                                ui.colored_label(
                                    Color32::from_rgb(200, 150, 50),
                                    "Wallet is locked.",
                                );
                                if ui.button("Unlock Wallet").clicked() {
                                    self.wallet_unlock_popup.open();
                                }
                                should_return_early = true;
                            }
                        } else {
                            self.selected_wallet = None;
                            ui.colored_label(
                                Color32::RED,
                                "Selected wallet is no longer loaded. We'll search unlocked wallets instead.",
                            );
                        }
                    }
                } else {
                    ui.colored_label(
                        Color32::GRAY,
                        "No wallets are currently loaded. Import one to scan for keys.",
                    );
                }
            }
        });

        if should_return_early {
            return action;
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

                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
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
        if ui.button("+ Add key manually").clicked() {
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

    fn render_wallet_selection(&mut self, ui: &mut Ui) {
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

    fn render_by_wallet(&mut self, ui: &mut egui::Ui, wallets_len: usize) -> AppAction {
        let mut action = AppAction::None;

        if wallets_len == 0 {
            ui.colored_label(
                Color32::GRAY,
                "No wallets available. Import a wallet to search by derivation path.",
            );
            return action;
        }

        // Wallet selection
        if wallets_len > 1 {
            self.render_wallet_selection(ui);
        }

        if self.selected_wallet.is_none() {
            ui.label("Select a wallet to search for linked identities.");
            return action;
        };

        let wallet = self.selected_wallet.as_ref().unwrap();

        // Try to open wallet without password if it doesn't use one
        if let Err(e) = try_open_wallet_no_password(wallet) {
            self.error_message = Some(e);
        }

        if wallet_needs_unlock(wallet) {
            ui.add_space(10.0);
            ui.colored_label(
                Color32::from_rgb(200, 150, 50),
                "Wallet is locked. Please unlock to continue.",
            );
            ui.add_space(8.0);
            if ui.button("Unlock Wallet").clicked() {
                self.wallet_unlock_popup.open();
            }
            return action;
        }

        let mut wallet_mode_changed = false;
        ui.horizontal(|ui| {
            ui.label("Search type:");
            wallet_mode_changed |= ui
                .selectable_value(
                    &mut self.wallet_search_mode,
                    WalletIdentitySearchMode::SpecificIndex,
                    "Specific index",
                )
                .changed();
            wallet_mode_changed |= ui
                .selectable_value(
                    &mut self.wallet_search_mode,
                    WalletIdentitySearchMode::UpToIndex,
                    "All up to index",
                )
                .changed();
        });
        if wallet_mode_changed {
            self.add_identity_status = AddIdentityStatus::NotStarted;
            self.error_message = None;
            self.backend_message = None;
            self.success_message = None;
        }
        ui.add_space(6.0);

        let identity_index_label = match self.wallet_search_mode {
            WalletIdentitySearchMode::SpecificIndex => "Identity index:",
            WalletIdentitySearchMode::UpToIndex => {
                "Highest identity index to search (inclusive, max 29):"
            }
        };

        ui.horizontal(|ui| {
            ui.label(identity_index_label);
            ui.text_edit_singleline(&mut self.identity_index_input);
        });

        match self.wallet_search_mode {
            WalletIdentitySearchMode::SpecificIndex => {
                ui.label("This is the derivation index used when the identity was created.");
            }
            WalletIdentitySearchMode::UpToIndex => {
                ui.label(
                    "Searches each derivation index starting at 0 up to the provided index (inclusive).",
                );
            }
        }

        let button_label = match self.wallet_search_mode {
            WalletIdentitySearchMode::SpecificIndex => "Search For Identity",
            WalletIdentitySearchMode::UpToIndex => "Load Identities",
        };

        if ui.button(button_label).clicked() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();
            self.add_identity_status = AddIdentityStatus::WaitingForResult(now);
            self.backend_message = None;
            self.success_message = None;

            // Parse identity index input
            if let Ok(identity_index) = self.identity_index_input.trim().parse::<u32>() {
                let wallet_ref = self.selected_wallet.as_ref().unwrap().clone().into();
                action = AppAction::BackendTask(BackendTask::IdentityTask(
                    match self.wallet_search_mode {
                        WalletIdentitySearchMode::SpecificIndex => {
                            IdentityTask::SearchIdentityFromWallet(wallet_ref, identity_index)
                        }
                        WalletIdentitySearchMode::UpToIndex => {
                            IdentityTask::SearchIdentitiesUpToIndex(wallet_ref, identity_index)
                        }
                    },
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
        let selected_wallet_seed_hash = if self.identity_associated_with_wallet {
            self.selected_wallet
                .as_ref()
                .map(|wallet| wallet.read().unwrap().seed_hash())
        } else {
            None
        };

        let identity_input = IdentityInputToLoad {
            identity_id_input: self.identity_id_input.trim().to_string(),
            identity_type: self.identity_type,
            alias_input: self.alias_input.clone(),
            voting_private_key_input: self.voting_private_key_input.clone(),
            owner_private_key_input: self.owner_private_key_input.clone(),
            payout_address_private_key_input: self.payout_address_private_key_input.clone(),
            keys_input: self.keys_input.clone(),
            derive_keys_from_wallets: self.identity_associated_with_wallet,
            selected_wallet_seed_hash,
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
            let success_text = self
                .success_message
                .clone()
                .unwrap_or_else(|| "Successfully loaded identity.".to_string());
            ui.label(RichText::new(success_text));

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
                self.backend_message = None;
                self.success_message = None;
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

impl ScreenLike for AddExistingIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.add_identity_status = AddIdentityStatus::ErrorMessage(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::LoadedIdentity(_) = backend_task_success_result {
            self.success_message = Some("Successfully loaded identity.".to_string());
            self.add_identity_status = AddIdentityStatus::Complete;
            self.backend_message = None;
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

                    let mut mode_changed = false;
                    ui.horizontal(|ui| {
                        mode_changed |= ui
                            .selectable_value(
                                &mut self.mode,
                                LoadIdentityMode::ByIdentityId,
                                "By Identity",
                            )
                            .changed();
                        mode_changed |= ui
                            .selectable_value(
                                &mut self.mode,
                                LoadIdentityMode::ByWallet,
                                "By Wallet",
                            )
                            .changed();
                    });
                    ui.add_space(10.0);

                    if mode_changed {
                        self.add_identity_status = AddIdentityStatus::NotStarted;
                        self.error_message = None;
                        self.backend_message = None;
                        self.success_message = None;
                    }

                    match self.mode {
                        LoadIdentityMode::ByIdentityId => {
                            inner_action |= self.render_by_identity(ui);
                        }
                        LoadIdentityMode::ByWallet => {
                            let wallets_len = {
                                let wallets = self.app_context.wallets.read().unwrap();
                                wallets.len()
                            };
                            inner_action |= self.render_by_wallet(ui, wallets_len);
                        }
                    }

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

                            if self.backend_message.is_some() {
                                ui.label(self.backend_message.clone().unwrap().to_string());
                            }
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
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut popup =
                        InfoPopup::new("Load Identity Information", &show_pop_up_info_text);
                    if popup.show(ui).inner {
                        self.show_pop_up_info = None;
                    }
                });
        }

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open() {
            if let Some(wallet) = &self.selected_wallet {
                let result =
                    self.wallet_unlock_popup
                        .show(ctx, wallet, &self.app_context);
                if result == WalletUnlockResult::Unlocked {
                    // Wallet unlocked successfully
                }
            }
        }

        action
    }
}
