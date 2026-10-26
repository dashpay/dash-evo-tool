use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dashpay::ContactQr;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::MessageBanner;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::identity::wallet_for_signing_key;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{RichText, ScrollArea, TextEdit, Ui};
use std::sync::{Arc, RwLock};

pub struct QRScannerScreen {
    pub app_context: Arc<AppContext>,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    qr_data_input: String,
    parsed_qr_data: Option<ContactQr>,
    sending: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
}

impl QRScannerScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        let identities = app_context.load_local_user_identities().unwrap_or_default();
        let selected_identity = {
            use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
            app_context
                .selected_identity_id()
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .or_else(|| identities.first().cloned())
        };
        let selected_identity_string = selected_identity
            .as_ref()
            .map(|qi| {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                qi.identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            })
            .unwrap_or_default();

        Self {
            app_context,
            selected_identity,
            selected_identity_string,
            qr_data_input: String::new(),
            parsed_qr_data: None,
            sending: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
        }
    }

    fn parse_qr_code(&mut self) {
        if self.qr_data_input.is_empty() {
            self.parsed_qr_data = None;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please enter QR code data",
                MessageType::Error,
            );
            return;
        }

        match ContactQr::parse(&self.qr_data_input, now_secs()) {
            Ok(data) => {
                self.parsed_qr_data = Some(data);
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "QR code parsed successfully",
                    MessageType::Success,
                );
            }
            Err(error) => {
                self.parsed_qr_data = None;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    error.to_string(),
                    MessageType::Error,
                )
                .with_details(error);
            }
        }
    }

    fn send_contact_request_with_proof(&mut self) -> AppAction {
        if let Some(identity) = &self.selected_identity {
            if let Some(qr_data) = &self.parsed_qr_data {
                // Get signing key
                let signing_key = match identity
                    .dashpay_signing_key(&self.app_context.dashpay_contract(), "contactRequest")
                {
                    Some(key) => key,
                    None => {
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "This identity has no key that can send a contact request. Add a high-security authentication key to the identity and try again.",
                            MessageType::Error,
                        );
                        return AppAction::None;
                    }
                };

                self.sending = true;

                // Create task to send contact request with proof
                let task =
                    BackendTask::DashPayTask(Box::new(DashPayTask::SendContactRequestWithProof {
                        identity: identity.clone(),
                        signing_key: signing_key.clone(),
                        qr: qr_data.clone(),
                    }));

                return AppAction::BackendTask(task);
            } else {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Please parse a QR code first",
                    MessageType::Error,
                );
            }
        } else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please select an identity",
                MessageType::Error,
            );
        }

        AppAction::None
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Header
        ui.heading("Scan Contact QR Code");
        ui.add_space(10.0);

        // Identity selector
        let identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();

        if identities.is_empty() {
            action |= super::render_no_identities_card(ui, &self.app_context);
            return action;
        }

        ScrollArea::vertical().show(ui, |ui| {

            ui.group(|ui| {
                ui.label(RichText::new("1. Select Your Identity").strong());
                ui.separator();

                // Track identity before selection to detect changes
                let prev_identity_id = self.selected_identity.as_ref().map(|i| i.identity.id());

                ui.horizontal(|ui| {
                    ui.label("Identity:");
                    // SYNC: write-back via syncing_global on user pick (FR-6: the source list is
                    // User-only, so a masternode/evonode can never leak to the app-global identity).
                    ui.add(
                        IdentitySelector::new(
                            "qr_scanner_identity_selector",
                            &mut self.selected_identity_string,
                            &identities,
                        )
                        .selected_identity(&mut self.selected_identity)
                        .unwrap()
                        .width(300.0)
                        .other_option(false)
                        .syncing_global(self.app_context.clone()),
                    );
                });

                // Update wallet if identity changed
                let new_identity_id = self.selected_identity.as_ref().map(|i| i.identity.id());
                if prev_identity_id != new_identity_id {
                    if let Some(identity) = &self.selected_identity {
                        let key = identity.dashpay_signing_key(
                            &self.app_context.dashpay_contract(),
                            "contactRequest",
                        );
                        self.selected_wallet = wallet_for_signing_key(identity, key);
                    } else {
                        self.selected_wallet = None;
                    }
                    self.wallet_open_attempted = false;
                }
            });

            ui.add_space(20.0);

            ui.group(|ui| {
                ui.label(RichText::new("2. Enter QR Code Data").strong());
                ui.separator();

                ui.label(RichText::new("Paste the QR code data below:").small());

                ui.add(
                    TextEdit::multiline(&mut self.qr_data_input)
                        .hint_text("dash:?du=...&dapk=...")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                );

                ui.horizontal(|ui| {
                    if ui.button("Parse QR Code").clicked() {
                        self.parse_qr_code();
                    }

                    if ui.button("Clear").clicked() {
                        self.qr_data_input.clear();
                        self.parsed_qr_data = None;
                    }
                });
            });

            ui.add_space(20.0);

            // Display parsed QR data
            if let Some(qr_data) = self.parsed_qr_data.clone() {
                ui.group(|ui| {
                    ui.label(RichText::new("3. QR Code Details").strong());
                    ui.separator();

                    egui::Grid::new("qr_details_grid")
                        .num_columns(2)
                        .spacing([10.0, 5.0])
                        .show(ui, |ui| {
                            ui.label("Contact:");
                            ui.label(qr_data.dpns_name());
                            ui.end_row();

                            ui.label("Expires:");
                            ui.label(crate::model::datetime::local_date_time_from_unix_millis(
                                u64::from(qr_data.expires_at()) * 1000,
                            ));
                            ui.end_row();
                        });

                    ui.add_space(10.0);

                    // Check wallet lock status before showing send button
                    let wallet_locked = if let Some(wallet) = &self.selected_wallet {
                        if !self.wallet_open_attempted {
                            if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                                MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                                    .disable_auto_dismiss();
                            }
                            self.wallet_open_attempted = true;
                        }
                        wallet_needs_unlock(wallet)
                    } else {
                        false
                    };

                    if wallet_locked {
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to add contact.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                    } else {
                        ui.horizontal(|ui| {
                            if self.sending {
                                ui.spinner();
                                ui.label("Sending contact request...");
                            } else if ui.button("Add Contact").clicked() {
                                action = self.send_contact_request_with_proof();
                            }
                        });
                    }

                    ui.add_space(10.0);

                    ui.label(RichText::new("ℹ️ This sends a contact request that your contact's DashPay wallet can accept automatically.").small());
                });
            }

            ui.add_space(20.0);

            // Information box
            ui.group(|ui| {
                ui.label(RichText::new("ℹ️ About QR Code Scanning").strong());
                ui.separator();
                ui.label("• A contact QR code names your contact's username");
                ui.label("• Your contact's DashPay wallet can accept a request sent from it automatically");
                ui.label("• A contact QR code works for one hour after it is created");
            });
        });

        action
    }

    pub fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
    }

    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.sending = false;
        if let BackendTaskSuccessResult::DashPayContactRequestSent(_)
        | BackendTaskSuccessResult::DashPayContactAlreadyEstablished { .. } = result
        {
            // Clear the form on success
            self.qr_data_input.clear();
            self.parsed_qr_data = None;
        }
    }
}

impl ScreenLike for QRScannerScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("Scan QR Code", AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenDashpay);

        // Add DashPay subscreen chooser panel
        action |=
            add_dashpay_subscreen_chooser_panel(ui, &self.app_context, DashPaySubscreen::Contacts);

        // Main content area with island styling
        action |= island_central_panel(ui, |ui| self.render(ui));

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully, UI will update on next frame
            }
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; no side-effects needed.
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.display_task_result(result);
    }
}

/// The wall clock in Unix seconds, the clock a contact QR code's expiry is
/// compared against.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}
