mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::identity::{IdentityKeys, IdentityRegistrationInfo, IdentityTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::Component;
use crate::ui::components::funding_widget::FundingWidget;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::secp256k1::hashes::hex::DisplayHex;
use dash_sdk::dpp::dashcore::{PrivateKey, Transaction};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::Identifier;
use eframe::egui::Context;
use egui::ahash::HashSet;
use egui::{Button, Color32, ComboBox, ScrollArea};
use std::cmp::PartialEq;
use std::fmt;
use std::sync::{Arc, RwLock};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum FundingMethod {
    NoSelection,
    UseUnusedAssetLock,
    UseWalletBalance,
    AddressWithQRCode,
}

impl fmt::Display for FundingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = match self {
            FundingMethod::NoSelection => "Select funding method",
            FundingMethod::AddressWithQRCode => "Address with QR Code",
            FundingMethod::UseWalletBalance => "Use Wallet Balance",
            FundingMethod::UseUnusedAssetLock => "Use Unused Asset Lock (recommended)",
        };
        write!(f, "{}", output)
    }
}

pub struct AddNewIdentityScreen {
    identity_id_number: u32,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    funding_address: Option<Address>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    alias_input: String,
    identity_keys: IdentityKeys,
    error_message: Option<String>,
    show_password: bool,
    wallet_password: String,
    show_pop_up_info: Option<String>,
    in_key_selection_advanced_mode: bool,
    pub app_context: Arc<AppContext>,
    successful_qualified_identity_id: Option<Identifier>,
    funding_widget: Option<FundingWidget>,
}

impl AddNewIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        // Initialize funding widget immediately
        let funding_widget = FundingWidget::new(app_context.clone())
            .with_default_amount(crate::model::amount::Amount::new_dash(0.5)); // 0.5 DASH

        Self {
            identity_id_number: 0, // updated later when wallet is selected
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            selected_wallet: None, // will be set by funding widget
            funding_address: None,
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "0.5".to_string(),
            funding_amount_exact: None,
            alias_input: String::new(),
            identity_keys: IdentityKeys {
                master_private_key: None,
                master_private_key_type: KeyType::ECDSA_HASH160,
                keys_input: vec![],
            },
            error_message: None,
            show_password: false,
            wallet_password: "".to_string(),
            show_pop_up_info: None,
            in_key_selection_advanced_mode: false,
            app_context: app_context.clone(),
            successful_qualified_identity_id: None,
            funding_widget: Some(funding_widget),
        }
    }

    /// Ensure that identity keys are correctly set up and generated.
    ///
    /// If the master key is not set, it generates a new master key and derives other keys from it.
    /// Otherwise, it updates the existing keys based on the current wallet and identity index.
    ///
    /// ## Return value
    ///
    /// * Ok(()) when the keys are correctly set up.
    /// * Err(String) if there was an error during the process, e.g. wallet not open
    pub fn ensure_correct_identity_keys(&mut self) -> Result<(), String> {
        if self.identity_keys.master_private_key.is_some() {
            return match self.update_identity_key() {
                Ok(true) => Ok(()),
                Ok(false) => Err("failed to update identity keys".to_string()),
                Err(e) => Err(format!("failed to update identity keys: {}", e)),
            };
        }

        if let Some(wallet_lock) = &self.selected_wallet {
            // sanity checks
            {
                let wallet = wallet_lock.read().unwrap();
                if !wallet.is_open() {
                    return Err(format!(
                        "wallet {} is not open",
                        wallet
                            .alias
                            .as_ref()
                            .unwrap_or(&wallet.seed_hash().to_lower_hex_string())
                    ));
                }
            }

            let app_context = &self.app_context;
            let identity_id_number = self.next_identity_id(); // note: this grabs rlock on the wallet

            const DEFAULT_KEY_TYPES: [(KeyType, Purpose, SecurityLevel); 3] = [
                (
                    KeyType::ECDSA_HASH160,
                    Purpose::AUTHENTICATION,
                    SecurityLevel::CRITICAL,
                ),
                (
                    KeyType::ECDSA_HASH160,
                    Purpose::AUTHENTICATION,
                    SecurityLevel::HIGH,
                ),
                (
                    KeyType::ECDSA_HASH160,
                    Purpose::TRANSFER,
                    SecurityLevel::CRITICAL,
                ),
            ];
            let mut wallet = wallet_lock.write().expect("wallet lock failed");
            let master_key = wallet.identity_authentication_ecdsa_private_key(
                app_context.network,
                identity_id_number,
                0,
                Some(app_context),
            )?;

            let other_keys = DEFAULT_KEY_TYPES
                .into_iter()
                .enumerate()
                .map(|(i, (key_type, purpose, security_level))| {
                    Ok((
                        wallet.identity_authentication_ecdsa_private_key(
                            app_context.network,
                            identity_id_number,
                            (i + 1).try_into().expect("key index must fit u32"), // key index 0 is the master key
                            Some(app_context),
                        )?,
                        key_type,
                        purpose,
                        security_level,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;

            self.identity_keys = IdentityKeys {
                master_private_key: Some(master_key),
                master_private_key_type: KeyType::ECDSA_HASH160,
                keys_input: other_keys,
            };

            Ok(())
        } else {
            Err("no wallet selected".to_string())
        }
    }

    fn render_identity_index_input(&mut self, ui: &mut egui::Ui) {
        let mut index_changed = false; // Track if the index has changed

        ui.horizontal(|ui| {
            ui.label("Identity Index:");

            // Check if we have access to the selected wallet
            if let Some(wallet_guard) = self.selected_wallet.as_ref() {
                let wallet = wallet_guard.read().unwrap();
                let used_indices: HashSet<u32> = wallet.identities.keys().cloned().collect();

                // Modify the selected text to include "(used)" if the current index is used
                let selected_text = {
                    let is_used = used_indices.contains(&self.identity_id_number);
                    if is_used {
                        format!("{} (used)", self.identity_id_number)
                    } else {
                        format!("{}", self.identity_id_number)
                    }
                };

                // Render a ComboBox to select the identity index
                ComboBox::from_id_salt("identity_index")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        // Provide up to 30 entries for selection (0 to 29)
                        for i in 0..30 {
                            let is_used = used_indices.contains(&i);
                            let label = if is_used {
                                format!("{} (used)", i)
                            } else {
                                format!("{}", i)
                            };

                            let is_selected = self.identity_id_number == i;

                            // Enable the option if it's not used or if it's the currently selected index
                            let enabled = !is_used || is_selected;

                            // Use `add_enabled` to disable used indices
                            let response =
                                ui.add_enabled(enabled, Button::selectable(is_selected, label));

                            // Only allow selection if the index is not used
                            if response.clicked() && !is_used {
                                self.identity_id_number = i;
                                index_changed = true;
                            }
                        }
                    });
            } else {
                ui.label("No wallet selected");
            }
        });

        // If the index has changed, update the identity key
        if index_changed {
            self.ensure_correct_identity_keys()
                .expect("failed to update identity key");
        }
    }

    // fn render_wallet_unlock(&mut self, ui: &mut Ui) -> bool {
    //     if let Some(wallet_guard) = self.selected_wallet.as_ref() {
    //         let mut wallet = wallet_guard.write().unwrap();
    //
    //         // Only render the unlock prompt if the wallet requires a password and is locked
    //         if wallet.uses_password && !wallet.is_open() {
    //             ui.add_space(10.0);
    //             ui.label("This wallet is locked. Please enter the password to unlock it:");
    //
    //             let mut unlocked = false;
    //             ui.horizontal(|ui| {
    //                 let password_input = ui.add(
    //                     egui::TextEdit::singleline(&mut self.wallet_password)
    //                         .password(!self.show_password)
    //                         .hint_text("Enter password"),
    //                 );
    //
    //                 ui.checkbox(&mut self.show_password, "Show Password");
    //
    //                 unlocked = if password_input.lost_focus()
    //                     && ui.input(|i| i.key_pressed(egui::Key::Enter))
    //                 {
    //                     let unlocked = match wallet.wallet_seed.open(&self.wallet_password) {
    //                         Ok(_) => {
    //                             self.error_message = None; // Clear any previous error
    //                             true
    //                         }
    //                         Err(_) => {
    //                             if let Some(hint) = wallet.password_hint() {
    //                                 self.error_message = Some(format!(
    //                                     "Incorrect Password, password hint is {}",
    //                                     hint
    //                                 ));
    //                             } else {
    //                                 self.error_message = Some("Incorrect Password".to_string());
    //                             }
    //                             false
    //                         }
    //                     };
    //                     // Clear the password field after submission
    //                     self.wallet_password.zeroize();
    //                     unlocked
    //                 } else {
    //                     false
    //                 };
    //             });
    //
    //             // Display error message if the password was incorrect
    //             if let Some(error_message) = &self.error_message {
    //                 ui.add_space(5.0);
    //                 ui.colored_label(Color32::RED, error_message);
    //             }
    //
    //             return unlocked;
    //         }
    //     }
    //     false
    // }

    /// Update selected wallet and trigger all dependent actions, like updating identity keys
    /// and identity index.
    ///
    /// This function is called whenever a wallet was changed in the UI or unlocked
    fn update_wallet(&mut self, wallet: Arc<RwLock<Wallet>>) {
        let is_open = wallet.read().expect("wallet lock poisoned").is_open();

        self.selected_wallet = Some(wallet);
        self.identity_id_number = self.next_identity_id();

        if is_open {
            self.ensure_correct_identity_keys()
                .expect("failed to initialize keys")
        }
    }

    /// Generate next identity ID that can be used for the new identity.
    ///
    /// TODO: This function is not working in a reliable way, because it relies on the
    /// `identities` map in the wallet, which may not be up to date (user can remove
    /// identities from the wallet while they still are stored on the Platform).
    fn next_identity_id(&self) -> u32 {
        self.selected_wallet
            .as_ref()
            .unwrap()
            .read()
            .unwrap()
            .identities
            .keys()
            .copied()
            .max()
            .map(|max| max + 1)
            .unwrap_or_default()
    }

    // Function to render the key selection mode (Default or Advanced)
    fn render_key_selection(&mut self, ui: &mut egui::Ui) {
        // Provide the selection toggle for Default or Advanced mode
        ui.horizontal(|ui| {
            ui.label("Key Selection Mode:");

            ComboBox::from_id_salt("key_selection_mode")
                .selected_text(if self.in_key_selection_advanced_mode {
                    "Advanced"
                } else {
                    "Default"
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            !self.in_key_selection_advanced_mode,
                            "Default (Recommended)",
                        )
                        .clicked()
                    {
                        self.in_key_selection_advanced_mode = false;
                    }
                    if ui
                        .selectable_label(self.in_key_selection_advanced_mode, "Advanced")
                        .clicked()
                    {
                        self.in_key_selection_advanced_mode = true;
                    }
                });
        });

        ui.add_space(10.0);

        // Render additional key options only if "Advanced" mode is selected
        if self.in_key_selection_advanced_mode {
            // Render the master key input
            if let Some((master_key, _)) = self.identity_keys.master_private_key {
                self.render_master_key(ui, master_key);
            }

            // Render additional keys input (if any) and allow adding more keys
            self.render_keys_input(ui);
        } else {
            ui.colored_label(Color32::DARK_GREEN, "Default allows for most operations on Platform: updating the identity, interacting with data contracts, transferring credits to other identities, and withdrawing to the Core payment chain. More keys can always be added later.".to_string());
        }
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];

        for (i, ((key, _), key_type, purpose, security_level)) in
            self.identity_keys.keys_input.iter_mut().enumerate()
        {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label(format!(" • Key {}:", i + 1));
                ui.label(key.to_wif());

                // Purpose selection
                ComboBox::from_id_salt(format!("purpose_combo_{}", i))
                    .selected_text(format!("{:?}", purpose))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(purpose, Purpose::AUTHENTICATION, "AUTHENTICATION");
                        ui.selectable_value(purpose, Purpose::TRANSFER, "TRANSFER");
                    });

                // Key Type selection with conditional filtering
                ComboBox::from_id_salt(format!("key_type_combo_{}", i))
                    .selected_text(format!("{:?}", key_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(key_type, KeyType::ECDSA_HASH160, "ECDSA_HASH160");
                        ui.selectable_value(key_type, KeyType::ECDSA_SECP256K1, "ECDSA_SECP256K1");
                        // ui.selectable_value(key_type, KeyType::BLS12_381, "BLS12_381");
                        // ui.selectable_value(
                        //     key_type,
                        //     KeyType::EDDSA_25519_HASH160,
                        //     "EDDSA_25519_HASH160",
                        // );
                    });

                // Security Level selection with conditional filtering
                ComboBox::from_id_salt(format!("security_level_combo_{}", i))
                    .selected_text(format!("{:?}", security_level))
                    .show_ui(ui, |ui| {
                        if *purpose == Purpose::TRANSFER {
                            // For TRANSFER purpose, security level is locked to CRITICAL
                            *security_level = SecurityLevel::CRITICAL;
                            ui.label("Locked to CRITICAL");
                        } else {
                            // For AUTHENTICATION, allow all except MASTER
                            ui.selectable_value(
                                security_level,
                                SecurityLevel::CRITICAL,
                                "CRITICAL",
                            );
                            ui.selectable_value(security_level, SecurityLevel::HIGH, "HIGH");
                            ui.selectable_value(security_level, SecurityLevel::MEDIUM, "MEDIUM");
                        }
                    });

                if ui.button("-").clicked() {
                    keys_to_remove.push(i);
                }
            });
        }

        // Remove keys marked for deletion
        for i in keys_to_remove.iter().rev() {
            self.identity_keys.keys_input.remove(*i);
        }

        // Add new key input entry
        ui.add_space(15.0);
        if ui.button("+ Add Key").clicked() {
            self.add_identity_key(
                KeyType::ECDSA_HASH160,  // Default key type
                Purpose::AUTHENTICATION, // Default purpose
                SecurityLevel::HIGH,     // Default security level
            );
        }
    }

    fn register_identity_clicked(
        &mut self,
        funding_widget_response: &crate::ui::components::funding_widget::FundingWidgetResponse,
    ) -> AppAction {
        let Some(selected_wallet) = &self.selected_wallet else {
            return AppAction::None;
        };
        if self.identity_keys.master_private_key.is_none() {
            return AppAction::None;
        };

        // reset error message
        self.error_message = None;

        // Process the funding method
        if let Some(funding_method) = &funding_widget_response.funding_secured {
            let register_identity_funding_method = funding_method
                .clone()
                .to_register_identity_funding_method(self.identity_id_number);

            let identity_input = IdentityRegistrationInfo {
                alias_input: self.alias_input.clone(),
                keys: self.identity_keys.clone(),
                wallet: Arc::clone(selected_wallet),
                wallet_identity_index: self.identity_id_number,
                identity_funding_method: register_identity_funding_method,
            };

            // Set the appropriate step based on funding method
            let mut step = self.step.write().unwrap();
            *step = match funding_method {
                crate::ui::components::funding_widget::FundingWidgetMethod::UseAssetLock(
                    _,
                    _,
                    _,
                ) => WalletFundedScreenStep::WaitingForPlatformAcceptance,
                crate::ui::components::funding_widget::FundingWidgetMethod::FundWithWallet(_) => {
                    WalletFundedScreenStep::WaitingForAssetLock
                }
                crate::ui::components::funding_widget::FundingWidgetMethod::FundWithUtxo(
                    _,
                    _,
                    _,
                ) => WalletFundedScreenStep::WaitingForAssetLock,
            };

            AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                identity_input,
            )))
        } else {
            AppAction::None
        }
    }

    /// Update existing identity keys based on the current wallet and identity index.
    ///
    /// When the wallet is updated, we need to ensure that all the private keys are
    /// generated with the correct parameters (seed, derivation path, etc.).
    ///
    /// If the master key is not set, this function is a no-op and returns Ok(false).
    fn update_identity_key(&mut self) -> Result<bool, String> {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let mut wallet = wallet_guard.write().unwrap();
            let identity_index = self.identity_id_number;

            // Update the master private key and keys input from the wallet
            self.identity_keys.master_private_key =
                Some(wallet.identity_authentication_ecdsa_private_key(
                    self.app_context.network,
                    identity_index,
                    0,
                    Some(&self.app_context),
                )?);

            // Update the additional keys input
            self.identity_keys.keys_input = self
                .identity_keys
                .keys_input
                .iter()
                .enumerate()
                .map(|(key_index, (_, key_type, purpose, security_level))| {
                    Ok((
                        wallet.identity_authentication_ecdsa_private_key(
                            self.app_context.network,
                            identity_index,
                            key_index as u32 + 1,
                            Some(&self.app_context),
                        )?,
                        *key_type,
                        *purpose,
                        *security_level,
                    ))
                })
                .collect::<Result<_, String>>()?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn add_identity_key(
        &mut self,
        key_type: KeyType,
        purpose: Purpose,
        security_level: SecurityLevel,
    ) {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let mut wallet = wallet_guard.write().unwrap();
            let new_key_index = self.identity_keys.keys_input.len() as u32 + 1;

            // Add a new key with default parameters
            self.identity_keys.keys_input.push((
                wallet
                    .identity_authentication_ecdsa_private_key(
                        self.app_context.network,
                        self.identity_id_number,
                        new_key_index,
                        Some(&self.app_context),
                    )
                    .expect("expected to have decrypted wallet"),
                key_type, // Default key type
                purpose,
                security_level,
            ));
        }
    }

    fn render_master_key(&mut self, ui: &mut egui::Ui, key: PrivateKey) {
        ui.horizontal(|ui| {
            ui.label(" • Master Private Key:");
            ui.label(key.to_wif());

            ComboBox::from_id_salt("master_key_type")
                .selected_text(format!("{:?}", self.identity_keys.master_private_key_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.identity_keys.master_private_key_type,
                        KeyType::ECDSA_SECP256K1,
                        "ECDSA_SECP256K1",
                    );
                    ui.selectable_value(
                        &mut self.identity_keys.master_private_key_type,
                        KeyType::ECDSA_HASH160,
                        "ECDSA_HASH160",
                    );
                });
        });
    }
}

impl ScreenWithWalletUnlock for AddNewIdentityScreen {
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

impl ScreenLike for AddNewIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Error {
            self.error_message = Some(format!("Error registering identity: {}", message));
            let mut step = self.step.write().unwrap();
            if *step == WalletFundedScreenStep::WaitingForAssetLock {
                // Funding rejected, reset to funding method selection
                *step = WalletFundedScreenStep::ChooseFundingMethod;
            }
        } else {
            self.error_message = Some(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        let mut step = self.step.write().unwrap();
        match *step {
            WalletFundedScreenStep::ChooseFundingMethod => {}
            WalletFundedScreenStep::WaitingForAssetLock => {
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                ) = backend_task_success_result
                {
                    if let Some(TransactionPayload::AssetLockPayloadType(asset_lock_payload)) =
                        tx.special_transaction_payload
                    {
                        if asset_lock_payload.credit_outputs.iter().any(|tx_out| {
                            let Ok(address) = Address::from_script(
                                &tx_out.script_pubkey,
                                self.app_context.network,
                            ) else {
                                return false;
                            };
                            if let Some(wallet) = &self.selected_wallet {
                                let wallet = wallet.read().unwrap();
                                wallet.known_addresses.contains_key(&address)
                            } else {
                                false
                            }
                        }) {
                            *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
                        }
                    }
                }
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                if let BackendTaskSuccessResult::RegisteredIdentity(qualified_identity) =
                    backend_task_success_result
                {
                    self.successful_qualified_identity_id = Some(qualified_identity.identity.id());
                    *step = WalletFundedScreenStep::Success;
                }
            }
            WalletFundedScreenStep::Success => {}
        }
    }
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Create Identity", AppAction::None),
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
            // Ensure we use whole window width
            ui.set_width(ui.available_width());
            ScrollArea::vertical().show(ui, |ui| {
                let step = {*self.step.read().unwrap()};
                if step == WalletFundedScreenStep::Success {
                    inner_action |= self.show_success(ui);
                    return;
                }
                ui.add_space(10.0);
                ui.heading("Follow these steps to create your identity!");
                ui.add_space(15.0);

                // Step 1: Show FundingWidget with wallet selection
                ui.heading("1. Select wallet and provide funding details");
                ui.add_space(10.0);

                // Track funding readiness
                let mut funding_ready = false;
                let mut funding_widget_response: Option<crate::ui::components::funding_widget::FundingWidgetResponse> = None;

                // Render the funding widget first - it handles wallet selection
                if let Some(ref mut widget) = self.funding_widget {
                    // Disable balance checks if operation is in progress
                    let step = {*self.step.read().unwrap()};
                    let enabled = step == WalletFundedScreenStep::ChooseFundingMethod;
                    let response_data = ui.add_enabled_ui(enabled, |ui|{
                        widget.show(ui).inner
                    }).inner;

                    // Store the response for later use
                    funding_widget_response = Some(response_data.clone());

                    // Handle wallet changes from the funding widget
                    if let Some(wallet) = response_data.wallet_changed {
                        self.update_wallet(wallet);
                        // Clear funding asset lock when wallet changes
                        self.funding_asset_lock = None;
                    }

                    if let Some(method) = response_data.funding_method_changed {
                        let mut funding_method_guard = self.funding_method.write().unwrap();
                        *funding_method_guard = method;
                        // Clear funding asset lock when method changes
                        self.funding_asset_lock = None;
                    }

                    if let Some(amount) = response_data.amount_changed {
                        self.funding_amount = amount.clone();
                        self.funding_amount_exact = amount.parse::<f64>().ok().map(|f| {
                            (f * 1e8) as u64 // Convert to Duffs
                        });
                    }

                    if let Some(address) = response_data.address_changed {
                        self.funding_address = Some(address);
                    }

                    if let Some(asset_lock) = response_data.asset_lock_selected {
                        self.funding_asset_lock = Some(asset_lock);
                    }

                    if let Some(error) = response_data.error {
                       self.error_message = Some(error);
                    }

                    // Get funding readiness from the widget response
                    funding_ready = response_data.funding_secured.is_some() || step!= WalletFundedScreenStep::ChooseFundingMethod;
                }

                // Don't proceed if funding is not ready
                if funding_ready {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock {
                        if just_unlocked {
                            // Select wallet will properly update all dependencies
                            self.update_wallet(self.selected_wallet.clone().expect("we just checked selected_wallet set above"));
                        } else {
                            return;
                        }
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Step 2: Identity index selection
                    ui.horizontal(|ui| {
                        let wallet_guard = self.selected_wallet.as_ref().unwrap();
                        let wallet = wallet_guard.read().unwrap();
                        if wallet.identities.is_empty() {
                            ui.heading("2. Choose an identity index. Leave this 0 if this is your first identity for this wallet.");
                        } else {
                            ui.heading(format!(
                                "2. Choose an identity index. Leaving this {} is recommended.",
                                self.next_identity_id(),
                            ));
                        }


                        // Create info icon button with tooltip
                        let response = crate::ui::helpers::info_icon_button(ui, "The identity index is an internal reference within the wallet. The wallet's seed phrase can always be used to recover any identity, including this one, by using the same index.");

                        // Check if the label was clicked
                        if response.clicked() {
                            self.show_pop_up_info = Some("The identity index is an internal reference within the wallet. The wallet’s seed phrase can always be used to recover any identity, including this one, by using the same index.".to_string());
                        }
                    });

                    ui.add_space(8.0);

                    self.render_identity_index_input(ui);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Step 3: Key selection
                    ui.horizontal(|ui| {
                        ui.heading("3. Choose what keys you want to add to this new identity.");

                        // Create info icon button with tooltip
                        let response = crate::ui::helpers::info_icon_button(ui, "Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.");

                        // Check if the label was clicked
                        if response.clicked() {
                            self.show_pop_up_info = Some("Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.".to_string());
                        }
                    });

                    ui.add_space(8.0);

                    self.render_key_selection(ui);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Step 4: Identity alias and registration
                    ui.heading("4. Final details and registration");
                    ui.add_space(10.0);

                    // Add alias input
                    ui.horizontal(|ui| {
                        ui.label("Identity Alias (Optional):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.alias_input)
                                .hint_text("Enter a friendly name for this identity")
                                .desired_width(200.0),
                        );
                    });

                    if let Some(ref widget_response) = funding_widget_response {
                        ui.add_space(15.0);
                        ui.separator();
                        ui.add_space(10.0);

                        let step = self.step.read().unwrap().to_owned();

                        let enabled= !matches!(step,
                            WalletFundedScreenStep::WaitingForAssetLock |
                            WalletFundedScreenStep::WaitingForPlatformAcceptance);

                        ui.add_enabled_ui(enabled,|ui|{
                            if ui.button("Register Identity").clicked() {
                                inner_action |= self.register_identity_clicked(widget_response);
                            }
                        }).response.on_disabled_hover_text(format!("Registration in progress, please wait until it is finished. Current step: {step}"));
                    }
                } // end of if funding_ready

                // Show error message if any
                if let Some(error_message) = self.error_message.as_ref() {
                    ui.add_space(10.0);
                    ui.colored_label(Color32::DARK_RED, error_message);
                }

                // Show step status
                let step = *self.step.read().unwrap();
                ui.add_space(20.0);
                ui.vertical_centered(|ui| match step {
                    WalletFundedScreenStep::WaitingForAssetLock => {
                        ui.heading("=> Creating asset lock transaction <=");
                    }
                    WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                        ui.heading("=> Waiting for Platform acknowledgement <=");
                        ui.add_space(10.0);
                        ui.label("NOTE: If this gets stuck, the funds were likely either transferred to the wallet or asset locked,\nand you can use the funding method selector to change the method and use those funds to complete the process.");
                    }
                    WalletFundedScreenStep::Success => {
                        ui.heading("...Success...");
                    }
                    _ => {}
                });
            });

            inner_action
        });

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::Window::new("Identity Index Information")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(show_pop_up_info_text);

                    // Add a close button to dismiss the popup
                    if ui.button("Close").clicked() {
                        self.show_pop_up_info = None
                    }
                });
        }

        action
    }
}
