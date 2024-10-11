use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::IdentityType;
use crate::platform::identity::{IdentityInputToLoad, IdentityTask};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::Context;
use rand::prelude::IteratorRandom;
use rand::thread_rng;
use serde::Deserialize;
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
struct MasternodeInfo {
    #[serde(rename = "pro-tx-hash")]
    pro_tx_hash: String,
    owner: KeyInfo,
    collateral: KeyInfo,
    voter: KeyInfo,
    operator: OperatorInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct HPMasternodeInfo {
    #[serde(rename = "protx-tx-hash")]
    protx_tx_hash: String,
    owner: KeyInfo,
    collateral: KeyInfo,
    voter: KeyInfo,
    payout: KeyInfo,
    operator: OperatorInfo,
    #[serde(rename = "node_key")]
    node_key: Option<NodeKeyInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct KeyInfo {
    address: String,
    #[serde(rename = "private_key")]
    private_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OperatorInfo {
    #[serde(rename = "public_key")]
    public_key: String,
    #[serde(rename = "private_key")]
    private_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct NodeKeyInfo {
    id: String,
    #[serde(rename = "private_key")]
    private_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TestnetNodes {
    masternodes: std::collections::HashMap<String, MasternodeInfo>,
    hp_masternodes: std::collections::HashMap<String, HPMasternodeInfo>,
}

fn load_testnet_nodes_from_yml(
    file_path: &str,
) -> Result<TestnetNodes, Box<dyn std::error::Error>> {
    let file_content = fs::read_to_string(file_path)?;
    let nodes: TestnetNodes = serde_yaml::from_str(&file_content)?;
    Ok(nodes)
}

pub enum AddIdentityStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct AddIdentityScreen {
    identity_id_input: String,
    identity_type: IdentityType,
    alias_input: String,
    voting_private_key_input: String,
    owner_private_key_input: String,
    payout_address_private_key_input: String,
    keys_input: Vec<String>,
    add_identity_status: AddIdentityStatus,
    testnet_loaded_nodes: Option<TestnetNodes>,
    pub app_context: Arc<AppContext>,
}

impl AddIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let testnet_loaded_nodes = if app_context.network == Network::Testnet {
            Some(
                load_testnet_nodes_from_yml(".testnet_nodes.yml")
                    .expect("Failed to load testnet nodes"),
            )
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
            keys_input: vec![String::new()],
            add_identity_status: AddIdentityStatus::NotStarted,
            testnet_loaded_nodes,
            app_context: app_context.clone(),
        }
    }

    fn render_identity_type_selection(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Identity Type:");
            egui::ComboBox::from_label("")
                .selected_text(format!("{:?}", self.identity_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.identity_type, IdentityType::User, "User");
                    ui.selectable_value(
                        &mut self.identity_type,
                        IdentityType::Masternode,
                        "Masternode",
                    );
                    ui.selectable_value(&mut self.identity_type, IdentityType::Evonode, "Evonode");
                });
        });
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        match self.identity_type {
            IdentityType::Masternode | IdentityType::Evonode => {
                // Store the voting and owner private key references before borrowing `self` mutably
                let voting_private_key_input = &mut self.voting_private_key_input;
                let owner_private_key_input = &mut self.owner_private_key_input;
                let payout_address_private_key_input = &mut self.payout_address_private_key_input;

                ui.horizontal(|ui| {
                    ui.label("Voting Private Key:");
                    ui.text_edit_singleline(voting_private_key_input);
                });

                ui.horizontal(|ui| {
                    ui.label("Owner Private Key:");
                    ui.text_edit_singleline(owner_private_key_input);
                });

                ui.horizontal(|ui| {
                    ui.label("Payout Address Private Key:");
                    ui.text_edit_singleline(payout_address_private_key_input);
                });
            }
            IdentityType::User => {
                // A temporary vector to store indices of keys to be removed
                let mut keys_to_remove = vec![];

                // For User, show multiple key inputs
                for (i, key) in self.keys_input.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Key {}:", i + 1));
                        ui.text_edit_singleline(key);
                        if ui.button("-").clicked() {
                            keys_to_remove.push(i);
                        }
                    });
                }

                // Remove the keys after the loop to avoid borrowing conflicts
                for i in keys_to_remove.iter().rev() {
                    self.keys_input.remove(*i);
                }

                // Add button to add more keys
                if ui.button("+ Add Key").clicked() {
                    self.keys_input.push(String::new());
                }
            }
        }
    }

    fn load_identity_clicked(&mut self, ui: &mut egui::Ui) -> AppAction {
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
}

impl ScreenLike for AddIdentityScreen {
    fn display_message(&mut self, message: String, message_type: MessageType) {
        if message_type == MessageType::Info && message == "Success" {
            self.add_identity_status = AddIdentityStatus::Complete;
        } else {
            self.add_identity_status = AddIdentityStatus::ErrorMessage(message);
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Add Identity", AppAction::None),
            ],
            None,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Add Identity");

            if self.app_context.network == Network::Testnet && self.testnet_loaded_nodes.is_some() {
                if ui.button("Fill Random HPMN").clicked() {
                    self.fill_random_hpmn();
                }

                if ui.button("Fill Random Masternode").clicked() {
                    self.fill_random_masternode();
                }
            }

            ui.horizontal(|ui| {
                ui.label("Identity ID (Hex or Base58):");
                ui.text_edit_singleline(&mut self.identity_id_input);
            });

            self.render_identity_type_selection(ui);

            // Input for Alias
            ui.horizontal(|ui| {
                ui.label("Alias:");
                ui.text_edit_singleline(&mut self.alias_input);
            });

            // Render the keys input based on identity type
            self.render_keys_input(ui);

            if ui.button("Load Identity").clicked() {
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.add_identity_status = AddIdentityStatus::WaitingForResult(now);
                action = self.load_identity_clicked(ui);
            }

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
                    ui.label(format!("Error: {}", msg));
                }
                AddIdentityStatus::Complete => {
                    action = AppAction::PopScreenAndRefresh;
                }
            }
        });

        action
    }
}
