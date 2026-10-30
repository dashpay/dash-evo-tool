use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::MessageBanner;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::qr_code_image::QrCodeImage;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::state::PendingRequest;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{RichText, ScrollArea, Ui};
use std::sync::{Arc, RwLock};

const QR_CODE_INFO_TEXT: &str = "About Contact QR Codes:\n\n\
    Someone who scans your QR code sends you a contact request that DashPay wallets can accept automatically.\n\n\
    The QR code shows your username and works for one hour.\n\n\
    WARNING: Anyone who has this QR code before it expires can send you such a request. Share it only with people you want as contacts.";

/// The wallet whose recovery phrase derives `identity`'s contact QR code key,
/// for the unlock prompt.
fn qr_wallet(identity: &QualifiedIdentity) -> Option<Arc<RwLock<Wallet>>> {
    identity.dashpay_wallet().map(|(_, wallet)| wallet.clone())
}

pub struct QRCodeGeneratorScreen {
    pub app_context: Arc<AppContext>,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    generated_qr_data: Option<String>,
    /// The outstanding generate request and the identity it is for. Only its
    /// result is shown, and only while that identity is still selected.
    qr_request: PendingRequest<dash_sdk::platform::Identifier>,
    qr_code: Option<QrCodeImage>,
    show_info_popup: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
}

impl QRCodeGeneratorScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        let mut new_self = Self {
            app_context: app_context.clone(),
            selected_identity: None,
            selected_identity_string: String::new(),
            generated_qr_data: None,
            qr_request: PendingRequest::default(),
            qr_code: None,
            show_info_popup: false,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
        };

        // Seed from the app-scoped selected identity (W3 SYNC); fall back to first.
        if let Ok(identities) = app_context.load_local_user_identities()
            && !identities.is_empty()
        {
            use dash_sdk::dpp::platform_value::string_encoding::Encoding;

            let selected_id = app_context.selected_identity_id();
            let preferred = selected_id
                .and_then(|id| identities.iter().find(|qi| qi.identity.id() == id).cloned())
                .unwrap_or_else(|| identities[0].clone());

            new_self.selected_identity = Some(preferred.clone());
            new_self.selected_identity_string = preferred.identity.id().to_string(Encoding::Base58);

            // Get wallet for the selected identity
            new_self.selected_wallet = qr_wallet(&preferred);
        }

        new_self
    }

    /// React to a new `selected_identity`: follow its wallet and drop the QR
    /// code shown for the previous one.
    fn identity_changed(&mut self) {
        self.selected_wallet = self.selected_identity.as_ref().and_then(qr_wallet);
        self.wallet_open_attempted = false;
        self.generated_qr_data = None;
        self.qr_request.clear();
    }

    /// Dispatch the auto-accept QR build to the backend, which resolves the key
    /// and derives the proof through the JIT chokepoint (no seed in the UI).
    fn generate_qr_code(&mut self) -> AppAction {
        let Some(identity) = &self.selected_identity else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please select an identity first",
                MessageType::Error,
            );
            return AppAction::None;
        };

        let identity_id = identity.identity.id();
        self.qr_request.dispatch_with(
            BackendTask::DashPayTask(Box::new(DashPayTask::GenerateAutoAcceptQrCode {
                identity: identity.clone(),
            })),
            identity_id,
        )
    }

    pub fn render(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

        // Header with info icon
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                action = AppAction::PopScreen;
            }
            ui.heading("Generate Contact QR Code");
            ui.add_space(5.0);
            if crate::ui::helpers::info_icon_button(ui, QR_CODE_INFO_TEXT).clicked() {
                self.show_info_popup = true;
            }
        });

        ui.separator();

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
                ui.label(
                    RichText::new("Configuration")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.separator();

                egui::Grid::new("qr_config_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Identity:").color(DashColors::text_primary(dark_mode)),
                        );
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            // SYNC: write-back via syncing_global on user pick (FR-6: the source list is
                    // User-only, so a masternode/evonode can never leak to the app-global identity).
                            let response = ui.add(
                                IdentitySelector::new(
                                    "qr_identity_selector",
                                    &mut self.selected_identity_string,
                                    &identities,
                                )
                                    .selected_identity(&mut self.selected_identity)
                                    .unwrap()
                                    .width(300.0)
                                    .other_option(false)
                                    .syncing_global(self.app_context.clone()),
                            );

                            if response.changed() {
                                self.identity_changed();
                            }
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);

                // Check wallet lock status before showing generate button
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
                    ui.add_space(10.0);
                    ui.colored_label(
                        DashColors::warning_color(dark_mode),
                        "Wallet is locked. Please unlock to generate QR code.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui.button("Generate QR Code").clicked() {
                            action |= self.generate_qr_code();
                        }

                        if self.generated_qr_data.is_some()
                            && ui.button("Clear").clicked() {
                                self.generated_qr_data = None;
                            }
                    });
                }
            });

            ui.add_space(20.0);

            // Display generated QR data
            let mut show_copied_message = false;
            if let Some(qr_data) = &self.generated_qr_data {
                ui.group(|ui| {
                    ui.label(
                        RichText::new("Generated QR Code")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.separator();

                    // Center the QR code
                    ui.vertical_centered(|ui| {
                        self.qr_code
                            .get_or_insert_with(|| {
                                QrCodeImage::new().with_natural_size().with_fallback_text(
                                    "The QR code could not be created. Copy the data below instead.",
                                )
                            })
                            .show(ui, qr_data);
                    });

                    ui.add_space(10.0);

                    // Show the text data in a collapsible section
                    ui.collapsing("QR Code Data (text)", |ui| {
                        ui.code(qr_data);
                    });

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        let copy_text = qr_data.clone();
                        if ui.button("Copy Data to Clipboard").clicked() {
                            ui.ctx().copy_text(copy_text);
                            show_copied_message = true;
                        }
                    });

                    ui.add_space(10.0);

                    ui.label(
                        RichText::new(
                            "Share this QR code with someone to establish a mutual contact",
                        )
                        .small()
                        .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(
                            "WARNING: Anyone with this QR code can automatically become your contact",
                        )
                        .small()
                        .color(DashColors::warning_color(dark_mode)),
                    );
                });
            }

            if show_copied_message {
                MessageBanner::set_global(ui.ctx(), "Copied to clipboard", MessageType::Success);
            }
        });

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ui.ctx(), wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully, UI will update on next frame
            }
        }

        action
    }

    pub fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
    }
}

impl ScreenLike for QRCodeGeneratorScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel
        action |= add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("DashPay", AppAction::None),
                ("QR Generator", AppAction::None),
            ],
            vec![],
        );

        // Highlight DashPay in the main left panel
        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenDashpay);

        // Add DashPay subscreen chooser panel
        action |= add_dashpay_subscreen_chooser_panel(
            ui,
            &self.app_context,
            DashPaySubscreen::Contacts, // Use Contacts as the active subscreen since QR Generator is launched from there
        );

        // Main content area with island styling
        action |= island_central_panel(ui, |ui| self.render(ui));

        // Show info popup if requested
        if self.show_info_popup {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("dashpay_contact_qr_code_info_popup"),
                        "About Contact QR Codes",
                        QR_CODE_INFO_TEXT,
                    );
                    if popup.show(ui).inner {
                        self.show_info_popup = false;
                    }
                });
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Banner display is handled globally by AppState; no side-effects needed.
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        let BackendTaskSuccessResult::DashPayAutoAcceptQrCode(qr_string) = result else {
            return;
        };
        // A superseded request, or one for an identity no longer selected,
        // would show another identity's contact code under this one.
        let Some(requested_for) = self.qr_request.settle(context) else {
            return;
        };
        if self
            .selected_identity
            .as_ref()
            .is_some_and(|identity| identity.identity.id() == requested_for)
        {
            self.generated_qr_data = Some(qr_string);
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.qr_request.settle(context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTaskContext;
    use crate::context::test_support::bare_user_identity;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::platform::Identifier;

    /// The context the screen's generate request is reported under.
    fn dispatch_context(action: AppAction) -> BackendTaskContext {
        match action {
            AppAction::BackendTaskWithContext { context, .. } => context,
            _ => BackendTaskContext::Unknown,
        }
    }

    fn qr_result(qr: &str) -> BackendTaskSuccessResult {
        BackendTaskSuccessResult::DashPayAutoAcceptQrCode(qr.to_string())
    }

    /// Regression: a QR code generated for one identity arrived after the user
    /// had picked another and was shown under it, handing out the first
    /// identity's contact code as if it were the second's.
    #[test]
    fn a_qr_code_for_a_previously_selected_identity_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = QRCodeGeneratorScreen::new(context);
        let first = bare_user_identity(Identifier::new([1; 32]), Network::Testnet);
        let second = bare_user_identity(Identifier::new([2; 32]), Network::Testnet);

        screen.selected_identity = Some(first);
        let for_first = dispatch_context(screen.generate_qr_code());
        screen.selected_identity = Some(second);
        screen.identity_changed();
        screen.display_backend_task_result(&for_first, qr_result("first"));
        assert_eq!(screen.generated_qr_data, None, "the identity changed");

        // A newer request for the second identity supersedes the first one.
        let for_second = dispatch_context(screen.generate_qr_code());
        screen.display_backend_task_result(&for_first, qr_result("first"));
        assert_eq!(screen.generated_qr_data, None, "a superseded request");
        screen.display_backend_task_result(&for_second, qr_result("second"));
        assert_eq!(screen.generated_qr_data.as_deref(), Some("second"));
    }
}
