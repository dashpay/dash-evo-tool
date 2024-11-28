use crate::app::AppAction;
use crate::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel, TimestampMillis};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::Context;
use egui::{Color32, RichText};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub enum RegisterDpnsNameStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct RegisterDpnsNameScreen {
    pub show_identity_selector: bool,
    pub qualified_identities: Vec<(QualifiedIdentity, Vec<IdentityPublicKey>)>,
    pub selected_qualified_identity: Option<(QualifiedIdentity, Vec<IdentityPublicKey>)>,
    name_input: String,
    register_dpns_name_status: RegisterDpnsNameStatus,
    pub app_context: Arc<AppContext>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl RegisterDpnsNameScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let security_level_of_contract = app_context
            .dpns_contract
            .document_type_for_name("domain")
            .unwrap()
            .security_level_requirement();
        let security_level_requirements = SecurityLevel::CRITICAL..=security_level_of_contract;

        let qualified_identities: Vec<_> = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| {
                let keys = e
                    .identity
                    .public_keys()
                    .values()
                    .filter(|key| {
                        key.purpose() == Purpose::AUTHENTICATION
                            && security_level_requirements.contains(&key.security_level())
                            && !key.is_disabled()
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if keys.is_empty() {
                    None
                } else {
                    Some((e, keys))
                }
            })
            .collect();
        let selected_qualified_identity = qualified_identities.first().cloned();

        let mut error_message: Option<String> = None;
        let selected_wallet = get_selected_wallet(
            &selected_qualified_identity,
            app_context,
            &mut error_message,
        );

        let show_identity_selector = qualified_identities.len() > 0;
        Self {
            show_identity_selector,
            qualified_identities,
            selected_qualified_identity,
            name_input: String::new(),
            register_dpns_name_status: RegisterDpnsNameStatus::NotStarted,
            app_context: app_context.clone(),
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message,
        }
    }

    pub fn select_identity(&mut self, identity_id: Identifier) {
        // Find the qualified identity with the matching identity_id
        if let Some(qi) = self
            .qualified_identities
            .iter()
            .find(|(qi, _)| qi.identity.id() == &identity_id)
        {
            // Set the selected_qualified_identity to the found identity
            self.selected_qualified_identity = Some(qi.clone());
        } else {
            // If not found, you might want to handle this case
            // For now, we'll set selected_qualified_identity to None
            self.selected_qualified_identity = None;
        }
    }

    fn render_identity_id_selection(&mut self, ui: &mut egui::Ui) {
        if self.qualified_identities.len() == 1 {
            // Only one identity, display it directly
            let qualified_identity = &self.qualified_identities[0];
            ui.horizontal(|ui| {
                ui.label("Identity ID:");
                ui.label(
                    qualified_identity
                        .0
                        .alias
                        .as_ref()
                        .unwrap_or(
                            &qualified_identity
                                .0
                                .identity
                                .id()
                                .to_string(Encoding::Base58),
                        )
                        .clone(),
                );
            });
            self.selected_qualified_identity = Some(qualified_identity.clone());
        } else {
            // Multiple identities, display ComboBox
            ui.horizontal(|ui| {
                ui.label("Identity ID:");

                // Create a ComboBox for selecting a Qualified Identity
                egui::ComboBox::from_label("")
                    .selected_text(
                        self.selected_qualified_identity
                            .as_ref()
                            .map(|qi| {
                                qi.0.alias
                                    .as_ref()
                                    .unwrap_or(&qi.0.identity.id().to_string(Encoding::Base58))
                                    .clone()
                            })
                            .unwrap_or_else(|| "Select an identity".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        // Loop through the qualified identities and display each as selectable
                        for qualified_identity in &self.qualified_identities {
                            // Display each QualifiedIdentity as a selectable item
                            if ui
                                .selectable_value(
                                    &mut self.selected_qualified_identity,
                                    Some(qualified_identity.clone()),
                                    qualified_identity.0.alias.as_ref().unwrap_or(
                                        &qualified_identity
                                            .0
                                            .identity
                                            .id()
                                            .to_string(Encoding::Base58),
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_qualified_identity = Some(qualified_identity.clone());

                                // Reset the error message
                                self.error_message = None;

                                // Update the selected wallet using the helper function
                                self.selected_wallet = get_selected_wallet(
                                    &self.selected_qualified_identity,
                                    &self.app_context,
                                    &mut self.error_message,
                                );

                                // Optionally handle the case where no wallet is found
                                if self.selected_wallet.is_none() && self.error_message.is_none() {
                                    self.error_message = Some(
                                        "No associated wallet found for the selected identity."
                                            .to_string(),
                                    );
                                }
                            }
                        }
                    });
            });
        }
    }

    fn register_dpns_name_clicked(&mut self) -> AppAction {
        let Some(qualified_identity) = self.selected_qualified_identity.as_ref() else {
            return AppAction::None;
        };
        let dpns_name_input = RegisterDpnsNameInput {
            qualified_identity: qualified_identity.0.clone(),
            name_input: self.name_input.trim().to_string(),
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(
            dpns_name_input,
        )))
    }
}

impl ScreenLike for RegisterDpnsNameScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "Successfully registered dpns name" {
                    self.register_dpns_name_status = RegisterDpnsNameStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.register_dpns_name_status =
                    RegisterDpnsNameStatus::ErrorMessage(message.to_string());
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contested Names", AppAction::GoToMainScreen),
                ("Register DPNS Name", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Register DPNS Name");
            ui.add_space(10.0);

            // If no identities loaded, give message
            if self.qualified_identities.is_empty() {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    "No qualified identities available to register a DPNS name.",
                );
                return;
            }

            // Select the identity to register the name for
            ui.heading("1. Select Identity");
            ui.add_space(5.0);
            self.render_identity_id_selection(ui);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            if self.selected_wallet.is_some() {
                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                if needed_unlock && !just_unlocked {
                    return;
                }
            }

            // Input for the name
            ui.heading("2. Enter the Name to Register:");
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Name (without \".dash\"):");
                ui.text_edit_singleline(&mut self.name_input);
            });
            ui.add_space(10.0);

            // Display if the name is contested and the estimated cost
            let name = self.name_input.trim();
            if !name.is_empty() && name.len() >= 3 {
                if is_contested_name(&name.to_lowercase()) {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        "This is a contested name.",
                    );
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        "Cost ≈ 0.2006 Dash",
                    );
                } else {
                    ui.colored_label(
                        egui::Color32::DARK_GREEN,
                        "This is not a contested name.",
                    );
                    ui.colored_label(
                        egui::Color32::DARK_GREEN,
                        "Cost ≈ 0.0006 Dash",
                    );
                }
            }

            ui.add_space(10.0);

            // Register button
            let button = egui::Button::new(RichText::new("Register Name").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .frame(true)
                .rounding(3.0)
                .min_size(egui::vec2(80.0, 30.0));

            if ui.add(button).clicked() {
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.register_dpns_name_status = RegisterDpnsNameStatus::WaitingForResult(now);
                action = self.register_dpns_name_clicked();
            }

            // Handle registration status messages
            match &self.register_dpns_name_status {
                RegisterDpnsNameStatus::NotStarted => {
                    // Do nothing
                }
                RegisterDpnsNameStatus::WaitingForResult(start_time) => {
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

                    ui.label(format!(
                        "Registering... Time taken so far: {}",
                        display_time
                    ));
                }
                RegisterDpnsNameStatus::ErrorMessage(msg) => {
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                }
                RegisterDpnsNameStatus::Complete => {
                    action = AppAction::PopScreenAndRefresh;
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // DPNS Name Constraints Explanation
            ui.heading("DPNS Name Constraints:");
            ui.add_space(5.0);
            ui.label("  • Minimum length: 3 characters");
            ui.label("  • Maximum length: 63 characters");
            ui.label("  • Allowed characters: letters (A-Z, case-insensitive), numbers (0-9), and hyphens (-)");
            ui.label("  • Cannot start or end with a hyphen (-)");
            ui.label("  • Names are case-sensitive");

            ui.add_space(20.0);

            // Contested Names Explanation
            ui.heading("Contested Names Info:");
            ui.add_space(5.0);
            ui.label("  • To prevent name front-running, some names are contested and require a higher fee to register.");
            ui.label("  • Masternodes vote whether or not to award contested names to contestants.");
            ui.label("  • Contests last two weeks and new contenders can only join during the first week.");
            ui.label("  • Contested names are those that are:");
            ui.label("  • Less than 20 characters long (i.e. “alice”, “quantumexplorer”)");
            ui.label("  • AND");
            ui.label("  • Contain no numbers or only contain the number(s) 0 and/or 1 (i.e. “bob”, “carol01”)");
        });

        action
    }
}

impl ScreenWithWalletUnlock for RegisterDpnsNameScreen {
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

pub fn is_contested_name(name: &str) -> bool {
    let length = name.len();
    if length >= 20 {
        return false;
    }
    for c in name.chars() {
        if c.is_digit(10) {
            if c != '0' && c != '1' {
                return false;
            }
        }
    }
    true
}

pub fn get_selected_wallet<PublicKey>(
    selected_qualified_identity: &Option<(QualifiedIdentity, Vec<PublicKey>)>,
    app_context: &AppContext,
    error_message: &mut Option<String>,
) -> Option<Arc<RwLock<Wallet>>> {
    let (qualified_identity, _) = selected_qualified_identity.as_ref()?;

    let dpns_contract = &app_context.dpns_contract;

    let preorder_document_type = match dpns_contract.document_type_for_name("preorder") {
        Ok(doc_type) => doc_type,
        Err(e) => {
            *error_message = Some(format!("DPNS preorder document type not found: {}", e));
            return None;
        }
    };

    let public_key = match qualified_identity.document_signing_key(&preorder_document_type) {
        Some(key) => key,
        None => {
            *error_message = Some(
                "Identity doesn't have an authentication key for signing document transitions"
                    .to_string(),
            );
            return None;
        }
    };

    let key = (PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id());

    if let Some((_, PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path))) =
        qualified_identity.private_keys.private_keys.get(&key)
    {
        qualified_identity
            .associated_wallets
            .get(&wallet_derivation_path.wallet_seed_hash)
            .cloned()
    } else {
        None
    }
}
