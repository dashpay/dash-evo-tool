use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::FeeResult;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt, ResultBannerExt};
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::identity::get_selected_wallet;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Setters;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use eframe::egui::{self, Color32, Frame, Margin, TextEdit};
use egui::{RichText, ScrollArea, Ui};
use std::sync::{Arc, RwLock};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    ParsingError(String),
    ValidContract(Box<DataContract>),
    Broadcasting,
    ProofError,
    BroadcastError(String),
    /// The registration was refused before it ran, or the network has not
    /// confirmed it; the message says what to do.
    NotRegistered(String),
    Done,
}

pub struct RegisterDataContractScreen {
    pub app_context: Arc<AppContext>,
    contract_json_input: String,
    contract_alias_input: String,
    broadcast_status: BroadcastStatus,

    pub qualified_identities: Vec<QualifiedIdentity>,
    pub selected_qualified_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    pub selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_open_attempted: bool,
    wallet_unlock_popup: WalletUnlockPopup,
    completed_fee_result: Option<FeeResult>,
    refresh_banner: Option<BannerHandle>,
    /// The contract last sent for registration, offered again after a
    /// definite refusal.
    sent_contract: Option<Box<DataContract>>,
}

impl RegisterDataContractScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let qualified_identities: Vec<QualifiedIdentity> =
            app_context.load_local_user_identities().unwrap_or_default();

        // Seed from the app-scoped selected identity (W2 SYNC); fall back to first.
        let selected_qualified_identity = app_context
            .selected_identity_id()
            .and_then(|id| {
                qualified_identities
                    .iter()
                    .find(|qi| qi.identity.id() == id)
                    .cloned()
            })
            .or_else(|| qualified_identities.first().cloned());

        let selected_wallet = if let Some(ref identity) = selected_qualified_identity {
            get_selected_wallet(identity, Some(app_context), None).unwrap_or(None)
        } else {
            None
        };

        // Auto-select a suitable key for contract registration
        use dash_sdk::dpp::identity::KeyType;
        let selected_key = selected_qualified_identity.as_ref().and_then(|identity| {
            identity
                .identity
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    [SecurityLevel::HIGH, SecurityLevel::CRITICAL].into(),
                    KeyType::all_key_types().into(),
                    false,
                )
                .cloned()
        });

        let selected_identity_string = selected_qualified_identity
            .as_ref()
            .map(|qi| {
                qi.identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            })
            .unwrap_or_default();

        Self {
            app_context: app_context.clone(),
            contract_json_input: String::new(),
            contract_alias_input: String::new(),
            broadcast_status: BroadcastStatus::Idle,

            qualified_identities,
            selected_qualified_identity,
            selected_identity_string,
            selected_key,
            show_advanced_options: false,

            selected_wallet,
            wallet_open_attempted: false,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
            refresh_banner: None,
            sent_contract: None,
        }
    }

    fn parse_contract(&mut self) {
        // Clear any previous parse/broadcast states
        self.broadcast_status = BroadcastStatus::Idle;

        if self.contract_json_input.trim().is_empty() {
            // No input yet
            return;
        }

        // Try to parse the user’s JSON -> serde_json::Value
        let json_result: Result<serde_json::Value, serde_json::Error> =
            serde_json::from_str(&self.contract_json_input);

        match json_result {
            Ok(json_val) => {
                let platform_version = self.app_context.platform_version();
                match DataContract::from_json(json_val, true, platform_version) {
                    Ok(mut contract) => {
                        // ------------------------------------------
                        // 1) Overwrite the contract’s ownerId
                        // ------------------------------------------
                        if let Some(qualified_identity) = &self.selected_qualified_identity {
                            let new_owner_id = qualified_identity.identity.id();
                            contract.set_owner_id(new_owner_id);
                        }

                        // Mark it as a valid contract in our screen state
                        self.broadcast_status = BroadcastStatus::ValidContract(Box::new(contract));
                    }
                    Err(e) => {
                        self.broadcast_status =
                            BroadcastStatus::ParsingError(format!("DataContract parse error: {e}"));
                    }
                }
            }
            Err(e) => {
                self.broadcast_status = BroadcastStatus::ParsingError(format!("Invalid JSON: {e}"));
            }
        }
    }

    fn ui_input_field(&mut self, ui: &mut egui::Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        let response = ui.add(
            TextEdit::multiline(&mut self.contract_json_input)
                .desired_rows(12)
                .desired_width(ui.available_width())
                .text_color(DashColors::text_primary(dark_mode))
                .background_color(DashColors::input_background(dark_mode))
                .code_editor(),
        );
        if response.changed() {
            self.parse_contract();
        }
    }

    /// The contract the Register button would send now, if any.
    fn registrable_contract(&self) -> Option<&DataContract> {
        match &self.broadcast_status {
            BroadcastStatus::ValidContract(contract) => Some(contract),
            _ => None,
        }
    }

    /// The text of the error bubble for the current status, if any.
    fn error_bubble_text(&self) -> Option<String> {
        match &self.broadcast_status {
            BroadcastStatus::ParsingError(err) => Some(format!("Parsing error: {err}")),
            BroadcastStatus::BroadcastError(msg) => Some(format!("Broadcast error: {msg}")),
            BroadcastStatus::NotRegistered(msg) => Some(msg.clone()),
            _ => None,
        }
    }

    /// Renders an error message at the top of the screen with a styled bubble
    fn render_error_bubble(&mut self, ui: &mut egui::Ui) {
        if let Some(msg) = self.error_bubble_text() {
            let dark_mode = ui.style().visuals.dark_mode;
            let error_color = DashColors::error_color(dark_mode);
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(&msg).color(error_color)).wrap());
                        ui.add_space(8.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.broadcast_status = BroadcastStatus::Idle;
                        }
                    });
                });
            ui.add_space(10.0);
        }
    }

    fn ui_parsed_contract(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut app_action = AppAction::None;

        ui.add_space(5.0);

        match &self.broadcast_status {
            BroadcastStatus::Idle => {
                ui.label("No contract parsed yet or empty input.");
            }
            BroadcastStatus::ParsingError(_)
            | BroadcastStatus::BroadcastError(_)
            | BroadcastStatus::NotRegistered(_) => {
                // Errors are now shown at the top via render_error_bubble
            }
            BroadcastStatus::ValidContract(contract) => {
                // Display estimated fee using SDK's registration_cost method
                // This accounts for document types, indexes, tokens, and keywords
                let platform_version = self.app_context.platform_version();
                let registration_fee = contract.registration_cost(platform_version).unwrap_or(0);
                // Add storage and processing fees for the contract data
                let contract_size = self.contract_json_input.len();
                let storage_fee = self
                    .app_context
                    .fee_estimator()
                    .estimate_storage_based_fee(contract_size, 20); // ~20 seeks for tree operations
                let estimated_fee = registration_fee.saturating_add(storage_fee);
                ui.add_space(10.0);
                let dark_mode = ui.style().visuals.dark_mode;
                Frame::new()
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated Fee:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format_credits_as_dash(estimated_fee))
                                    .color(DashColors::text_primary(dark_mode))
                                    .strong(),
                            );
                        });
                    });
                ui.add_space(10.0);

                // Register button
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                if ComponentStyles::add_primary_button(ui, "Register Contract").clicked() {
                    // The button is only reachable once an identity and key are
                    // selected; guard rather than unwrap so a missing selection
                    // degrades to an actionable banner instead of a panic.
                    if let (Some(qualified_identity), Some(key)) = (
                        self.selected_qualified_identity.clone(),
                        self.selected_key.clone(),
                    ) {
                        app_action = AppAction::BackendTask(BackendTask::ContractTask(Box::new(
                            ContractTask::RegisterDataContract(
                                (**contract).clone(),
                                self.contract_alias_input.clone(),
                                qualified_identity,
                                key,
                            ),
                        )));
                    } else {
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "Select an identity and key before registering the contract.",
                            MessageType::Error,
                        );
                    }
                }
            }
            BroadcastStatus::Broadcasting => {
                ui.spinner();
            }
            BroadcastStatus::ProofError => {
                ui.spinner();
            }
            BroadcastStatus::Done => {
                ui.colored_label(
                    Color32::DARK_GREEN,
                    "Data Contract registered successfully!",
                );
            }
        }

        if let AppAction::BackendTask(BackendTask::ContractTask(contract_task)) = &app_action
            && let ContractTask::RegisterDataContract(_, _, _, _) = **contract_task
        {
            self.broadcast_status = BroadcastStatus::Broadcasting;
            self.refresh_banner.take_and_clear();
            let handle =
                MessageBanner::set_global(ui.ctx(), "Broadcasting contract...", MessageType::Info);
            handle.with_elapsed();
            self.refresh_banner = Some(handle);
        }

        app_action
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let action = crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Data Contract Registered Successfully!".to_string(),
            vec![
                (
                    "Back to Contracts screen".to_string(),
                    AppAction::GoToMainScreen,
                ),
                (
                    "Register another contract".to_string(),
                    AppAction::Custom("register_another".to_string()),
                ),
            ],
            None,
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "register_another"
        {
            self.contract_json_input = String::new();
            self.contract_alias_input = String::new();
            self.broadcast_status = BroadcastStatus::Idle;
            self.completed_fee_result = None;
            return AppAction::None;
        }

        action
    }
}

impl ScreenLike for RegisterDataContractScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.refresh_banner.take_and_clear();
        }
        if message_type == MessageType::Error {
            self.broadcast_status = BroadcastStatus::BroadcastError(message.to_string());
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if !matches!(
            error,
            TaskError::ContractRegistrationUnconfirmed { .. }
                | TaskError::ContractRegistrationInProgress
                | TaskError::RegistrationNonceRefused { .. }
                | TaskError::ContractRegistrationPreempted { .. }
        ) {
            return false;
        }
        self.refresh_banner.take_and_clear();
        self.broadcast_status = BroadcastStatus::NotRegistered(error.to_string());
        true
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::FetchedNonce => {
                self.broadcast_status = BroadcastStatus::Broadcasting;
            }
            BackendTaskSuccessResult::RegisteredContract { fee_result, .. } => {
                self.refresh_banner.take_and_clear();
                self.completed_fee_result = Some(fee_result);
                self.broadcast_status = BroadcastStatus::Done;
            }
            BackendTaskSuccessResult::ProofErrorLogged => {
                self.broadcast_status = BroadcastStatus::ProofError;
            }
            BackendTaskSuccessResult::ContractSavedAfterProofError => {
                self.refresh_banner.take_and_clear();
                self.broadcast_status = BroadcastStatus::Done;
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Register Data Contract", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        action |= island_central_panel(ui, |ui| {
            if self.broadcast_status == BroadcastStatus::Done {
                return self.show_success(ui);
            }

            ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Register Data Contract");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Show error message at the top if there's an error
                self.render_error_bubble(ui);

                // If no identities loaded, give message
                if self.qualified_identities.is_empty() {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        "No identities loaded. Please load an identity first.",
                    );
                    return AppAction::None;
                }

                // Check if any identity has suitable private keys for contract registration
                let has_suitable_keys = self.qualified_identities.iter().any(|qi| {
                    qi.private_keys
                        .identity_public_keys()
                        .iter()
                        .any(|key_ref| {
                            let key = &key_ref.1.identity_public_key;
                            // Contract registration requires Authentication keys with High or Critical security level
                            key.purpose() == Purpose::AUTHENTICATION
                                && (key.security_level() == SecurityLevel::HIGH
                                    || key.security_level() == SecurityLevel::CRITICAL)
                        })
                });

                if !has_suitable_keys {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        "No identities with high or critical authentication private keys loaded. Contract registration requires high or critical security level keys.",
                    );
                    return AppAction::None;
                }

                // Select the identity to register the contract for
                ui.heading("1. Select Identity");
                ui.add_space(5.0);

                // Identity selector
                let response = ui.add(
                    IdentitySelector::new(
                        "register_contract_identity_selector",
                        &mut self.selected_identity_string,
                        &self.qualified_identities,
                    )
                    .selected_identity(&mut self.selected_qualified_identity)
                    .unwrap()
                    .width(300.0)
                    .label("Identity:")
                    .other_option(false)
                    .syncing_global(self.app_context.clone()),
                );

                // Handle identity change - auto-select key and update wallet
                if response.changed() {
                    if let Some(identity) = &self.selected_qualified_identity {
                        // Auto-select a suitable key for contract registration
                        use dash_sdk::dpp::identity::KeyType;
                        self.selected_key = identity
                            .identity
                            .get_first_public_key_matching(
                                Purpose::AUTHENTICATION,
                                [SecurityLevel::HIGH, SecurityLevel::CRITICAL].into(),
                                KeyType::all_key_types().into(),
                                false,
                            )
                            .cloned();

                        // Update wallet
                        self.selected_wallet = get_selected_wallet(
                            identity,
                            Some(&self.app_context),
                            None,
                        )
                        .or_show_error(self.app_context.egui_ctx())
                        .unwrap_or(None);
                        self.wallet_open_attempted = false;

                        // Re-parse contract with new owner ID
                        self.parse_contract();
                    } else {
                        self.selected_key = None;
                        self.selected_wallet = None;
                        self.wallet_open_attempted = false;
                    }
                }

                // Key selector (only shown in advanced mode)
                if self.show_advanced_options {
                    ui.add_space(10.0);
                    if let Some(identity) = &self.selected_qualified_identity {
                        add_key_chooser(
                            ui,
                            &self.app_context,
                            identity,
                            &mut self.selected_key,
                            TransactionType::RegisterContract,
                        );
                    }
                }

                ui.add_space(5.0);
                if let Some(identity) = &self.selected_qualified_identity {
                    ui.label(format!(
                        "Identity balance: {:.6}",
                        identity.identity.balance() as f64 * 1e-11
                    ));
                }

                if self.selected_key.is_none() {
                    return AppAction::None;
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render wallet unlock if needed
                if let Some(wallet) = &self.selected_wallet {
                    if !self.wallet_open_attempted {
                        let _ = try_open_wallet_no_password(&self.app_context, wallet)
                            .or_show_error(ui.ctx());
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
                        return AppAction::None;
                    }
                }

                // Input for the alias
                ui.heading("2. Contract alias for DET (optional)");
                ui.add_space(5.0);
                ui.text_edit_singleline(&mut self.contract_alias_input);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input for the contract
                ui.heading("3. Paste the contract JSON below");
                ui.add_space(5.0);

                ui.label(
                    "Paste a data contract JSON compatible with the current Dash Platform protocol.",
                );
                ui.add_space(5.0);

                self.ui_input_field(ui);

                // Parse the contract and show the result
                self.ui_parsed_contract(ui)
            }).inner
        });

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
mod tests {
    use super::*;

    /// A registration the network has not confirmed is shown by the screen
    /// in its own words, not as a broadcast error, and needs no extra
    /// banner; other errors keep the generic handling.
    #[test]
    fn an_unconfirmed_registration_is_not_shown_as_a_broadcast_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = RegisterDataContractScreen::new(&app_context);
        screen.broadcast_status = BroadcastStatus::Broadcasting;

        assert!(!screen.display_task_error(&TaskError::TaskManagerShuttingDown));
        assert!(screen.broadcast_status == BroadcastStatus::Broadcasting);

        let error = TaskError::ContractRegistrationUnconfirmed {
            contract_id: dash_sdk::platform::Identifier::from([0x42; 32]),
            source_error: Box::new(dash_sdk::Error::Generic("wait failed".to_string())),
        };
        assert!(screen.display_task_error(&error));
        assert_eq!(screen.error_bubble_text(), Some(error.to_string()));
    }

    /// A registration refused before it could run, or whose place another
    /// registration took, is shown in its own words, not as a broadcast error.
    #[test]
    fn a_refused_registration_is_not_shown_as_a_broadcast_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = RegisterDataContractScreen::new(&app_context);

        for error in [
            TaskError::ContractRegistrationInProgress,
            TaskError::RegistrationNonceRefused {
                source_error: Box::new(dash_sdk::Error::Generic("refused".to_string())),
            },
            TaskError::ContractRegistrationPreempted {
                contract_id: dash_sdk::platform::Identifier::from([0x42; 32]),
                source_error: Box::new(dash_sdk::Error::Generic("taken".to_string())),
            },
        ] {
            screen.broadcast_status = BroadcastStatus::Broadcasting;
            assert!(screen.display_task_error(&error), "{error:?}");
            assert_eq!(screen.error_bubble_text(), Some(error.to_string()));
        }
    }

    /// After a definite refusal the contract can be sent again with an
    /// explicit click, next to the refusal; an unconfirmed registration keeps
    /// only its guidance to check the contract ID.
    #[test]
    fn a_refused_registration_can_be_registered_again() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = RegisterDataContractScreen::new(&app_context);
        let contract_id = dash_sdk::platform::Identifier::from([0x42; 32]);
        let source = || Box::new(dash_sdk::Error::Generic("refused".to_string()));

        for error in [
            TaskError::RegistrationNonceRefused {
                source_error: source(),
            },
            TaskError::ContractRegistrationInProgress,
            TaskError::ContractRegistrationPreempted {
                contract_id,
                source_error: source(),
            },
            TaskError::ContractRegistrationReplacedByEarlier { contract_id },
        ] {
            screen.sent_contract = Some(Box::new(app_context.dpns_contract().as_ref().clone()));
            screen.broadcast_status = BroadcastStatus::Broadcasting;
            assert!(screen.display_task_error(&error), "{error:?}");
            assert_eq!(screen.error_bubble_text(), Some(error.to_string()));
            assert!(screen.registrable_contract().is_some(), "{error:?}");
        }

        screen.sent_contract = Some(Box::new(app_context.dpns_contract().as_ref().clone()));
        screen.broadcast_status = BroadcastStatus::Broadcasting;
        let unconfirmed = TaskError::ContractRegistrationUnconfirmed {
            contract_id,
            source_error: source(),
        };
        assert!(screen.display_task_error(&unconfirmed));
        assert_eq!(screen.error_bubble_text(), Some(unconfirmed.to_string()));
        assert!(screen.registrable_contract().is_none());
    }
}
