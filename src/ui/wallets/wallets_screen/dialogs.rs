use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::Amount;
use crate::model::secret::Secret;
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::address_input::AddressInput;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::helpers::clicked_outside_window;
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::{ComponentStyles, DashColors};
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::{self, ComboBox, Context};
use eframe::epaint::TextureHandle;
use egui::load::SizedTexture;
use egui::{Frame, Margin, RichText, TextureOptions};
use std::sync::{Arc, RwLock};

use super::WalletsBalancesScreen;

#[derive(Default)]
pub(super) struct SendDialogState {
    pub is_open: bool,
    pub address: String,
    pub address_error: Option<String>,
    pub amount: Option<Amount>,
    pub amount_input: Option<AmountInput>,
    pub subtract_fee: bool,
    pub memo: String,
    pub error: Option<String>,
}

/// Type of address to receive to
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReceiveAddressType {
    /// Core (L1) address for receiving Dash
    #[default]
    Core,
    /// Platform address for receiving credits
    Platform,
}

/// Unified state for the receive dialog (Core and Platform)
#[derive(Default)]
pub(super) struct ReceiveDialogState {
    pub is_open: bool,
    /// Selected address type (Core or Platform)
    pub address_type: ReceiveAddressType,
    /// Core addresses with balances: (address, balance_duffs)
    pub core_addresses: Vec<(String, u64)>,
    /// Currently selected Core address index
    pub selected_core_index: usize,
    /// Platform addresses with balances: (display_address, balance_credits)
    pub platform_addresses: Vec<(String, u64)>,
    /// Currently selected Platform address index
    pub selected_platform_index: usize,
    pub qr_texture: Option<TextureHandle>,
    pub qr_address: Option<String>,
    pub status: Option<String>,
}

/// State for the Fund Platform Address from Asset Lock dialog
#[derive(Default)]
pub(super) struct FundPlatformAddressDialogState {
    pub is_open: bool,
    /// Selected asset lock txid (as byte array)
    pub selected_asset_lock_txid: Option<[u8; 32]>,
    /// Selected Platform address to fund
    pub selected_platform_address: Option<String>,
    /// List of Platform addresses available
    pub platform_addresses: Vec<(String, u64)>,
    pub status: Option<String>,
    /// Whether the current status is an error message
    pub status_is_error: bool,
    pub is_processing: bool,
    /// Whether we should continue funding after the wallet is unlocked
    pub pending_fund_after_unlock: bool,
}

/// State for the Mine Blocks dialog (dev mode, Regtest/Devnet only)
#[derive(Default)]
pub(super) struct MineDialogState {
    pub is_open: bool,
    pub address_input: Option<AddressInput>,
    pub validated_address: Option<ValidatedAddress>,
    pub block_count_str: String,
    pub error: Option<String>,
}

/// State for the Private Key dialog
#[derive(Default)]
pub(super) struct PrivateKeyDialogState {
    pub is_open: bool,
    /// The address being displayed
    pub address: String,
    /// The private key in WIF format
    pub private_key_wif: Secret,
    /// Whether to show the private key (hidden by default)
    pub show_key: bool,
    /// Pending derivation path (when wallet needs unlock first)
    pub pending_derivation_path: Option<DerivationPath>,
    /// Pending address string (when wallet needs unlock first)
    pub pending_address: Option<String>,
}

impl WalletsBalancesScreen {
    pub(super) fn draw_modal_overlay(ctx: &Context, id: &str) {
        let screen_rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new(id),
        ));
        painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());
    }

    pub(super) fn modal_frame(ctx: &Context) -> Frame {
        Frame {
            inner_margin: egui::Margin::same(20),
            outer_margin: egui::Margin::same(0),
            corner_radius: egui::CornerRadius::same(8),
            shadow: egui::epaint::Shadow {
                offset: [0, 8],
                blur: 16,
                spread: 0,
                color: DashColors::popup_shadow(),
            },
            fill: ctx.style().visuals.window_fill,
            stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
        }
    }

    pub(super) fn render_send_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.send_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.send_dialog.is_open;

        // Draw dark overlay behind the dialog
        Self::draw_modal_overlay(ctx, "send_dialog_overlay");

        egui::Window::new("Send Dash")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Recipient Address");
                let hint = if self.app_context.network == Network::Mainnet {
                    "Enter Core address (X.../7...)"
                } else {
                    "Enter Core address (y.../8...)"
                };
                let response = ui
                    .add(egui::TextEdit::singleline(&mut self.send_dialog.address).hint_text(hint));

                // Validate address when it changes
                if response.changed() {
                    if self.send_dialog.address.trim().is_empty() {
                        self.send_dialog.address_error = None;
                    } else {
                        let trimmed = self.send_dialog.address.trim();
                        if crate::ui::helpers::is_platform_address_string(trimmed) {
                            self.send_dialog.address_error = Some(
                                "Platform addresses not supported. Use a Core address.".to_string(),
                            );
                        } else {
                            match trimmed.parse::<Address<NetworkUnchecked>>() {
                                Ok(_) => {
                                    self.send_dialog.address_error = None;
                                }
                                Err(_) => {
                                    self.send_dialog.address_error =
                                        Some("Invalid Core address".to_string());
                                }
                            }
                        }
                    }
                }

                if let Some(error) = &self.send_dialog.address_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
                }

                ui.add_space(8.0);

                // Amount input using AmountInput component
                let amount_input = self.send_dialog.amount_input.get_or_insert_with(|| {
                    AmountInput::new(Amount::new_dash(0.0))
                        .with_label("Amount (DASH):")
                        .with_hint_text("Enter amount (e.g., 0.01)")
                        .with_desired_width(150.0)
                });

                let response = amount_input.show(ui);
                response.inner.update(&mut self.send_dialog.amount);

                ui.checkbox(
                    &mut self.send_dialog.subtract_fee,
                    "Subtract fee from amount",
                );

                ui.label("Memo (optional)");
                ui.add(egui::TextEdit::singleline(&mut self.send_dialog.memo));

                if let Some(error) = self.send_dialog.error.clone() {
                    let error_color = DashColors::ERROR;
                    Frame::new()
                        .fill(error_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, error_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Error: {}", error)).color(error_color),
                                );
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.send_dialog.error = None;
                                }
                            });
                        });
                }

                ui.add_space(8.0);
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.horizontal(|ui| {
                    let has_address_error = self.send_dialog.address_error.is_some();
                    if ComponentStyles::add_primary_button_enabled(ui, !has_address_error, "Send")
                        .clicked()
                    {
                        match self.prepare_send_action() {
                            Ok(app_action) => {
                                action = app_action;
                                self.send_dialog = SendDialogState::default();
                            }
                            Err(err) => self.send_dialog.error = Some(err),
                        }
                    }
                    if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked() {
                        self.send_dialog = SendDialogState::default();
                    }
                });
            });

        self.send_dialog.is_open = open;
        action
    }

    pub(super) fn render_receive_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.receive_dialog.is_open {
            return AppAction::None;
        }

        // Refresh cached balances from the wallet so SPV updates are reflected
        if let Some(wallet) = &self.selected_wallet
            && let Ok(wallet_guard) = wallet.read()
        {
            use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
            for (addr_str, balance) in &mut self.receive_dialog.core_addresses {
                if let Ok(addr) = addr_str.parse::<Address<NetworkUnchecked>>()
                    && let Ok(addr) = addr.require_network(self.app_context.network)
                {
                    *balance = wallet_guard.address_balance(&addr);
                }
            }
        }

        let dark_mode = ctx.style().visuals.dark_mode;

        // Determine current address based on selected type
        let current_address = match self.receive_dialog.address_type {
            ReceiveAddressType::Core => self
                .receive_dialog
                .core_addresses
                .get(self.receive_dialog.selected_core_index)
                .map(|(addr, _)| addr.clone()),
            ReceiveAddressType::Platform => self
                .receive_dialog
                .platform_addresses
                .get(self.receive_dialog.selected_platform_index)
                .map(|(addr, _)| addr.clone()),
        };

        // Generate QR texture if needed
        if let Some(address) = current_address.clone() {
            let needs_texture = self.receive_dialog.qr_texture.is_none()
                || self.receive_dialog.qr_address.as_deref() != Some(&address);
            if needs_texture {
                match generate_qr_code_image(&address) {
                    Ok(image) => {
                        let texture = ctx.load_texture(
                            format!("receive_{}", address),
                            image,
                            TextureOptions::LINEAR,
                        );
                        self.receive_dialog.qr_texture = Some(texture);
                        self.receive_dialog.qr_address = Some(address);
                    }
                    Err(err) => {
                        self.receive_dialog.status = Some(err.to_string());
                    }
                }
            }
        }

        let mut open = self.receive_dialog.is_open;

        // Draw dark overlay behind the dialog (only when open)
        if open {
            Self::draw_modal_overlay(ctx, "receive_dialog_overlay");
        }

        let window_response = egui::Window::new("Receive")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(Self::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_min_width(350.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);

                    // Address type selector at the top
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.receive_dialog.address_type,
                            ReceiveAddressType::Core,
                            RichText::new("Core").color(DashColors::text_primary(dark_mode)),
                        );
                        ui.selectable_value(
                            &mut self.receive_dialog.address_type,
                            ReceiveAddressType::Platform,
                            RichText::new("Platform").color(DashColors::text_primary(dark_mode)),
                        );
                    });

                    // Clear QR when switching types
                    let type_label = match self.receive_dialog.address_type {
                        ReceiveAddressType::Core => "Core Address",
                        ReceiveAddressType::Platform => "Platform Address",
                    };

                    ui.add_space(5.0);
                    ui.label(
                        RichText::new(type_label)
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(10.0);

                    // Show QR code
                    if let Some(texture) = &self.receive_dialog.qr_texture {
                        ui.image(SizedTexture::new(texture.id(), egui::vec2(220.0, 220.0)));
                    } else if current_address.is_some() {
                        ui.label("Generating QR code...");
                    }

                    ui.add_space(10.0);

                    match self.receive_dialog.address_type {
                        ReceiveAddressType::Core => {
                            // Core address selector (if multiple addresses)
                            if self.receive_dialog.core_addresses.len() > 1 {
                                ui.horizontal(|ui| {
                                    ui.label("Address:");
                                    ComboBox::from_id_salt("core_addr_selector")
                                        .selected_text(
                                            self.receive_dialog
                                                .core_addresses
                                                .get(self.receive_dialog.selected_core_index)
                                                .map(|(addr, balance)| {
                                                    let balance_dash = *balance as f64 / 1e8;
                                                    format!(
                                                        "{}... ({:.4} DASH)",
                                                        &addr[..12.min(addr.len())],
                                                        balance_dash
                                                    )
                                                })
                                                .unwrap_or_default(),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (idx, (addr, balance)) in
                                                self.receive_dialog.core_addresses.iter().enumerate()
                                            {
                                                let balance_dash = *balance as f64 / 1e8;
                                                let label = format!(
                                                    "{}... ({:.4} DASH)",
                                                    &addr[..12.min(addr.len())],
                                                    balance_dash
                                                );
                                                if ui
                                                    .selectable_label(
                                                        idx == self.receive_dialog.selected_core_index,
                                                        label,
                                                    )
                                                    .clicked()
                                                {
                                                    self.receive_dialog.selected_core_index = idx;
                                                    // Clear QR so it regenerates
                                                    self.receive_dialog.qr_texture = None;
                                                    self.receive_dialog.qr_address = None;
                                                }
                                            }
                                        });
                                });
                                ui.add_space(5.0);
                            }

                            // Show selected Core address
                            if let Some((address, balance)) = self
                                .receive_dialog
                                .core_addresses
                                .get(self.receive_dialog.selected_core_index)
                                .cloned()
                            {
                                ui.label(
                                    RichText::new(&address)
                                        .monospace()
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                let balance_dash = balance as f64 / 1e8;
                                ui.label(
                                    RichText::new(format!("Balance: {:.8} DASH", balance_dash))
                                        .color(DashColors::text_secondary(dark_mode)),
                                );

                                ui.add_space(8.0);

                                let mut copy_status: Option<String> = None;
                                let mut generate_new = false;

                                ui.horizontal(|ui| {
                                    if ComponentStyles::add_primary_button(ui, "Copy Address")
                                        .clicked()
                                    {
                                        if let Err(err) = copy_text_to_clipboard(&address) {
                                            copy_status = Some(format!("Error: {}", err));
                                        } else {
                                            copy_status = Some("Address copied!".to_string());
                                        }
                                    }

                                    if ComponentStyles::add_secondary_button(
                                        ui,
                                        "New Address",
                                        dark_mode,
                                    )
                                    .clicked()
                                    {
                                        generate_new = true;
                                    }
                                });

                                if let Some(status) = copy_status {
                                    self.receive_dialog.status = Some(status);
                                }

                                if generate_new
                                    && let Some(wallet) = &self.selected_wallet {
                                        match self.generate_new_core_receive_address(wallet) {
                                            Ok((new_addr, new_balance)) => {
                                                self.receive_dialog.core_addresses.push((new_addr, new_balance));
                                                self.receive_dialog.selected_core_index =
                                                    self.receive_dialog.core_addresses.len() - 1;
                                                self.receive_dialog.qr_texture = None;
                                                self.receive_dialog.qr_address = None;
                                                self.receive_dialog.status = Some("New address generated!".to_string());
                                            }
                                            Err(err) => {
                                                self.receive_dialog.status = Some(err);
                                            }
                                        }
                                    }
                            }

                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Send Dash to this address to add funds to your wallet.")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(11.0)
                                    .italics(),
                            );
                        }
                        ReceiveAddressType::Platform => {
                            // Platform address selector (if multiple addresses)
                            if self.receive_dialog.platform_addresses.len() > 1 {
                                ui.horizontal(|ui| {
                                    ui.label("Address:");
                                    ComboBox::from_id_salt("platform_addr_selector")
                                        .selected_text(
                                            self.receive_dialog
                                                .platform_addresses
                                                .get(self.receive_dialog.selected_platform_index)
                                                .map(|(addr, balance)| {
                                                    let credits_as_dash =
                                                        *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                                    format!(
                                                        "{}... ({:.4} DASH)",
                                                        &addr[..12.min(addr.len())],
                                                        credits_as_dash
                                                    )
                                                })
                                                .unwrap_or_default(),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (idx, (addr, balance)) in
                                                self.receive_dialog.platform_addresses.iter().enumerate()
                                            {
                                                let credits_as_dash =
                                                    *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                                let label = format!(
                                                    "{}... ({:.4} DASH)",
                                                    &addr[..12.min(addr.len())],
                                                    credits_as_dash
                                                );
                                                if ui
                                                    .selectable_label(
                                                        idx == self.receive_dialog.selected_platform_index,
                                                        label,
                                                    )
                                                    .clicked()
                                                {
                                                    self.receive_dialog.selected_platform_index = idx;
                                                    // Clear QR so it regenerates
                                                    self.receive_dialog.qr_texture = None;
                                                    self.receive_dialog.qr_address = None;
                                                }
                                            }
                                        });
                                });
                                ui.add_space(5.0);
                            }

                            // Show selected Platform address
                            let selected_addr_data = self
                                .receive_dialog
                                .platform_addresses
                                .get(self.receive_dialog.selected_platform_index)
                                .cloned();

                            if let Some((address, balance)) = selected_addr_data {
                                ui.label(
                                    RichText::new(&address)
                                        .monospace()
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                let credits_as_dash = balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.label(
                                    RichText::new(format!("Balance: {:.8} DASH", credits_as_dash))
                                        .color(DashColors::text_secondary(dark_mode)),
                                );

                                ui.add_space(8.0);

                                let mut copy_status: Option<String> = None;
                                let mut new_addr_result: Option<Result<String, String>> = None;

                                ui.horizontal(|ui| {
                                    if ComponentStyles::add_primary_button(ui, "Copy Address")
                                        .clicked()
                                    {
                                        if let Err(err) = copy_text_to_clipboard(&address) {
                                            copy_status = Some(format!("Error: {}", err));
                                        } else {
                                            copy_status = Some("Address copied!".to_string());
                                        }
                                    }

                                    // Button to add new Platform address
                                    if let Some(wallet) = &self.selected_wallet
                                        && ComponentStyles::add_secondary_button(
                                            ui,
                                            "New Address",
                                            dark_mode,
                                        )
                                        .clicked()
                                    {
                                        new_addr_result = Some(self.generate_platform_address(wallet));
                                    }
                                });

                                // Handle copy status after the closure
                                if let Some(status) = copy_status {
                                    self.receive_dialog.status = Some(status);
                                }

                                // Handle new address generation after the closure
                                if let Some(result) = new_addr_result {
                                    match result {
                                        Ok(new_addr) => {
                                            self.receive_dialog.platform_addresses.push((new_addr, 0));
                                            self.receive_dialog.selected_platform_index =
                                                self.receive_dialog.platform_addresses.len() - 1;
                                            self.receive_dialog.qr_texture = None;
                                            self.receive_dialog.qr_address = None;
                                            self.receive_dialog.status =
                                                Some("New address generated!".to_string());
                                        }
                                        Err(err) => {
                                            self.receive_dialog.status = Some(err);
                                        }
                                    }
                                }
                            }

                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(
                                    "Send credits from an identity or another Platform address to fund this address.",
                                )
                                .color(DashColors::text_secondary(dark_mode))
                                .size(11.0)
                                .italics(),
                            );
                        }
                    }

                    if let Some(status) = &self.receive_dialog.status {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(status).color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                });
            });

        if let Some(ref resp) = window_response
            && clicked_outside_window(ctx, resp.response.rect)
        {
            open = false;
        }

        self.receive_dialog.is_open = open;
        if !self.receive_dialog.is_open {
            self.receive_dialog = ReceiveDialogState::default();
        }
        AppAction::None
    }

    /// Generate a new Platform address for the wallet.
    /// Returns the address in Bech32m format (e.g., tdash1k... for testnet per DIP-18)
    pub(super) fn generate_platform_address(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<String, String> {
        use crate::model::wallet::{
            DerivationPathHelpers, DerivationPathReference, DerivationPathType,
        };
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
        use dash_sdk::dpp::key_wallet::bip32::DerivationPath;

        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
        let pw = wallet_guard
            .platform_wallet
            .as_ref()
            .ok_or_else(|| "Wallet is locked".to_string())?;
        let network = self.app_context.network;

        // Find the highest existing platform payment address index
        let info = pw.state_blocking();
        let existing_indices: Vec<u32> =
            crate::platform_wallet_bridge::CoreAddressInfo::all_from_wallet_info(&info.core_wallet)
                .iter()
                .filter(|a| a.derivation_path.is_platform_payment(network))
                .filter_map(|a| {
                    use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
                    a.derivation_path
                        .as_ref()
                        .last()
                        .and_then(|child| match child {
                            ChildNumber::Normal { index } | ChildNumber::Hardened { index } => {
                                Some(*index)
                            }
                            _ => None,
                        })
                })
                .collect();

        let next_index = existing_indices.iter().max().map(|m| m + 1).unwrap_or(0);

        // Derive a new platform payment address
        let seed = *wallet_guard.seed_bytes().map_err(|e| e.to_string())?;
        let secp = Secp256k1::new();
        let derivation_path = DerivationPath::platform_payment_path(network, 0, 0, next_index);
        let extended_private_key = derivation_path
            .derive_priv_ecdsa_for_master_seed(&seed, network)
            .map_err(|e| e.to_string())?;
        let private_key = extended_private_key.to_priv();
        let public_key = private_key.public_key(&secp);
        let address = dash_sdk::dpp::dashcore::Address::p2pkh(&public_key, network);

        // Persist to DB
        let canonical = Wallet::canonical_address(&address, network);
        self.app_context
            .db
            .add_address_if_not_exists(
                &wallet_guard.seed_hash(),
                &canonical,
                &network,
                &derivation_path,
                DerivationPathReference::PlatformPayment,
                DerivationPathType::CLEAR_FUNDS,
                None,
            )
            .map_err(|e| e.to_string())?;

        // Convert to PlatformAddress and encode as Bech32m per DIP-18
        let platform_addr =
            PlatformAddress::try_from(address).map_err(|e| format!("Invalid address: {}", e))?;
        Ok(platform_addr.to_bech32m_string(network))
    }

    /// Generate a new Core receive address for the wallet
    /// Returns (address_string, balance_duffs)
    pub(super) fn generate_new_core_receive_address(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<(String, u64), String> {
        let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
        let address = wallet_guard
            .receive_address(self.app_context.network, true, Some(&self.app_context))
            .map_err(|e| e.to_string())?;
        let balance = wallet_guard.address_balance(&address);
        Ok((address.to_string(), balance))
    }

    /// Render the Fund Platform Address from Asset Lock dialog
    pub(super) fn render_fund_platform_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.fund_platform_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.fund_platform_dialog.is_open;
        let dark_mode = ctx.style().visuals.dark_mode;

        // Draw dark overlay behind the popup
        Self::draw_modal_overlay(ctx, "fund_platform_dialog_overlay");

        let window_response = egui::Window::new("Fund Platform Address from Asset Lock")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(Self::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_min_width(400.0);

                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Select a Platform address to fund:")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);

                    // Platform address selector
                    if self.fund_platform_dialog.platform_addresses.is_empty() {
                        ui.label(
                            RichText::new("No Platform addresses found. Generate one first.")
                                .color(DashColors::text_secondary(dark_mode))
                                .italics(),
                        );
                    } else {
                        ComboBox::from_id_salt("fund_platform_addr_selector")
                            .selected_text(
                                self.fund_platform_dialog
                                    .selected_platform_address
                                    .as_deref()
                                    .map(|addr| {
                                        let balance = self
                                            .fund_platform_dialog
                                            .platform_addresses
                                            .iter()
                                            .find(|(a, _)| a == addr)
                                            .map(|(_, b)| *b)
                                            .unwrap_or(0);
                                        let credits_as_dash =
                                            balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                        format!(
                                            "{}... ({:.4} DASH)",
                                            &addr[..12.min(addr.len())],
                                            credits_as_dash
                                        )
                                    })
                                    .unwrap_or_else(|| "Select an address".to_string()),
                            )
                            .show_ui(ui, |ui| {
                                for (addr, balance) in &self.fund_platform_dialog.platform_addresses
                                {
                                    let credits_as_dash =
                                        *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                    let label = format!(
                                        "{}... ({:.4} DASH)",
                                        &addr[..12.min(addr.len())],
                                        credits_as_dash
                                    );
                                    let is_selected = self
                                        .fund_platform_dialog
                                        .selected_platform_address
                                        .as_deref()
                                        == Some(addr.as_str());
                                    if ui.selectable_label(is_selected, label).clicked() {
                                        self.fund_platform_dialog.selected_platform_address =
                                            Some(addr.clone());
                                    }
                                }
                            });
                    }

                    ui.add_space(15.0);

                    // Status message
                    if let Some(status) = &self.fund_platform_dialog.status {
                        let status_color = if self.fund_platform_dialog.status_is_error {
                            DashColors::DANGER_RED
                        } else {
                            DashColors::text_secondary(dark_mode)
                        };
                        ui.label(RichText::new(status).color(status_color));
                        ui.add_space(10.0);
                    }

                    // Buttons
                    ui.horizontal(|ui| {
                        let can_fund = self.fund_platform_dialog.selected_platform_address.is_some()
                            && self.fund_platform_dialog.selected_asset_lock_txid.is_some()
                            && !self.fund_platform_dialog.is_processing;

                        // Cancel button
                        if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode)
                            .clicked()
                        {
                            self.fund_platform_dialog.is_open = false;
                        }

                        ui.add_space(8.0);

                        // Fund button
                        let fund_label = if self.fund_platform_dialog.is_processing {
                            "Funding..."
                        } else {
                            "Fund Address"
                        };
                        let fund_button = ComponentStyles::primary_button(fund_label)
                            .fill(if can_fund {
                                ComponentStyles::primary_button_fill()
                            } else {
                                DashColors::text_secondary(dark_mode)
                            });

                        if ui
                            .add_enabled(can_fund, fund_button)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            // Check if wallet is locked
                            let is_locked = self
                                .selected_wallet
                                .as_ref()
                                .and_then(|w| w.read().ok())
                                .map(|w| !w.is_open())
                                .unwrap_or(false);

                            if is_locked {
                                // Wallet is locked - open unlock popup and set pending flag
                                self.fund_platform_dialog.pending_fund_after_unlock = true;
                                self.wallet_unlock_popup.open();
                            } else {
                                action = self.prepare_fund_platform_action();
                            }
                        }
                    });

                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(
                            "The entire asset lock amount will be used to fund the Platform address.",
                        )
                        .color(DashColors::text_secondary(dark_mode))
                        .size(11.0)
                        .italics(),
                    );
                });
            });

        if let Some(ref resp) = window_response
            && clicked_outside_window(ctx, resp.response.rect)
        {
            open = false;
        }

        // Only update from `open` if we didn't manually close via cancel button
        if self.fund_platform_dialog.is_open {
            self.fund_platform_dialog.is_open = open;
        }
        if !self.fund_platform_dialog.is_open {
            self.fund_platform_dialog = FundPlatformAddressDialogState::default();
        }
        action
    }

    /// Render the Private Key dialog
    pub(super) fn render_private_key_dialog(&mut self, ctx: &Context) {
        if !self.private_key_dialog.is_open {
            return;
        }

        let dark_mode = ctx.style().visuals.dark_mode;
        let mut open = self.private_key_dialog.is_open;

        // Draw dark overlay behind the dialog
        if open {
            Self::draw_modal_overlay(ctx, "private_key_dialog_overlay");
        }

        egui::Window::new("Private Key")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(Self::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_min_width(400.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);

                    // Address label
                    ui.label(
                        RichText::new("Address")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(5.0);

                    // Address value
                    ui.label(
                        RichText::new(&self.private_key_dialog.address)
                            .monospace()
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    ui.add_space(5.0);

                    // Copy address button
                    if ComponentStyles::add_secondary_button(ui, "Copy Address", dark_mode)
                        .clicked()
                    {
                        let _ = copy_text_to_clipboard(&self.private_key_dialog.address);
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(15.0);

                    // Private key label
                    ui.label(
                        RichText::new("Private Key (WIF)")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(5.0);

                    // Private key value (hidden by default)
                    if self.private_key_dialog.show_key {
                        ui.label(
                            RichText::new(self.private_key_dialog.private_key_wif.expose_secret())
                                .monospace()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    } else {
                        ui.label(
                            RichText::new("••••••••••••••••••••••••••••••••••••••••••••••••••••")
                                .monospace()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }

                    ui.add_space(10.0);

                    // Show/Hide and Copy buttons
                    ui.horizontal(|ui| {
                        let toggle_label = if self.private_key_dialog.show_key {
                            "Hide Key"
                        } else {
                            "Show Key"
                        };
                        if ComponentStyles::add_secondary_button(ui, toggle_label, dark_mode)
                            .clicked()
                        {
                            self.private_key_dialog.show_key = !self.private_key_dialog.show_key;
                        }

                        if ComponentStyles::add_primary_button(ui, "Copy Key").clicked() {
                            let _ = copy_text_to_clipboard(
                                self.private_key_dialog.private_key_wif.expose_secret(),
                            );
                        }
                    });

                    ui.add_space(15.0);

                    // Warning message
                    ui.label(
                        RichText::new("Keep your private key secure. Never share it with anyone.")
                            .color(DashColors::error_color(dark_mode))
                            .size(11.0)
                            .italics(),
                    );
                });
            });

        self.private_key_dialog.is_open = open;
        if !self.private_key_dialog.is_open {
            self.private_key_dialog = PrivateKeyDialogState::default();
        }
    }

    /// Prepare the backend task for funding a Platform address from asset lock
    pub(super) fn prepare_fund_platform_action(&mut self) -> AppAction {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use std::collections::BTreeMap;

        let Some(wallet_arc) = &self.selected_wallet else {
            self.fund_platform_dialog.status = Some("No wallet selected".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        let Some(selected_addr) = &self.fund_platform_dialog.selected_platform_address else {
            self.fund_platform_dialog.status = Some("Select a Platform address".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        let Some(asset_lock_txid) = self.fund_platform_dialog.selected_asset_lock_txid else {
            self.fund_platform_dialog.status = Some("No asset lock selected".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        // Get the asset lock proof and address from the database
        let (seed_hash, asset_lock_proof, asset_lock_address, platform_addr) = {
            let wallet = match wallet_arc.read() {
                Ok(guard) => guard,
                Err(e) => {
                    self.fund_platform_dialog.status = Some(e.to_string());
                    self.fund_platform_dialog.status_is_error = true;
                    return AppAction::None;
                }
            };

            // Read from the database (source of truth for all asset locks).
            let db_record = self
                .app_context
                .db
                .get_asset_lock_transaction(&asset_lock_txid);
            let Some((
                tx,
                _amount,
                islock,
                chain_locked_height,
                _identity_id,
                _wallet_seed,
                _network,
            )) = db_record.ok().flatten()
            else {
                self.fund_platform_dialog.status =
                    Some("Asset lock not found or not ready".to_string());
                self.fund_platform_dialog.status_is_error = true;
                return AppAction::None;
            };

            // Build proof from IS-lock or chain-locked height
            let proof = if let Some(ref islock) = islock {
                use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
                AssetLockProof::Instant(InstantAssetLockProof::new(islock.clone(), tx.clone(), 0))
            } else if let Some(height) = chain_locked_height {
                use dash_sdk::dpp::dashcore::OutPoint;
                use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
                AssetLockProof::Chain(ChainAssetLockProof {
                    core_chain_locked_height: height,
                    out_point: OutPoint::new(tx.txid(), 0),
                })
            } else {
                self.fund_platform_dialog.status =
                    Some("Asset lock proof not yet available".to_string());
                self.fund_platform_dialog.status_is_error = true;
                return AppAction::None;
            };

            // Derive address from credit output
            let addr = if let Some(dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType(payload)) = &tx.special_transaction_payload {
                payload.credit_outputs.first()
                    .and_then(|output| dash_sdk::dpp::dashcore::Address::from_script(&output.script_pubkey, self.app_context.network).ok())
                    .unwrap_or_else(|| dash_sdk::dpp::dashcore::Address::from_script(&tx.output[0].script_pubkey, self.app_context.network).unwrap())
            } else {
                self.fund_platform_dialog.status =
                    Some("Could not derive address from asset lock".to_string());
                self.fund_platform_dialog.status_is_error = true;
                return AppAction::None;
            };

            // Parse the Platform address (Bech32m format: dash1.../tdash1... per DIP-18)
            let platform_addr = if crate::ui::helpers::is_platform_address_string(selected_addr) {
                match PlatformAddress::from_bech32m_string(selected_addr) {
                    Ok((addr, network)) => {
                        // Validate that address network matches app network
                        if !crate::model::wallet::networks_address_compatible(
                            &network,
                            &self.app_context.network,
                        ) {
                            self.fund_platform_dialog.status = Some(format!(
                                "Address network mismatch: address is for {:?} but app is on {:?}",
                                network, self.app_context.network
                            ));
                            self.fund_platform_dialog.status_is_error = true;
                            return AppAction::None;
                        }
                        addr
                    }
                    Err(e) => {
                        self.fund_platform_dialog.status =
                            Some(format!("Invalid Bech32m address: {}", e));
                        self.fund_platform_dialog.status_is_error = true;
                        return AppAction::None;
                    }
                }
            } else {
                // Fall back to base58 parsing for backwards compatibility
                match selected_addr
                    .parse::<Address<NetworkUnchecked>>()
                    .map_err(|e| e.to_string())
                    .and_then(|a: Address<NetworkUnchecked>| {
                        PlatformAddress::try_from(a.assume_checked())
                            .map_err(|e| format!("Invalid Platform address: {}", e))
                    }) {
                    Ok(addr) => addr,
                    Err(e) => {
                        self.fund_platform_dialog.status = Some(e);
                        self.fund_platform_dialog.status_is_error = true;
                        return AppAction::None;
                    }
                }
            };

            (
                wallet.seed_hash(),
                Box::new(proof.clone()),
                addr.clone(),
                platform_addr,
            )
        };

        // Build outputs - fund the entire asset lock to the selected Platform address
        let mut outputs: BTreeMap<PlatformAddress, Option<u64>> = BTreeMap::new();
        outputs.insert(platform_addr, None); // None = take the full amount

        self.fund_platform_dialog.is_processing = true;
        self.fund_platform_dialog.status = Some("Processing...".to_string());
        self.fund_platform_dialog.status_is_error = false;

        AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromAssetLock {
                seed_hash,
                asset_lock_proof,
                asset_lock_address,
                outputs,
            },
        ))
    }

    pub(super) fn prepare_send_action(&mut self) -> Result<AppAction, String> {
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or_else(|| "Select a wallet first".to_string())?;

        let amount_duffs = self
            .send_dialog
            .amount
            .as_ref()
            .ok_or_else(|| "Enter an amount".to_string())?
            .dash_to_duffs()?;

        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        {
            let guard = wallet.read().map_err(|e| e.to_string())?;
            let spendable = guard
                .platform_wallet
                .as_ref()
                .map(|pw| pw.core().balance().spendable())
                .unwrap_or(0);
            if amount_duffs > spendable {
                return Err("Insufficient confirmed balance".to_string());
            }
        }

        if self.send_dialog.address.trim().is_empty() {
            return Err("Enter a recipient address".to_string());
        }

        let memo = self.send_dialog.memo.trim();
        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: self.send_dialog.address.trim().to_string(),
                amount_duffs,
            }],
            subtract_fee_from_amount: self.send_dialog.subtract_fee,
            memo: if memo.is_empty() {
                None
            } else {
                Some(memo.to_string())
            },
            override_fee: None,
        };

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet: wallet.clone(),
                request,
            },
        )))
    }

    pub(super) fn open_receive_dialog(&mut self, _ctx: &Context) -> AppAction {
        let Some(wallet) = self.selected_wallet.clone() else {
            self.receive_dialog.status = Some("Select a wallet first".to_string());
            self.receive_dialog.core_addresses.clear();
            self.receive_dialog.platform_addresses.clear();
            self.receive_dialog.qr_texture = None;
            self.receive_dialog.qr_address = None;
            self.receive_dialog.is_open = true;
            return AppAction::None;
        };

        self.receive_dialog.is_open = true;
        self.receive_dialog.qr_texture = None;
        self.receive_dialog.qr_address = None;

        // Load Core addresses (works with locked wallet - uses existing addresses)
        self.load_core_addresses_for_receive(&wallet);

        // Load Platform addresses (works with locked wallet - uses existing addresses)
        self.load_platform_addresses_for_receive(&wallet);

        AppAction::None
    }

    /// Load BIP44 external addresses with balances from a wallet.
    /// Uses PlatformWallet's CoreAddressInfo as the canonical source.
    /// Locked wallets (no PlatformWallet) return empty.
    fn load_bip44_external_addresses(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<Vec<(String, u64)>, String> {
        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
        let network = self.app_context.network;

        let addresses: Vec<(String, u64)> = if let Some(pw) = wallet_guard.platform_wallet.as_ref()
        {
            let info = pw.state_blocking();
            crate::platform_wallet_bridge::CoreAddressInfo::all_from_wallet_info(&info.core_wallet)
                .into_iter()
                .filter(|a| a.derivation_path.is_bip44_external(network))
                .map(|a| (a.address.to_string(), a.balance))
                .collect()
        } else {
            Vec::new()
        };
        Ok(addresses)
    }

    /// Load Core addresses into the receive dialog
    fn load_core_addresses_for_receive(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        match self.load_bip44_external_addresses(wallet) {
            Ok(addresses) if addresses.is_empty() => {
                match self.generate_new_core_receive_address(wallet) {
                    Ok((address, balance)) => {
                        self.receive_dialog.core_addresses = vec![(address, balance)];
                        self.receive_dialog.selected_core_index = 0;
                    }
                    Err(err) => {
                        self.receive_dialog.status = Some(err);
                        self.receive_dialog.core_addresses.clear();
                    }
                }
            }
            Ok(addresses) => {
                self.receive_dialog.core_addresses = addresses;
                self.receive_dialog.selected_core_index = 0;
            }
            Err(err) => {
                self.receive_dialog.status = Some(err);
            }
        }
    }

    /// Load Platform addresses into the receive dialog
    fn load_platform_addresses_for_receive(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        let wallet_guard = match wallet.read() {
            Ok(guard) => guard,
            Err(err) => {
                self.receive_dialog.status = Some(err.to_string());
                return;
            }
        };

        // Collect Platform addresses with their balances (using DIP-18 Bech32m format)
        let network = self.app_context.network;
        let db_info = self
            .app_context
            .db
            .get_all_platform_address_info(&wallet_guard.seed_hash(), &network)
            .unwrap_or_default();
        let platform_addresses: Vec<(String, u64)> = db_info
            .into_iter()
            .filter_map(|(core_addr, balance, _nonce)| {
                use dash_sdk::dpp::address_funds::PlatformAddress;
                PlatformAddress::try_from(core_addr)
                    .ok()
                    .map(|pa| (pa.to_bech32m_string(network), balance))
            })
            .collect();

        drop(wallet_guard);

        if platform_addresses.is_empty() {
            // Generate a new Platform address if none exists
            match self.generate_platform_address(wallet) {
                Ok(address) => {
                    self.receive_dialog.platform_addresses = vec![(address, 0)];
                    self.receive_dialog.selected_platform_index = 0;
                }
                Err(err) => {
                    self.receive_dialog.status = Some(err);
                    self.receive_dialog.platform_addresses.clear();
                }
            }
        } else {
            self.receive_dialog.platform_addresses = platform_addresses;
            self.receive_dialog.selected_platform_index = 0;
        }
    }

    pub(super) fn derive_private_key_wif(&self, path: &DerivationPath) -> Result<Secret, String> {
        let wallet_arc = self
            .selected_wallet
            .clone()
            .ok_or_else(|| "Select a wallet first".to_string())?;
        let wallet = wallet_arc.read().map_err(|e| e.to_string())?;
        if wallet.uses_password && !wallet.is_open() {
            return Err("Unlock this wallet to view private keys.".to_string());
        }
        let private_key = wallet.private_key_at_derivation_path(path, self.app_context.network)?;
        Ok(Secret::new(private_key.to_wif()))
    }

    pub(super) fn open_mine_dialog(&mut self) {
        let Some(wallet) = self.selected_wallet.clone() else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Select a wallet first",
                MessageType::Error,
            );
            return;
        };

        let address_input = AddressInput::new(self.app_context.network)
            .with_label("Mine to address:")
            .with_address_kinds(&[AddressKind::Core])
            .with_wallets(&[wallet], Some(&self.app_context.db))
            .with_selection_only(true)
            .with_full_addresses(true);

        self.mine_dialog = MineDialogState {
            is_open: true,
            address_input: Some(address_input),
            validated_address: None,
            block_count_str: "1".to_string(),
            error: None,
        };
    }

    pub(super) fn render_mine_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.mine_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.mine_dialog.is_open;
        let dark_mode = ctx.style().visuals.dark_mode;

        Self::draw_modal_overlay(ctx, "mine_dialog_overlay");

        let window_response = egui::Window::new("Mine Blocks")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(Self::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_min_width(350.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Mine blocks to a wallet address:")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);

                    // Address selector using AddressInput component
                    if let Some(address_input) = self.mine_dialog.address_input.as_mut() {
                        let resp = address_input.show(ui);
                        resp.inner.update(&mut self.mine_dialog.validated_address);
                    }

                    ui.add_space(10.0);

                    // Block count input
                    ui.label("Number of blocks:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mine_dialog.block_count_str)
                            .hint_text("1")
                            .desired_width(100.0),
                    );
                    self.mine_dialog
                        .block_count_str
                        .retain(|c| c.is_ascii_digit());

                    // Error display
                    if let Some(error) = self.mine_dialog.error.clone() {
                        ui.add_space(8.0);
                        let error_color = DashColors::ERROR;
                        Frame::new()
                            .fill(error_color.gamma_multiply(0.1))
                            .inner_margin(Margin::symmetric(10, 8))
                            .corner_radius(5.0)
                            .stroke(egui::Stroke::new(1.0, error_color))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Error: {}", error))
                                            .color(error_color),
                                    );
                                    ui.add_space(10.0);
                                    if ui.small_button("Dismiss").clicked() {
                                        self.mine_dialog.error = None;
                                    }
                                });
                            });
                    }

                    ui.add_space(15.0);

                    // Buttons
                    ui.horizontal(|ui| {
                        if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked()
                        {
                            self.mine_dialog = MineDialogState::default();
                        }

                        ui.add_space(8.0);

                        if ComponentStyles::add_primary_button(ui, "Mine").clicked() {
                            const MAX_MINE_BLOCKS: u64 = 1_000;
                            let block_count: u64 =
                                match self.mine_dialog.block_count_str.trim().parse() {
                                    Ok(n) if n > 0 && n <= MAX_MINE_BLOCKS => n,
                                    Ok(n) if n > MAX_MINE_BLOCKS => {
                                        self.mine_dialog.error = Some(format!(
                                            "Maximum {} blocks at a time",
                                            MAX_MINE_BLOCKS
                                        ));
                                        return;
                                    }
                                    _ => {
                                        self.mine_dialog.error = Some(
                                            "Enter a valid number of blocks (> 0)".to_string(),
                                        );
                                        return;
                                    }
                                };

                            let Some(validated) = &self.mine_dialog.validated_address else {
                                self.mine_dialog.error =
                                    Some("Select an address first".to_string());
                                return;
                            };

                            let Some(address) = validated.as_core().cloned() else {
                                self.mine_dialog.error = Some("Select a Core address".to_string());
                                return;
                            };

                            let Some(wallet) = self.selected_wallet.clone() else {
                                self.mine_dialog.error = Some("No wallet selected".to_string());
                                return;
                            };

                            action = AppAction::BackendTask(BackendTask::CoreTask(
                                CoreTask::MineBlocks {
                                    block_count,
                                    address,
                                    wallet,
                                },
                            ));
                            self.mine_dialog = MineDialogState::default();
                        }
                    });
                });
            });

        if let Some(ref resp) = window_response
            && clicked_outside_window(ctx, resp.response.rect)
        {
            open = false;
        }

        if !open || !self.mine_dialog.is_open {
            self.mine_dialog = MineDialogState::default();
        }
        action
    }
}
