use crate::app::AppAction;
use crate::backend_task::identity::{IdentityInputToLoad, IdentityTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::IdentityType;
use crate::model::wallet::Wallet;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use bip39::rand::{prelude::IteratorRandom, thread_rng};
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::Context;
use egui::{Color32, ComboBox, RichText, Ui};
use serde::Deserialize;
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

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

fn load_testnet_nodes_from_yml(file_path: &str) -> Result<Option<TestnetNodes>, String> {
    let file_content = match fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    serde_yaml_ng::from_str::<TestnetNodes>(&file_content)
        .map(Some)
        .map_err(|e| {
            format!(
                "Failed to parse YAML file '{}': {}. Please check the file format.",
                file_path, e
            )
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadIdentityMode {
    IdentityId,
    Wallet,
    DpnsName,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WalletIdentitySearchMode {
    SpecificIndex,
    UpToIndex,
}

#[derive(PartialEq)]
pub enum AddIdentityStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

pub struct AddExistingIdentityScreen {
    identity_id_input: String,
    pub identity_type: IdentityType,
    alias_input: String,
    voting_private_key_input: PasswordInput,
    owner_private_key_input: PasswordInput,
    payout_address_private_key_input: PasswordInput,
    keys_input: Vec<PasswordInput>,
    add_identity_status: AddIdentityStatus,
    testnet_loaded_nodes: Option<TestnetNodes>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    identity_associated_with_wallet: bool,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    pub identity_index_input: String,
    pub app_context: Arc<AppContext>,
    show_pop_up_info: Option<String>,
    mode: LoadIdentityMode,
    wallet_search_mode: WalletIdentitySearchMode,
    success_message: Option<String>,
    dpns_name_input: String,
    /// Whether to show advanced options
    show_advanced_options: bool,
    refresh_banner: Option<BannerHandle>,
}

impl AddExistingIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = app_context.wallets.read().unwrap().values().next().cloned();
        let (testnet_loaded_nodes, init_error) = if app_context.network == Network::Testnet {
            match load_testnet_nodes_from_yml(".testnet_nodes.yml") {
                Ok(nodes) => (nodes, None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (None, None)
        };
        if let Some(err) = init_error {
            MessageBanner::set_global(app_context.egui_ctx(), &err, MessageType::Error);
        }
        Self {
            identity_id_input: String::new(),
            identity_type: IdentityType::User,
            alias_input: String::new(),
            voting_private_key_input: PasswordInput::new()
                .with_hint_text("Private key (WIF or hex)")
                .with_monospace(),
            owner_private_key_input: PasswordInput::new()
                .with_hint_text("Private key (WIF or hex)")
                .with_monospace(),
            payout_address_private_key_input: PasswordInput::new()
                .with_hint_text("Private key (WIF or hex)")
                .with_monospace(),
            keys_input: vec![],
            add_identity_status: AddIdentityStatus::NotStarted,
            testnet_loaded_nodes,
            selected_wallet,
            identity_associated_with_wallet: true,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            identity_index_input: String::new(),
            app_context: app_context.clone(),
            show_pop_up_info: None,
            mode: LoadIdentityMode::IdentityId,
            wallet_search_mode: WalletIdentitySearchMode::SpecificIndex,
            success_message: None,
            dpns_name_input: String::new(),
            show_advanced_options: false,
            refresh_banner: None,
        }
    }

    fn render_by_identity(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Advanced: Testnet quick-fill buttons
        if self.show_advanced_options
            && self.app_context.network == Network::Testnet
            && self.testnet_loaded_nodes.is_some()
        {
            ui.horizontal(|ui| {
                if ui.button("Fill Random HPMN").clicked() {
                    self.fill_random_hpmn();
                }
                if ui.button("Fill Random Masternode").clicked() {
                    self.fill_random_masternode();
                }
            });
            ui.add_space(10.0);
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

        // In simple mode, always try to derive from wallets
        if !self.show_advanced_options {
            self.identity_associated_with_wallet = true;
            self.identity_type = IdentityType::User;
        }

        // Advanced: Wallet derivation checkbox and selection
        if self.show_advanced_options {
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
                                    self.wallet_open_attempted = false;
                                }

                                for (alias, wallet) in &wallets_snapshot {
                                    let is_selected = self
                                        .selected_wallet
                                        .as_ref()
                                        .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                                    if ui.selectable_label(is_selected, alias).clicked() {
                                        self.selected_wallet = Some(wallet.clone());
                                        self.wallet_open_attempted = false;
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
                                if !self.wallet_open_attempted {
                                    if let Err(e) = try_open_wallet_no_password(selected_wallet) {
                                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
                                    }
                                    self.wallet_open_attempted = true;
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
                                self.wallet_open_attempted = false;
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
            ui.add_space(10.0);
        }

        if should_return_early {
            return action;
        }

        // Main form
        egui::Grid::new("add_existing_identity_grid")
            .num_columns(2)
            .spacing([10.0, 10.0])
            .striped(false)
            .show(ui, |ui| {
                // Identity ID input - always shown
                ui.horizontal(|ui| {
                    ui.label("Identity ID:");
                    if self.show_advanced_options {
                        let response = crate::ui::helpers::info_icon_button(
                            ui,
                            "Enter the Identity ID in Hex or Base58 format. For masternodes/evonodes, use the ProTxHash.",
                        );
                        if response.clicked() {
                            self.show_pop_up_info = Some(
                                "Enter the Identity ID in Hex or Base58 format. For masternodes/evonodes, use the ProTxHash."
                                    .to_string(),
                            );
                        }
                    }
                });
                ui.text_edit_singleline(&mut self.identity_id_input);
                ui.end_row();

                // Advanced: Identity Type selector
                if self.show_advanced_options {
                    ui.label("Identity Type:");
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        egui::ComboBox::from_id_salt("identity_type_selector")
                            .selected_text(format!("{:?}", self.identity_type))
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
                    ui.end_row();
                }

                // Alias input - always shown
                ui.horizontal(|ui| {
                    ui.label("Alias (optional):");
                    let response = crate::ui::helpers::info_icon_button(
                        ui,
                        "Alias is optional. It is only used to help identify the identity in Dash Evo Tool. It isn't saved to Dash Platform.",
                    );
                    if response.clicked() {
                        self.show_pop_up_info = Some(
                            "Alias is optional. It is only used to help identify the identity in Dash Evo Tool. It isn't saved to Dash Platform."
                                .to_string(),
                        );
                    }
                });
                ui.text_edit_singleline(&mut self.alias_input);
                ui.end_row();

                // Advanced: Masternode/Evonode key inputs
                if self.show_advanced_options {
                    match self.identity_type {
                        IdentityType::Masternode | IdentityType::Evonode => {
                            ui.label("Voting Private Key:");
                            self.voting_private_key_input.show(ui);
                            ui.end_row();

                            ui.label("Owner Private Key:");
                            self.owner_private_key_input.show(ui);
                            ui.end_row();

                            ui.label("Payout Address Private Key:");
                            self.payout_address_private_key_input.show(ui);
                            ui.end_row();
                        }
                        IdentityType::User => {
                            // Manual key inputs for User type
                            let mut keys_to_remove = vec![];

                            for (i, key_input) in self.keys_input.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("Private Key {} (Hex or WIF):", i + 1));

                                    let response = crate::ui::helpers::info_icon_button(
                                        ui,
                                        "You don't need to add all or even any private keys here. Private keys can be added later. However, without private keys, you won't be able to sign any transactions.",
                                    );

                                    if response.clicked() {
                                        self.show_pop_up_info = Some(
                                            "You don't need to add all or even any private keys here. Private keys can be added later. However, without private keys, you won't be able to sign any transactions."
                                                .to_string(),
                                        );
                                    }
                                });

                                key_input.show(ui);

                                if ui.button("-").clicked() {
                                    keys_to_remove.push(i);
                                }

                                ui.end_row();
                            }

                            for i in keys_to_remove.iter().rev() {
                                self.keys_input.remove(*i);
                            }
                        }
                    }
                }
            });

        // Advanced: Add key manually button
        if self.show_advanced_options && self.identity_type == IdentityType::User {
            ui.add_space(10.0);
            if ui.button("+ Add key manually").clicked() {
                self.keys_input.push(
                    PasswordInput::new()
                        .with_hint_text("Private key (WIF or hex)")
                        .with_monospace(),
                );
            }
        }

        ui.add_space(15.0);

        // Validate identity ID
        let identity_id_trimmed = self.identity_id_input.trim().to_string();
        let is_valid_id = !identity_id_trimmed.is_empty()
            && Identifier::from_string_try_encodings(
                &identity_id_trimmed,
                &[Encoding::Base58, Encoding::Hex],
            )
            .is_ok();

        // Load Identity button - styled like Create Identity
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);

        let button = egui::Button::new(RichText::new("Load Identity").color(Color32::WHITE))
            .fill(if is_valid_id {
                DashColors::DASH_BLUE
            } else {
                DashColors::BUTTON_DISABLED
            })
            .frame(true)
            .corner_radius(3.0);

        if ui.add_enabled(is_valid_id, button).clicked() {
            self.add_identity_status = AddIdentityStatus::WaitingForResult;
            let handle = MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Loading identity...",
                MessageType::Info,
            );
            handle.with_elapsed();
            self.refresh_banner = Some(handle);
            action = self.load_identity_clicked();
        }

        // Show helpful message based on input state
        if identity_id_trimmed.is_empty() {
            ui.add_space(5.0);
            ui.label(RichText::new("Enter an Identity ID to continue.").color(Color32::GRAY));
        } else if !is_valid_id {
            ui.add_space(5.0);
            ui.label(
                RichText::new(
                    "Invalid Identity ID format. Must be valid Base58 or Hex (64 characters).",
                )
                .color(DashColors::VALIDATION_WARNING),
            );
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
                                self.wallet_open_attempted = false;
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

        // In simple mode, default to searching all indices up to 5
        if !self.show_advanced_options {
            self.wallet_search_mode = WalletIdentitySearchMode::UpToIndex;
            if self.identity_index_input.is_empty() {
                self.identity_index_input = "5".to_string();
            }
        }

        // Wallet selection
        if wallets_len > 1 {
            ui.label("Select which wallet to search for identities:");
            ui.add_space(5.0);
            self.render_wallet_selection(ui);
            ui.add_space(10.0);
        }

        if self.selected_wallet.is_none() {
            ui.label("Select a wallet to search for linked identities.");
            return action;
        };

        let wallet = self.selected_wallet.as_ref().unwrap();

        // Try to open wallet without password if it doesn't use one
        if !self.wallet_open_attempted {
            if let Err(e) = try_open_wallet_no_password(wallet) {
                MessageBanner::set_global(self.app_context.egui_ctx(), &e, MessageType::Error);
            }
            self.wallet_open_attempted = true;
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

        // Advanced: Search type selector
        if self.show_advanced_options {
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
        } else {
            // Simple mode: just show explanation and use default
            ui.label("This will search your wallet for any identities created with it.");
            ui.add_space(5.0);
        }

        ui.add_space(10.0);

        let button_label = match self.wallet_search_mode {
            WalletIdentitySearchMode::SpecificIndex => "Search For Identity",
            WalletIdentitySearchMode::UpToIndex => "Search Wallet for Identities",
        };

        // Styled button consistent with other modes
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);

        let button = egui::Button::new(RichText::new(button_label).color(Color32::WHITE))
            .fill(DashColors::DASH_BLUE)
            .frame(true)
            .corner_radius(3.0);

        if ui.add(button).clicked() {
            self.add_identity_status = AddIdentityStatus::WaitingForResult;
            self.success_message = None;
            let handle = MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Loading identity...",
                MessageType::Info,
            );
            handle.with_elapsed();
            self.refresh_banner = Some(handle);

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
                // Handle invalid index input
                self.add_identity_status = AddIdentityStatus::NotStarted;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Invalid identity index",
                    MessageType::Error,
                );
            }
        }
        action
    }

    fn render_by_dpns_name(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.label("Look up an identity by its registered DPNS username.");
        ui.add_space(15.0);

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

        // In simple mode, always try to derive from wallets
        if !self.show_advanced_options {
            self.identity_associated_with_wallet = true;
        }

        // Advanced: Wallet derivation options
        if self.show_advanced_options {
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut self.identity_associated_with_wallet,
                    "Try to automatically derive private keys from loaded wallet",
                );
                let response = crate::ui::helpers::info_icon_button(
                    ui,
                    "When enabled, Dash Evo Tool scans the selected unlocked wallet (or all unlocked wallets) to find matching keys.",
                );
                if response.clicked() {
                    self.show_pop_up_info = Some(
                        "When enabled, Dash Evo Tool scans the selected unlocked wallet (or all unlocked wallets) to find matching keys."
                            .to_string(),
                    );
                }
            });

            if self.identity_associated_with_wallet && has_wallets {
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

                ComboBox::from_id_salt("dpns_wallet_selector")
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
            }
            ui.add_space(10.0);
        }

        egui::Grid::new("dpns_search_grid")
            .num_columns(2)
            .spacing([10.0, 10.0])
            .show(ui, |ui| {
                ui.label("Username:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.dpns_name_input);
                    ui.label(".dash");
                });
                ui.end_row();
            });

        ui.add_space(5.0);
        ui.label(
            RichText::new("Example: Enter \"alice\" to look up \"alice.dash\"")
                .color(Color32::GRAY),
        );
        ui.add_space(15.0);

        // Search button - styled consistently
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);

        let name_trimmed = self.dpns_name_input.trim();
        let is_valid = !name_trimmed.is_empty() && name_trimmed.len() >= 3;

        let button = egui::Button::new(RichText::new("Search by Username").color(Color32::WHITE))
            .fill(if is_valid {
                DashColors::DASH_BLUE
            } else {
                DashColors::BUTTON_DISABLED
            })
            .frame(true)
            .corner_radius(3.0);

        if ui.add_enabled(is_valid, button).clicked() {
            self.add_identity_status = AddIdentityStatus::WaitingForResult;
            self.success_message = None;
            let handle = MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Loading identity...",
                MessageType::Info,
            );
            handle.with_elapsed();
            self.refresh_banner = Some(handle);

            // Get the selected wallet seed hash for key derivation
            let selected_wallet_seed_hash = if self.identity_associated_with_wallet {
                self.selected_wallet
                    .as_ref()
                    .map(|wallet| wallet.read().unwrap().seed_hash())
            } else {
                None
            };

            action = AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::SearchIdentityByDpnsName(
                    name_trimmed.to_string(),
                    selected_wallet_seed_hash,
                ),
            ));
        }

        if !is_valid && !name_trimmed.is_empty() {
            ui.add_space(5.0);
            ui.label(RichText::new("Username must be at least 3 characters.").color(Color32::GRAY));
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
            voting_private_key_input: self.voting_private_key_input.take_secret(),
            owner_private_key_input: self.owner_private_key_input.take_secret(),
            payout_address_private_key_input: self.payout_address_private_key_input.take_secret(),
            keys_input: self
                .keys_input
                .iter_mut()
                .map(|k| k.take_secret())
                .collect(),
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
            .and_then(|nodes| nodes.hp_masternodes.iter().choose(&mut thread_rng()))
        {
            self.identity_id_input = hpmn.protx_tx_hash.clone();
            self.identity_type = IdentityType::Evonode;
            self.alias_input = name.clone();
            self.voting_private_key_input
                .set_text(hpmn.voter.private_key.clone());
            self.owner_private_key_input
                .set_text(hpmn.owner.private_key.clone());
            self.payout_address_private_key_input
                .set_text(hpmn.payout.private_key.clone());
        }
    }

    fn fill_random_masternode(&mut self) {
        if let Some((name, masternode)) = self
            .testnet_loaded_nodes
            .as_ref()
            .and_then(|nodes| nodes.masternodes.iter().choose(&mut thread_rng()))
        {
            self.identity_id_input = masternode.pro_tx_hash.clone();
            self.identity_type = IdentityType::Masternode;
            self.alias_input = name.clone();
            self.voting_private_key_input
                .set_text(masternode.voter.private_key.clone());
            self.owner_private_key_input
                .set_text(masternode.owner.private_key.clone());
        }
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let success_text = self
            .success_message
            .clone()
            .unwrap_or_else(|| "Successfully loaded identity.".to_string());

        let action = crate::ui::helpers::show_success_screen(
            ui,
            success_text,
            vec![
                (
                    "Load Another".to_string(),
                    AppAction::Custom("load_another".to_string()),
                ),
                (
                    "Back to Identities Screen".to_string(),
                    AppAction::PopScreenAndRefresh,
                ),
            ],
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "load_another"
        {
            self.identity_id_input.clear();
            self.alias_input.clear();
            self.voting_private_key_input.clear();
            self.owner_private_key_input.clear();
            self.payout_address_private_key_input.clear();
            self.keys_input = vec![
                PasswordInput::new()
                    .with_hint_text("Private key (WIF or hex)")
                    .with_monospace(),
                PasswordInput::new()
                    .with_hint_text("Private key (WIF or hex)")
                    .with_monospace(),
                PasswordInput::new()
                    .with_hint_text("Private key (WIF or hex)")
                    .with_monospace(),
            ];
            self.identity_index_input.clear();
            self.dpns_name_input.clear();
            self.show_pop_up_info = None;
            self.add_identity_status = AddIdentityStatus::NotStarted;
            self.success_message = None;
            return AppAction::None;
        }

        action
    }
}

impl ScreenLike for AddExistingIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Error/success display is handled by the global MessageBanner.
        // Side-effects only: update status and progress tracking.
        match message_type {
            MessageType::Error => {
                self.refresh_banner.take_and_clear();
                self.add_identity_status = AddIdentityStatus::Error;
            }
            MessageType::Success => {
                // Check if this is a final success message or a progress update
                if message.starts_with("Successfully loaded")
                    || message.starts_with("Finished loading")
                {
                    self.refresh_banner.take_and_clear();
                    self.success_message = Some(message.to_string());
                    self.add_identity_status = AddIdentityStatus::Complete;
                } else {
                    // This is a progress update - update the banner text
                    if let Some(ref handle) = self.refresh_banner {
                        handle.set_message(message);
                    }
                }
            }
            _ => {}
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::LoadedIdentity(_) => {
                self.refresh_banner.take_and_clear();
                self.success_message = Some("Successfully loaded identity.".to_string());
                self.add_identity_status = AddIdentityStatus::Complete;
            }
            BackendTaskSuccessResult::Message(msg) => {
                // Check if this is a final success message or a progress update
                if msg.starts_with("Successfully loaded") || msg.starts_with("Finished loading") {
                    self.refresh_banner.take_and_clear();
                    self.success_message = Some(msg);
                    self.add_identity_status = AddIdentityStatus::Complete;
                } else {
                    // This is a progress update - update the banner text
                    if let Some(ref handle) = self.refresh_banner {
                        handle.set_message(&msg);
                    }
                }
            }
            BackendTaskSuccessResult::Progress(msg) => {
                // Progress updates only update the existing banner in-place
                if let Some(ref handle) = self.refresh_banner {
                    handle.set_message(&msg);
                }
            }
            _ => {}
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

            // Error display is handled by the global MessageBanner

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Show success screen without the header/description/checkbox
                    if self.add_identity_status == AddIdentityStatus::Complete {
                        inner_action |= self.show_success(ui);
                        return;
                    }

                    // Heading with checkbox on the same line
                    ui.horizontal(|ui| {
                        ui.heading("Load Existing Identity");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut self.show_advanced_options, "Show Advanced Options");
                        });
                    });
                    ui.add_space(5.0);
                    ui.label("Load an identity that already exists on Dash Platform.");
                    ui.add_space(15.0);

                    let mut mode_changed = false;
                    ui.horizontal(|ui| {
                        mode_changed |= ui
                            .selectable_value(
                                &mut self.mode,
                                LoadIdentityMode::IdentityId,
                                "By Identity ID",
                            )
                            .changed();
                        mode_changed |= ui
                            .selectable_value(&mut self.mode, LoadIdentityMode::Wallet, "By Wallet")
                            .changed();
                        mode_changed |= ui
                            .selectable_value(
                                &mut self.mode,
                                LoadIdentityMode::DpnsName,
                                "By DPNS Name",
                            )
                            .changed();
                    });
                    ui.add_space(15.0);

                    if mode_changed {
                        self.add_identity_status = AddIdentityStatus::NotStarted;
                        self.success_message = None;
                    }

                    match self.mode {
                        LoadIdentityMode::IdentityId => {
                            inner_action |= self.render_by_identity(ui);
                        }
                        LoadIdentityMode::Wallet => {
                            let wallets_len = {
                                let wallets = self.app_context.wallets.read().unwrap();
                                wallets.len()
                            };
                            inner_action |= self.render_by_wallet(ui, wallets_len);
                        }
                        LoadIdentityMode::DpnsName => {
                            inner_action |= self.render_by_dpns_name(ui);
                        }
                    }

                    // Status display is handled by the global MessageBanner
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
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        action
    }
}
