use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::{StyledButton, island_central_panel};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt, ResultBannerExt};
use crate::ui::identity::get_selected_wallet;
use crate::ui::state::derived_key_chooser::{ChooserStatus, DerivedKeyChooser};
use crate::ui::theme::{DashColors, ResponseExt};
use crate::ui::{MessageType, ScreenLike};
use bip39::rand::{SeedableRng, rngs::StdRng};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;
use eframe::egui::{self, Frame, Margin};
use egui::{Color32, RichText, Ui};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

#[derive(PartialEq)]
pub enum AddKeyStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

pub struct AddKeyScreen {
    pub identity: QualifiedIdentity,
    pub app_context: Arc<AppContext>,
    private_key_input: PasswordInput,
    /// "Create from wallet" state: availability, slot load and selection.
    derivation: DerivedKeyChooser,
    /// The next error routed to `display_message` belongs to the chooser's own
    /// warm task (already handled in `display_backend_task_error`), not to a
    /// key submission. `AppState` calls `display_message` right after
    /// `display_backend_task_error` for an unhandled error, and this screen
    /// never handles or suppresses one, so the flag is always consumed.
    warm_error_pending: bool,
    /// The slot chooser was rendered this frame. A slot load (which may open
    /// the wallet's secret prompt) is dispatched only then — never while the
    /// wallet-locked notice or the success page hides the chooser.
    derivation_visible: bool,
    key_type: KeyType,
    purpose: Purpose,
    security_level: SecurityLevel,
    add_key_status: AddKeyStatus,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    contract_id_input: String,
    document_type_input: String,
    enable_contract_bounds: bool,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    refresh_banner: Option<BannerHandle>,
}

impl AddKeyScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let identity_clone = identity.clone();
        let selected_key = identity_clone.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::MASTER]),
            KeyType::all_key_types().into(),
            false,
        );
        let selected_wallet = get_selected_wallet(&identity, None, selected_key)
            .or_show_error(app_context.egui_ctx())
            .unwrap_or(None);
        let derivation = DerivedKeyChooser::new(app_context, &identity, KeyType::ECDSA_SECP256K1);

        Self {
            identity,
            app_context: app_context.clone(),
            derivation,
            warm_error_pending: false,
            derivation_visible: false,
            private_key_input: PasswordInput::new()
                .with_hint_text("Private key (hex)")
                .with_char_limit(64)
                .with_monospace(),
            key_type: KeyType::ECDSA_SECP256K1,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            add_key_status: AddKeyStatus::NotStarted,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            contract_id_input: String::new(),
            document_type_input: String::new(),
            enable_contract_bounds: false,
            completed_fee_result: None,
            refresh_banner: None,
        }
    }

    /// Create a new AddKeyScreen pre-configured for adding a DashPay ENCRYPTION key.
    /// This is required for sending contact requests.
    pub fn new_for_dashpay_encryption(
        identity: QualifiedIdentity,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let identity_clone = identity.clone();
        let selected_key = identity_clone.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::MASTER]),
            KeyType::all_key_types().into(),
            false,
        );
        let selected_wallet = get_selected_wallet(&identity, None, selected_key)
            .or_show_error(app_context.egui_ctx())
            .unwrap_or(None);
        let derivation = DerivedKeyChooser::new(app_context, &identity, KeyType::ECDSA_SECP256K1);

        let dashpay_contract_id = app_context
            .dashpay_contract
            .id()
            .to_string(Encoding::Base58);

        Self {
            identity,
            app_context: app_context.clone(),
            derivation,
            warm_error_pending: false,
            derivation_visible: false,
            private_key_input: PasswordInput::new()
                .with_hint_text("Private key (hex)")
                .with_char_limit(64)
                .with_monospace(),
            key_type: KeyType::ECDSA_SECP256K1,
            purpose: Purpose::ENCRYPTION,
            security_level: SecurityLevel::MEDIUM,
            add_key_status: AddKeyStatus::NotStarted,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            contract_id_input: dashpay_contract_id,
            document_type_input: String::new(),
            enable_contract_bounds: true,
            completed_fee_result: None,
            refresh_banner: None,
        }
    }

    /// Create a new AddKeyScreen pre-configured for adding a DashPay DECRYPTION key.
    /// This is required for receiving contact requests.
    pub fn new_for_dashpay_decryption(
        identity: QualifiedIdentity,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let identity_clone = identity.clone();
        let selected_key = identity_clone.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::MASTER]),
            KeyType::all_key_types().into(),
            false,
        );
        let selected_wallet = get_selected_wallet(&identity, None, selected_key)
            .or_show_error(app_context.egui_ctx())
            .unwrap_or(None);
        let derivation = DerivedKeyChooser::new(app_context, &identity, KeyType::ECDSA_SECP256K1);

        let dashpay_contract_id = app_context
            .dashpay_contract
            .id()
            .to_string(Encoding::Base58);

        Self {
            identity,
            app_context: app_context.clone(),
            derivation,
            warm_error_pending: false,
            derivation_visible: false,
            private_key_input: PasswordInput::new()
                .with_hint_text("Private key (hex)")
                .with_char_limit(64)
                .with_monospace(),
            key_type: KeyType::ECDSA_SECP256K1,
            purpose: Purpose::DECRYPTION,
            security_level: SecurityLevel::MEDIUM,
            add_key_status: AddKeyStatus::NotStarted,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            contract_id_input: dashpay_contract_id,
            document_type_input: String::new(),
            enable_contract_bounds: true,
            completed_fee_result: None,
            refresh_banner: None,
        }
    }

    fn validate_and_add_key(&mut self) -> AppAction {
        let mut app_action = AppAction::None;
        // Handle contract bounds if enabled
        let contract_bounds = if self.enable_contract_bounds && !self.contract_id_input.is_empty() {
            match Identifier::from_string(&self.contract_id_input, Encoding::Base58) {
                Ok(contract_id) => {
                    if self.document_type_input.is_empty() {
                        Some(ContractBounds::SingleContract { id: contract_id })
                    } else {
                        Some(ContractBounds::SingleContractDocumentType {
                            id: contract_id,
                            document_type_name: self.document_type_input.clone(),
                        })
                    }
                }
                Err(error) => {
                    self.add_key_status = AddKeyStatus::Error;
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "The contract ID is not valid. Check the ID and try again.",
                        MessageType::Error,
                    )
                    .with_details(error);
                    return app_action;
                }
            }
        } else {
            None
        };

        if self.derivation.derived() {
            let Some(index) = self.derivation.submit() else {
                return app_action;
            };
            let new_key = IdentityPublicKeyV0 {
                id: 0,
                key_type: self.key_type,
                purpose: self.purpose,
                security_level: self.security_level,
                data: Vec::new().into(),
                read_only: false,
                disabled_at: None,
                contract_bounds,
            };
            return AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::AddDerivedKeyToIdentity {
                    identity: self.identity.clone(),
                    key: QualifiedIdentityPublicKey::from(
                        dash_sdk::platform::IdentityPublicKey::from(new_key),
                    ),
                    index,
                    expected_key_id: self.derivation.expected_key_id(),
                },
            ));
        }
        // Convert the input string to bytes (hex decoding)
        match hex::decode(self.private_key_input.text()) {
            Ok(private_key_bytes_vec) if private_key_bytes_vec.len() == 32 => {
                let private_key_bytes: [u8; 32] = private_key_bytes_vec
                    .try_into()
                    .expect("invariant: length checked to be 32 in the match guard");
                let public_key_data_result = self.key_type.public_key_data_from_private_key_data(
                    &private_key_bytes,
                    self.app_context.network,
                );
                if let Err(error) = public_key_data_result {
                    self.add_key_status = AddKeyStatus::Error;
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "The private key could not be verified. Check the key and try again.",
                        MessageType::Error,
                    )
                    .with_details(error);
                } else {
                    let new_key = IdentityPublicKeyV0 {
                        id: self.identity.identity.get_public_key_max_id() + 1,
                        key_type: self.key_type,
                        purpose: self.purpose,
                        security_level: self.security_level,
                        data: public_key_data_result
                            .expect("invariant: Err handled in the preceding branch")
                            .into(),
                        read_only: false,
                        disabled_at: None,
                        contract_bounds,
                    };

                    // Validate the private key against the public key
                    let validation_result = new_key
                        .validate_private_key_bytes(&private_key_bytes, self.app_context.network);
                    if let Err(error) = validation_result {
                        self.add_key_status = AddKeyStatus::Error;
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "The private key could not be verified. Check the key and try again.",
                            MessageType::Error,
                        )
                        .with_details(error);
                    } else if validation_result
                        .expect("invariant: Err handled in the preceding branch")
                    {
                        let new_qualified_key = QualifiedIdentityPublicKey {
                            identity_public_key: new_key.into(),
                            in_wallet_at_derivation_path: None,
                        };
                        app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::AddKeyToIdentity(
                                self.identity.clone(),
                                new_qualified_key,
                                private_key_bytes,
                            ),
                        ));
                    } else {
                        self.add_key_status = AddKeyStatus::Error;
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "Private key does not match the public key.",
                            MessageType::Error,
                        );
                    }
                }
            }
            Ok(_) => {
                self.add_key_status = AddKeyStatus::Error;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Private key not 32 bytes",
                    MessageType::Error,
                );
            }
            Err(_) => {
                self.add_key_status = AddKeyStatus::Error;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Invalid hex string for private key.",
                    MessageType::Error,
                );
            }
        }
        app_action
    }

    fn show_key_source(&mut self, ui: &mut Ui) {
        ui.label("Key source:");
        let possible = self.derivation.is_possible();
        ui.vertical(|ui| {
            let response = ui
                .add_enabled(
                    possible,
                    egui::Checkbox::new(self.derivation.derived_mut(), "Create from wallet"),
                )
                .clickable_tooltip(
                    "A key created from your wallet can be restored later with your wallet's recovery phrase.",
                )
                .disabled_tooltip(
                    "A single matching wallet could not be identified on this device. Enter a private key instead.",
                );
            if response.changed() {
                self.private_key_input.clear();
            }
            if !possible {
                ui.label(
                    "A single matching wallet could not be identified on this device. Enter a private key instead.",
                );
            }
        });
        ui.end_row();
        if self.derivation.derived() {
            self.show_derivation_index(ui);
        } else {
            ui.label("Private Key:");
            ui.horizontal(|ui| {
                self.private_key_input.show(ui);
                if ui.button("Generate Random").clicked() {
                    self.generate_random_private_key();
                }
            });
            ui.end_row();
        }
    }

    fn show_derivation_index(&mut self, ui: &mut Ui) {
        self.derivation_visible = true;
        ui.label("Wallet key slot:");
        ui.vertical(|ui| {
            match self.derivation.status() {
                // Unreachable while "Create from wallet" is on; kept for totality.
                ChooserStatus::NoWallet => {
                    ui.label(
                        "A single matching wallet could not be identified on this device. Enter a private key instead.",
                    );
                }
                ChooserStatus::UnsupportedKeyType => {
                    ui.label(
                        "This key type cannot be created from a wallet. Choose ECDSA_SECP256K1 or ECDSA_HASH160 as the key type, or turn off Create from wallet to enter a private key.",
                    );
                }
                ChooserStatus::WalletUnavailable => {
                    ui.label("The wallet is unavailable. Reopen the wallet and try again.");
                }
                ChooserStatus::RefreshingIdentity => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                        ui.label("Updating this identity from the network…");
                    });
                }
                ChooserStatus::Loading => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                        ui.label("Loading wallet key slots…");
                    });
                }
                ChooserStatus::LoadFailed => {
                    ui.horizontal(|ui| {
                        ui.label("The wallet key slots could not be loaded.");
                        if StyledButton::new("Retry").show(ui).clicked() {
                            self.derivation.retry();
                        }
                    });
                }
                ChooserStatus::NoFreeSlot => {
                    ui.label(
                        "All wallet key slots available to this identity are in use. Turn off Create from wallet to enter a private key instead.",
                    );
                }
                ChooserStatus::Ready => self.show_slot_list(ui),
            }
            if self.derivation.identity_protected() {
                ui.label(
                    "This key will be protected by your wallet, not by this identity's password.",
                );
            }
        });
        ui.end_row();
    }

    fn show_slot_list(&mut self, ui: &mut Ui) {
        let limit = self.derivation.limit();
        let selected = self.derivation.selected_index();
        egui::ComboBox::from_id_salt("derived_key_index")
            .selected_text(selected.map_or_else(String::new, |index| format!("Slot {index}")))
            .show_ui(ui, |ui| {
                for index in 0..limit {
                    let used = self.derivation.is_occupied(index);
                    let label = if used {
                        format!("Slot {index} (in use)")
                    } else {
                        format!("Slot {index}")
                    };
                    let response = ui.add_enabled_ui(!used, |ui| {
                        ui.selectable_value(self.derivation.index_mut(), Some(index), label)
                    });
                    response
                        .inner
                        .disabled_tooltip("This slot is already used by a key on this identity.");
                }
            });
        if let Some(suggested) = self.derivation.suggested_index() {
            ui.label(format!(
                "Other wallet apps restore this key most reliably from slot {suggested}. Choose slot {suggested} unless you need a different one."
            ));
        }
    }

    /// Why the Add Key button is disabled, or `None` when it is enabled.
    fn add_blocked_reason(&self) -> Option<&'static str> {
        if self.add_key_status == AddKeyStatus::WaitingForResult {
            return Some("The key is being added. Wait for it to finish.");
        }
        if !self.derivation.derived() {
            return None;
        }
        match self.derivation.status() {
            ChooserStatus::Ready => None,
            ChooserStatus::NoWallet => Some(
                "A single matching wallet could not be identified on this device. Enter a private key instead.",
            ),
            ChooserStatus::UnsupportedKeyType => Some(
                "Choose a key type that can be created from a wallet, or turn off Create from wallet.",
            ),
            ChooserStatus::WalletUnavailable => {
                Some("The wallet is unavailable. Reopen the wallet and try again.")
            }
            ChooserStatus::RefreshingIdentity => {
                Some("Wait for this identity to finish updating from the network.")
            }
            ChooserStatus::Loading => Some("Wait for the wallet key slots to load."),
            ChooserStatus::LoadFailed => {
                Some("The wallet key slots could not be loaded. Select Retry to try again.")
            }
            ChooserStatus::NoFreeSlot => Some(
                "All wallet key slots available to this identity are in use. Turn off Create from wallet to enter a private key instead.",
            ),
        }
    }

    /// Reload the identity from local storage and recompute the chooser.
    fn reload_identity(&mut self) {
        self.load_identity();
        self.derivation.reload(&self.app_context, &self.identity);
    }

    /// Reload the identity from local storage; keeps the current copy when it
    /// is not stored here.
    fn load_identity(&mut self) {
        match self
            .app_context
            .get_local_qualified_identity(&self.identity.identity.id())
        {
            Ok(Some(identity)) => self.identity = identity,
            Ok(None) => {}
            Err(error) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "This identity could not be reloaded from this device. Go back and open it again.",
                    MessageType::Error,
                )
                .with_details(error);
            }
        }
    }

    fn generate_random_private_key(&mut self) {
        // Create a new random number generator
        let mut rng = StdRng::from_entropy();

        // Generate a random private key based on the selected key type
        if let Ok((_, private_key_bytes)) = self
            .key_type
            .random_public_and_private_key_data(&mut rng, self.app_context.platform_version())
        {
            self.private_key_input
                .set_text(hex::encode(private_key_bytes));
        } else {
            self.add_key_status = AddKeyStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Failed to generate a random private key.",
                MessageType::Error,
            );
        }
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let action = crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Key Added Successfully!".to_string(),
            vec![
                (
                    "Back to Identities Screen".to_string(),
                    AppAction::PopScreenAndRefresh,
                ),
                (
                    "Add another key".to_string(),
                    AppAction::Custom("add_another".to_string()),
                ),
            ],
            None,
        );

        // Handle the custom action to reset the form and refresh identity
        if let AppAction::Custom(ref s) = action
            && s == "add_another"
        {
            self.private_key_input.clear();
            // Hold the slot list until the refreshed identity arrives, so the
            // slot just used is never offered again.
            self.derivation.await_identity_refresh();
            self.contract_id_input = String::new();
            self.document_type_input = String::new();
            self.enable_contract_bounds = false;
            self.add_key_status = AddKeyStatus::NotStarted;
            self.completed_fee_result = None;
            return AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshIdentity(self.identity.clone()),
            ));
        }

        action
    }
}

impl ScreenLike for AddKeyScreen {
    fn refresh(&mut self) {
        // Keeps an in-flight slot load and a still-valid selection: refresh
        // nudges arrive from unrelated tasks too.
        self.reload_identity();
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Error/success display is handled by the global MessageBanner.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            if std::mem::take(&mut self.warm_error_pending) {
                return;
            }
            self.refresh_banner.take_and_clear();
            self.add_key_status = AddKeyStatus::Error;
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        // Every slot-load failure is consumed here, even another wallet's: it
        // is never a failed submission, so the follow-up `display_message`
        // must not mark the form failed.
        if let Some((seed_hash, identity_index)) = context.identity_auth_pubkey_warm() {
            self.derivation.warm_failed(&seed_hash, identity_index);
            self.warm_error_pending = true;
            return;
        }
        if context.refreshed_identity() == Some(self.identity.identity.id()) {
            self.derivation
                .identity_refresh_finished(&self.app_context, &self.identity);
            return;
        }
        match error {
            TaskError::DerivedKeyIndexUnavailable => self.derivation.slot_rejected(),
            TaskError::DerivedKeyIdChanged => self.derivation.key_id_changed(),
            TaskError::DerivedKeySeedMismatch => self.derivation.key_unconfirmed(),
            _ => {}
        }
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        if matches!(
            result,
            BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { .. }
        ) {
            if let Some((seed_hash, identity_index)) = context.identity_auth_pubkey_warm() {
                self.derivation.warm_finished(
                    &self.app_context,
                    &self.identity,
                    &seed_hash,
                    identity_index,
                );
            }
            return;
        }
        self.display_task_result(result);
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::AddedKeyToIdentity(fee_result) => {
                self.refresh_banner.take_and_clear();
                self.completed_fee_result = Some(fee_result);
                self.add_key_status = AddKeyStatus::Complete;
            }
            BackendTaskSuccessResult::RefreshedIdentity(identity)
                if identity.identity.id() == self.identity.identity.id() =>
            {
                self.load_identity();
                self.derivation
                    .identity_refresh_finished(&self.app_context, &self.identity);
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        self.derivation_visible = false;
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Add Key", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentityHub,
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;

            // Show the success screen if the key was added successfully
            if self.add_key_status == AddKeyStatus::Complete {
                inner_action |= self.show_success(ui);
                return inner_action;
            }

            ui.heading("Add New Key");
            ui.add_space(10.0);

            if self.selected_wallet.is_some()
                && let Some(wallet) = &self.selected_wallet
            {
                if !self.wallet_open_attempted {
                    if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                            .disable_auto_dismiss();
                    }
                    self.wallet_open_attempted = true;
                }
                if wallet_needs_unlock(wallet) {
                    ui.add_space(10.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 150, 50),
                        "Wallet is locked. Please unlock to continue.",
                    );
                    ui.add_space(8.0);
                    if ui.button("Unlock Wallet").clicked() {
                        self.wallet_unlock_popup.open();
                    }
                    return inner_action;
                }
            }

            egui::Grid::new("add_key_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .striped(false)
                .show(ui, |ui| {
                    // Purpose
                    ui.label("Purpose:");
                    let prev_purpose = self.purpose;
                    egui::ComboBox::from_id_salt("purpose_selector")
                        .selected_text(format!("{purpose:?}", purpose = self.purpose))
                        .show_ui(ui, |ui| {
                            if self.enable_contract_bounds {
                                // When contract bounds are enabled, only allow ENCRYPTION and DECRYPTION
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::ENCRYPTION,
                                    "ENCRYPTION",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::DECRYPTION,
                                    "DECRYPTION",
                                );
                            } else {
                                // When contract bounds are disabled, show all purpose options
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::AUTHENTICATION,
                                    "AUTHENTICATION",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::TRANSFER,
                                    "TRANSFER",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::ENCRYPTION,
                                    "ENCRYPTION",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::DECRYPTION,
                                    "DECRYPTION",
                                );
                            }
                        });

                    // Auto-set security level when purpose changes
                    if self.purpose != prev_purpose {
                        match self.purpose {
                            Purpose::ENCRYPTION | Purpose::DECRYPTION => {
                                self.security_level = SecurityLevel::MEDIUM;
                            }
                            Purpose::TRANSFER => {
                                self.security_level = SecurityLevel::CRITICAL;
                            }
                            // AUTHENTICATION allows multiple levels, keep current if valid
                            // otherwise default to CRITICAL
                            Purpose::AUTHENTICATION
                                if self.security_level != SecurityLevel::CRITICAL
                                    && self.security_level != SecurityLevel::HIGH
                                    && self.security_level != SecurityLevel::MEDIUM =>
                            {
                                self.security_level = SecurityLevel::CRITICAL;
                            }
                            _ => {}
                        }
                    }
                    ui.end_row();

                    // Security Level
                    ui.label("Security Level:");
                    // Only AUTHENTICATION has multiple security level options
                    let has_multiple_security_levels = self.purpose == Purpose::AUTHENTICATION;
                    let inner_response = ui.add_enabled_ui(has_multiple_security_levels, |ui| {
                        egui::ComboBox::from_id_salt("security_level_selector")
                            .selected_text(format!(
                                "{security_level:?}",
                                security_level = self.security_level
                            ))
                            .show_ui(ui, |ui| {
                                if self.enable_contract_bounds {
                                    // When contract bounds are enabled, only allow MEDIUM
                                    ui.selectable_value(
                                        &mut self.security_level,
                                        SecurityLevel::MEDIUM,
                                        "MEDIUM",
                                    );
                                } else if self.purpose == Purpose::AUTHENTICATION {
                                    ui.selectable_value(
                                        &mut self.security_level,
                                        SecurityLevel::CRITICAL,
                                        "CRITICAL",
                                    );
                                    ui.selectable_value(
                                        &mut self.security_level,
                                        SecurityLevel::HIGH,
                                        "HIGH",
                                    );
                                    ui.selectable_value(
                                        &mut self.security_level,
                                        SecurityLevel::MEDIUM,
                                        "MEDIUM",
                                    );
                                } else if self.purpose == Purpose::ENCRYPTION
                                    || self.purpose == Purpose::DECRYPTION
                                {
                                    // ENCRYPTION and DECRYPTION only allow MEDIUM
                                    ui.selectable_value(
                                        &mut self.security_level,
                                        SecurityLevel::MEDIUM,
                                        "MEDIUM",
                                    );
                                } else {
                                    // TRANSFER only allows CRITICAL
                                    ui.selectable_value(
                                        &mut self.security_level,
                                        SecurityLevel::CRITICAL,
                                        "CRITICAL",
                                    );
                                }
                            })
                    });
                    if !has_multiple_security_levels {
                        // Use interact with hover sense to detect hover on disabled widget
                        let hover_response = ui.interact(
                            inner_response.response.rect,
                            egui::Id::new("security_level_tooltip"),
                            egui::Sense::hover(),
                        );
                        hover_response.info_tooltip(format!(
                            "{purpose:?} purpose requires {security_level:?} security level",
                            purpose = self.purpose,
                            security_level = self.security_level
                        ));
                    }
                    ui.end_row();

                    // Key Type
                    ui.label("Key Type:");
                    let prev_key_type = self.key_type;
                    egui::ComboBox::from_id_salt("key_type_selector")
                        .selected_text(format!("{key_type:?}", key_type = self.key_type))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::ECDSA_SECP256K1,
                                "ECDSA_SECP256K1",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::BLS12_381,
                                "BLS12_381",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::ECDSA_HASH160,
                                "ECDSA_HASH160",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::EDDSA_25519_HASH160,
                                "EDDSA_25519_HASH160",
                            );
                            // ui.selectable_value(
                            //     &mut self.key_type,
                            //     KeyType::BIP13_SCRIPT_HASH,
                            //     "BIP13_SCRIPT_HASH",
                            // );
                        });
                    if self.key_type != prev_key_type {
                        self.derivation.set_key_type(self.key_type);
                    }
                    ui.end_row();

                    self.show_key_source(ui);

                    // Contract Bounds Toggle
                    ui.label("Enable Contract Bounds:");
                    let prev_contract_bounds = self.enable_contract_bounds;
                    ui.checkbox(&mut self.enable_contract_bounds, "");

                    // If contract bounds was just enabled, set required values
                    if self.enable_contract_bounds && !prev_contract_bounds {
                        self.purpose = Purpose::ENCRYPTION;
                        self.security_level = SecurityLevel::MEDIUM;
                    }
                    ui.end_row();

                    // Contract ID Input (only shown if contract bounds are enabled)
                    if self.enable_contract_bounds {
                        ui.label("Contract ID:");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.contract_id_input);
                            ui.label(RichText::new("(required)").size(10.0).color(Color32::GRAY));
                        });
                        ui.end_row();

                        // Document Type Input
                        ui.label("Document Type Name:");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.document_type_input);
                            ui.label(RichText::new("(optional)").size(10.0).color(Color32::GRAY));
                        });
                        ui.end_row();
                    }
                });
            ui.add_space(20.0);

            // Fee estimation display
            let fee_estimator = self.app_context.fee_estimator();
            let estimated_fee = fee_estimator.estimate_identity_update();

            let dark_mode = ui.style().visuals.dark_mode;
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Estimated fee:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(format_credits_as_dash(estimated_fee))
                                .color(DashColors::text_primary(dark_mode))
                                .size(14.0),
                        );
                    });
                });

            ui.add_space(10.0);

            // Add Key button
            let mut new_style = (**ui.style()).clone();
            new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
            ui.set_style(new_style);
            let button = egui::Button::new(RichText::new("Add Key").color(Color32::WHITE))
                .fill(DashColors::DASH_BLUE)
                .frame(true)
                .corner_radius(3.0);
            let blocked_reason = self.add_blocked_reason();
            let add_response = ui.add_enabled(blocked_reason.is_none(), button);
            let add_response = match blocked_reason {
                Some(reason) => add_response.disabled_tooltip(reason),
                None => add_response,
            };
            if add_response.clicked() {
                let validation_action = self.validate_and_add_key();
                if matches!(&validation_action, AppAction::BackendTask(_)) {
                    self.add_key_status = AddKeyStatus::WaitingForResult;
                    let handle =
                        MessageBanner::set_global(ui.ctx(), "Adding key...", MessageType::Info);
                    handle.with_elapsed();
                    self.refresh_banner = Some(handle);
                }
                inner_action |= validation_action;
            }
            // Status display is handled by the global MessageBanner

            inner_action
        });

        // Chooser follow-ups go out only on an otherwise idle frame:
        // `AppAction` keeps a single task, and a dropped warm or refresh would
        // leave the chooser waiting forever.
        if matches!(action, AppAction::None) {
            if self.derivation.take_identity_refresh() {
                action = AppAction::BackendTask(BackendTask::IdentityTask(
                    IdentityTask::RefreshIdentity(self.identity.clone()),
                ));
            } else if self.derivation_visible
                && let Some(task) = self.derivation.take_warm_task()
            {
                action = AppAction::BackendTask(task);
            }
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

#[cfg(test)]
mod derived_key_tests {
    use super::*;
    use crate::backend_task::wallet::WalletTask;
    use crate::context::test_staging::{StagedIdentity, stage_identity_with_vaulted_keys};
    use crate::model::derived_identity_key::test_support::fixture;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    /// A staged context plus the derivation fixture's identity; the public-key
    /// cache is warm when `warm` is set.
    async fn staged_screen_parts(warm: bool) -> (StagedIdentity, QualifiedIdentity) {
        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let (identity, cache, seed_hash, _) = fixture();
        if warm {
            staged
                .ctx
                .wallet_backend()
                .unwrap()
                .auth_pubkey_cache()
                .put(staged.ctx.network, &seed_hash, &cache)
                .unwrap();
        }
        (staged, identity)
    }

    fn source_harness(screen: AddKeyScreen) -> Harness<'static, AddKeyScreen> {
        Harness::builder().with_max_steps(30).build_ui_state(
            |ui, screen: &mut AddKeyScreen| {
                egui::Grid::new("source").show(ui, |ui| {
                    screen.show_key_source(ui);
                });
            },
            screen,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_checkbox_defaults_on_and_switches_private_input() {
        let (staged, identity) = staged_screen_parts(true).await;
        let screen = AddKeyScreen::new(identity, &staged.ctx);
        assert!(screen.derivation.derived());
        let mut harness = source_harness(screen);
        harness.run();
        assert_eq!(harness.state().derivation.selected_index(), Some(1));
        assert!(harness.query_by_label("Private Key:").is_none());
        harness.get_by_label("Create from wallet").click();
        harness.run();
        assert!(!harness.state().derivation.derived());
        assert!(harness.query_by_label("Private Key:").is_some());
        assert!(harness.query_by_label("Wallet key slot:").is_none());
        harness
            .state_mut()
            .private_key_input
            .set_text("sensitive input".to_string());
        harness.get_by_label("Create from wallet").click();
        harness.run();
        assert!(harness.state().private_key_input.text().is_empty());
        assert_eq!(harness.state().derivation.selected_index(), Some(1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_occupied_index_cannot_be_selected_and_submit_carries_only_index() {
        let (staged, identity) = staged_screen_parts(true).await;
        let screen = AddKeyScreen::new(identity, &staged.ctx);
        let mut harness = source_harness(screen);
        harness.run();
        harness.get_by_role(egui::accesskit::Role::ComboBox).click();
        harness.run();
        assert!(
            harness
                .get_by_label("Slot 0 (in use)")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Slot 0 (in use)").click();
        harness.run();
        assert_eq!(harness.state().derivation.selected_index(), Some(1));
        if harness.query_by_label("Slot 2").is_none() {
            harness.get_by_role(egui::accesskit::Role::ComboBox).click();
            harness.run();
        }
        harness.get_by_label("Slot 2").click();
        harness.run();
        assert_eq!(harness.state().derivation.selected_index(), Some(2));
        let AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::AddDerivedKeyToIdentity {
                key,
                index,
                expected_key_id,
                ..
            },
        )) = harness.state_mut().validate_and_add_key()
        else {
            panic!("derived submission must use the derived backend task");
        };
        assert_eq!(index, 2);
        // SEC-103: the key id the slot was chosen against travels with the
        // add, even when the user picked a slot off it.
        assert_eq!(expected_key_id, 1);
        assert!(key.identity_public_key.data().is_empty());
        assert!(key.in_wallet_at_derivation_path.is_none());
    }

    /// CALL-001: an identity with no wallet path opens on manual entry, and the
    /// screen says why a wallet key is not offered.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_defaults_to_manual_entry_without_a_wallet_path() {
        let (staged, mut identity) = staged_screen_parts(true).await;
        identity.private_keys = Default::default();
        let screen = AddKeyScreen::new(identity, &staged.ctx);
        assert!(!screen.derivation.derived());
        assert_eq!(screen.add_blocked_reason(), None);
        let mut harness = source_harness(screen);
        harness.run();
        assert!(harness.query_by_label("Private Key:").is_some());
        assert!(
            harness
                .get_by_label("Create from wallet")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .query_by_label(
                    "A single matching wallet could not be identified on this device. Enter a private key instead."
                )
                .is_some()
        );
    }

    /// SEC-002: the default slot is the one matching the new key's id, even
    /// when a lower slot is free.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_default_slot_matches_the_new_key_id() {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        let (staged, mut identity) = staged_screen_parts(true).await;
        // A manually entered key takes id 2; the new key will get id 3.
        identity.identity.add_public_key(
            IdentityPublicKeyV0 {
                id: 2,
                key_type: KeyType::ECDSA_SECP256K1,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::HIGH,
                contract_bounds: None,
                read_only: false,
                disabled_at: None,
                data: dash_sdk::dpp::dashcore::secp256k1::PublicKey::from_slice(&[
                    0x02, 0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95,
                    0xCE, 0x87, 0x0B, 0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59,
                    0xF2, 0x81, 0x5B, 0x16, 0xF8, 0x17, 0x98,
                ])
                .unwrap()
                .serialize()
                .to_vec()
                .into(),
            }
            .into(),
        );
        let screen = AddKeyScreen::new(identity, &staged.ctx);
        assert_eq!(screen.derivation.selected_index(), Some(3));
        assert_eq!(screen.derivation.suggested_index(), None);
    }

    /// RUST-007 / QA-003: the slot load is tracked apart from the submission,
    /// dispatched once, never re-dispatched by a refresh, and Retry appears
    /// only after it actually failed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_slot_load_fails_and_retries_independently_of_submission() {
        let (staged, identity) = staged_screen_parts(false).await;
        let mut screen = AddKeyScreen::new(identity, &staged.ctx);
        assert_eq!(screen.derivation.status(), ChooserStatus::Loading);
        assert_eq!(
            screen.add_blocked_reason(),
            Some("Wait for the wallet key slots to load.")
        );
        let warm = screen
            .derivation
            .take_warm_task()
            .expect("a cold cache dispatches one warm task");
        assert!(screen.derivation.take_warm_task().is_none());
        screen.refresh();
        assert!(
            screen.derivation.take_warm_task().is_none(),
            "a refresh must not dispatch a duplicate warm task"
        );

        let context = BackendTaskContext::from(&warm);
        screen.display_backend_task_error(&context, &TaskError::WalletLocked);
        screen.display_message("The wallet is locked.", MessageType::Error);
        assert_eq!(screen.derivation.status(), ChooserStatus::LoadFailed);
        assert!(
            screen.add_key_status == AddKeyStatus::NotStarted,
            "a failed slot load is not a failed submission"
        );

        // A submission error is still recorded as one.
        screen.display_backend_task_error(&BackendTaskContext::Other, &TaskError::WalletLocked);
        screen.display_message("The wallet is locked.", MessageType::Error);
        assert!(screen.add_key_status == AddKeyStatus::Error);

        let mut harness = source_harness(screen);
        harness.run();
        harness.get_by_label("Retry").click();
        // The spinner that follows repaints forever; step instead of `run`.
        harness.run_steps(2);
        assert!(harness.state_mut().derivation.take_warm_task().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_ignores_another_wallet_warm_before_its_own_completion() {
        let (staged, identity) = staged_screen_parts(false).await;
        let mut screen = AddKeyScreen::new(identity, &staged.ctx);
        let warm = screen.derivation.take_warm_task().unwrap();
        let own_context = BackendTaskContext::from(&warm);
        let (seed_hash, identity_index) = own_context.identity_auth_pubkey_warm().unwrap();
        let mut other_seed_hash = seed_hash;
        other_seed_hash[0] ^= 1;
        let other_context = BackendTaskContext::IdentityAuthPubkeyWarm {
            seed_hash: other_seed_hash,
            identity_index,
        };
        screen.display_backend_task_result(
            &other_context,
            BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { identity_index },
        );
        assert_eq!(screen.derivation.status(), ChooserStatus::Loading);
        assert!(screen.derivation.take_warm_task().is_none());

        let (_, cache, _, _) = fixture();
        staged
            .ctx
            .wallet_backend()
            .unwrap()
            .auth_pubkey_cache()
            .put(staged.ctx.network, &seed_hash, &cache)
            .unwrap();
        screen.display_backend_task_result(
            &own_context,
            BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { identity_index },
        );
        assert_eq!(screen.derivation.status(), ChooserStatus::Ready);
        assert!(screen.derivation.selected_index().is_some());
        assert!(screen.add_blocked_reason().is_none());
    }

    /// Another wallet's slot-load failure is not this form's failed
    /// submission, and leaves this chooser's own load untouched.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_another_wallet_warm_error_does_not_fail_the_form() {
        let (staged, identity) = staged_screen_parts(false).await;
        let mut screen = AddKeyScreen::new(identity, &staged.ctx);
        let warm = screen.derivation.take_warm_task().unwrap();
        let (mut other_seed_hash, identity_index) = BackendTaskContext::from(&warm)
            .identity_auth_pubkey_warm()
            .unwrap();
        other_seed_hash[0] ^= 1;
        let other_context = BackendTaskContext::IdentityAuthPubkeyWarm {
            seed_hash: other_seed_hash,
            identity_index,
        };

        screen.display_backend_task_error(&other_context, &TaskError::WalletLocked);
        screen.display_message("The wallet is locked.", MessageType::Error);

        assert!(
            screen.add_key_status == AddKeyStatus::NotStarted,
            "another wallet's slot load is not a failed submission"
        );
        assert_eq!(screen.derivation.status(), ChooserStatus::Loading);
    }

    /// A refresh that drops the identity's wallet path while a slot load is in
    /// flight, then restores it, starts a fresh load instead of waiting
    /// forever on a result the chooser no longer accepts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_wallet_change_while_loading_restarts_the_load() {
        let (staged, identity) = staged_screen_parts(false).await;
        let mut screen = AddKeyScreen::new(identity.clone(), &staged.ctx);
        let warm = screen.derivation.take_warm_task().unwrap();
        let (seed_hash, identity_index) = BackendTaskContext::from(&warm)
            .identity_auth_pubkey_warm()
            .unwrap();

        let mut without_wallet = identity.clone();
        without_wallet.wallet_index = Some(identity_index + 1);
        screen.derivation.reload(&staged.ctx, &without_wallet);
        assert_eq!(screen.derivation.status(), ChooserStatus::NoWallet);

        screen.derivation.reload(&staged.ctx, &identity);
        // The first wallet's stale completion arrives after the change and is
        // ignored.
        screen.display_backend_task_result(
            &BackendTaskContext::from(&warm),
            BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { identity_index },
        );
        assert_eq!(screen.derivation.status(), ChooserStatus::Loading);
        let Some(BackendTask::WalletTask(WalletTask::WarmIdentityAuthPubkeys {
            seed_hash: warmed_seed_hash,
            ..
        })) = screen.derivation.take_warm_task()
        else {
            panic!("the restored wallet must dispatch a fresh slot load");
        };
        assert_eq!(warmed_seed_hash, seed_hash);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_warm_that_leaves_the_cache_cold_does_not_loop() {
        let (staged, identity) = staged_screen_parts(false).await;
        let mut screen = AddKeyScreen::new(identity, &staged.ctx);
        let warm = screen.derivation.take_warm_task().unwrap();
        screen.display_backend_task_result(
            &BackendTaskContext::from(&warm),
            BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { identity_index: 0 },
        );
        assert_eq!(screen.derivation.status(), ChooserStatus::LoadFailed);
        assert!(screen.derivation.take_warm_task().is_none());
    }

    /// RUST-005: a slot the backend rejects is dropped, the identity is
    /// reloaded from the network, and a new free slot is selected.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_rejected_slot_refreshes_and_selects_another() {
        let (staged, identity) = staged_screen_parts(true).await;
        for (seed, wallet) in &identity.associated_wallets {
            staged
                .ctx
                .wallet_context()
                .insert_test_wallet(*seed, Arc::clone(wallet));
        }
        let mut screen = AddKeyScreen::new(identity.clone(), &staged.ctx);
        assert_eq!(screen.derivation.selected_index(), Some(1));

        assert!(matches!(
            screen.validate_and_add_key(),
            AppAction::BackendTask(_)
        ));
        screen.add_key_status = AddKeyStatus::WaitingForResult;
        screen.display_backend_task_error(
            &BackendTaskContext::Other,
            &TaskError::DerivedKeyIndexUnavailable,
        );
        screen.display_message("rejected", MessageType::Error);
        assert!(screen.add_key_status == AddKeyStatus::Error);
        assert_eq!(
            screen.derivation.status(),
            ChooserStatus::RefreshingIdentity
        );
        assert!(screen.add_blocked_reason().is_some());
        assert!(screen.derivation.take_identity_refresh());
        assert!(!screen.derivation.take_identity_refresh());

        screen.display_task_result(BackendTaskSuccessResult::RefreshedIdentity(identity));
        assert_eq!(screen.derivation.status(), ChooserStatus::Ready);
        assert_eq!(screen.derivation.selected_index(), Some(2));
        assert!(screen.derivation.is_occupied(1));
    }

    /// A slot changed while the add is pending does not take the blame for
    /// the submitted slot's rejection: the submitted slot is marked used and
    /// the new selection survives the reload.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_rejection_marks_the_submitted_slot_not_the_current_one() {
        let (staged, identity) = staged_screen_parts(true).await;
        for (seed, wallet) in &identity.associated_wallets {
            staged
                .ctx
                .wallet_context()
                .insert_test_wallet(*seed, Arc::clone(wallet));
        }
        let mut screen = AddKeyScreen::new(identity.clone(), &staged.ctx);
        assert_eq!(screen.derivation.selected_index(), Some(1));

        let action = screen.validate_and_add_key();
        let AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::AddDerivedKeyToIdentity { index, .. },
        )) = action
        else {
            panic!("a derived add must dispatch AddDerivedKeyToIdentity");
        };
        assert_eq!(index, 1);
        screen.add_key_status = AddKeyStatus::WaitingForResult;

        // The user picks another slot before the rejection arrives.
        *screen.derivation.index_mut() = Some(3);
        screen.display_backend_task_error(
            &BackendTaskContext::Other,
            &TaskError::DerivedKeyIndexUnavailable,
        );
        screen.display_message("rejected", MessageType::Error);
        assert!(screen.derivation.take_identity_refresh());

        screen.display_task_result(BackendTaskSuccessResult::RefreshedIdentity(identity));
        assert_eq!(screen.derivation.status(), ChooserStatus::Ready);
        assert!(
            screen.derivation.is_occupied(1),
            "the submitted slot is used"
        );
        assert!(
            !screen.derivation.is_occupied(3),
            "the slot selected later is not blamed"
        );
        assert_eq!(screen.derivation.selected_index(), Some(3));
    }

    /// SEC-103: when the network assigned a different key id than the one the
    /// slot was chosen against, the selection is dropped (not blacklisted),
    /// the identity is reloaded, and the default slot and expected key id
    /// follow the fresh record.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_id_change_refreshes_and_reselects_the_matching_slot() {
        use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;

        let (staged, mut identity) = staged_screen_parts(true).await;
        identity.identity.set_id(staged.id);
        for (seed, wallet) in &identity.associated_wallets {
            staged
                .ctx
                .wallet_context()
                .insert_test_wallet(*seed, Arc::clone(wallet));
        }
        staged
            .ctx
            .update_local_qualified_identity(&identity)
            .unwrap();
        let mut screen = AddKeyScreen::new(identity.clone(), &staged.ctx);
        assert_eq!(screen.derivation.selected_index(), Some(1));
        assert_eq!(screen.derivation.expected_key_id(), 1);

        screen.add_key_status = AddKeyStatus::WaitingForResult;
        screen.display_backend_task_error(
            &BackendTaskContext::Other,
            &TaskError::DerivedKeyIdChanged,
        );
        screen.display_message("changed", MessageType::Error);
        assert_eq!(
            screen.derivation.status(),
            ChooserStatus::RefreshingIdentity
        );
        assert!(screen.derivation.take_identity_refresh());

        // Another device added a key (not from this wallet) at id 1.
        let mut refreshed = identity.clone();
        let mut foreign = identity.identity.public_keys()[&0].clone();
        foreign.set_id(1);
        let secret = SecretKey::from_slice(&[9; 32]).unwrap();
        foreign.set_data(
            PublicKey::from_secret_key(&Secp256k1::new(), &secret)
                .serialize()
                .to_vec()
                .into(),
        );
        refreshed.identity.add_public_key(foreign);
        staged
            .ctx
            .update_local_qualified_identity(&refreshed)
            .unwrap();
        screen.display_task_result(BackendTaskSuccessResult::RefreshedIdentity(refreshed));

        assert_eq!(screen.derivation.status(), ChooserStatus::Ready);
        assert_eq!(screen.derivation.expected_key_id(), 2);
        assert_eq!(screen.derivation.selected_index(), Some(2));
        assert!(
            !screen.derivation.is_occupied(1),
            "a key-id change does not mark the slot used"
        );
    }
}
