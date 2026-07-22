use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::secret::Secret;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::address_input::AddressInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::{ComponentStyles, DashColors};
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use eframe::egui::{self, ComboBox, Context};
use eframe::epaint::TextureHandle;
use egui::load::SizedTexture;
use egui::{Frame, Margin, RichText, TextureOptions};
use std::sync::{Arc, RwLock};

use super::WalletsBalancesScreen;

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
    /// A queued "generate Platform receive address" request the `ui()` loop
    /// drains into a `WalletTask::GeneratePlatformReceiveAddress` backend task.
    /// The seed is fetched just-in-time in the backend; only the new address
    /// returns here. Carries the wallet's seed hash.
    pub pending_platform_address_request: Option<WalletSeedHash>,
    /// A queued "generate Core receive address" request the `ui()` loop drains
    /// into a `WalletTask::GenerateReceiveAddress` backend task. The address is
    /// derived from the upstream SPV-watched pool so it is always monitored —
    /// never a DET-side index past the gap window. Carries the wallet's seed
    /// hash; the new address returns via `GeneratedReceiveAddress`.
    pub pending_core_address_request: Option<WalletSeedHash>,
}

impl ReceiveDialogState {
    pub(super) fn open(&mut self) {
        self.is_open = true;
    }
}

/// State for the Fund Platform Address from Asset Lock dialog
#[derive(Default)]
pub(super) struct FundPlatformAddressDialogState {
    pub is_open: bool,
    /// Outpoint of the upstream-tracked asset lock chosen to fund a Platform
    /// address. `None` until the user clicks "Fund" on a row in the asset-
    /// locks table.
    pub selected_asset_lock_out_point: Option<dash_sdk::dpp::dashcore::OutPoint>,
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
    /// A queued key-display request the `ui()` loop drains into a
    /// `WalletTask::DeriveKeyForDisplay` backend task. The seed is fetched
    /// just-in-time in the backend; only the WIF returns here. Tuple is
    /// `(seed_hash, derivation_path, display_address)`.
    pub pending_view_key_request: Option<(WalletSeedHash, DerivationPath, String)>,
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
            fill: ctx.global_style().visuals.window_fill,
            stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
        }
    }

    pub(super) fn render_receive_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.receive_dialog.is_open {
            return AppAction::None;
        }

        // Refresh cached balances from the display-only WalletBackend
        // snapshot so chain updates are reflected.
        if let Some(wallet) = &self.selected_wallet
            && let Ok(wallet_guard) = wallet.read()
        {
            use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
            let address_balances = self
                .app_context
                .snapshot_address_balances(&wallet_guard.seed_hash());
            for (addr_str, balance) in &mut self.receive_dialog.core_addresses {
                if let Ok(addr) = addr_str.parse::<Address<NetworkUnchecked>>()
                    && let Ok(addr) = addr.require_network(self.app_context.network)
                {
                    *balance = address_balances.get(&addr).copied().unwrap_or(0);
                }
            }
        }

        let dark_mode = ctx.global_style().visuals.dark_mode;

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
        let mut close_clicked = false;

        // Draw dark overlay behind the dialog (only when open)
        if open {
            Self::draw_modal_overlay(ctx, "receive_dialog_overlay");
        }

        egui::Window::new("Receive")
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
                                    && let Some(wallet) = self.selected_wallet.clone() {
                                        self.queue_core_address_request(&wallet);
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
                                let mut request_new_addr = false;

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
                                    if self.selected_wallet.is_some()
                                        && ComponentStyles::add_secondary_button(
                                            ui,
                                            "New Address",
                                            dark_mode,
                                        )
                                        .clicked()
                                    {
                                        request_new_addr = true;
                                    }
                                });

                                // Handle copy status after the closure
                                if let Some(status) = copy_status {
                                    self.receive_dialog.status = Some(status);
                                }

                                // Queue the backend address-generation request
                                // after the closure (it borrows `&mut self`).
                                if request_new_addr
                                    && let Some(wallet) = self.selected_wallet.clone()
                                {
                                    self.queue_platform_address_request(&wallet);
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

                    ui.add_space(10.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                            close_clicked = true;
                        }
                    });
                });
            });

        if close_clicked {
            open = false;
        }

        self.receive_dialog.is_open = open;
        if !self.receive_dialog.is_open {
            self.receive_dialog = ReceiveDialogState::default();
        }
        AppAction::None
    }

    /// Queue a "generate a new Platform receive address" request for the wallet.
    ///
    /// The actual derivation runs in the backend via
    /// `WalletTask::GeneratePlatformReceiveAddress` — the `ui()` loop drains
    /// `pending_platform_address_request` into a backend task, the seed is
    /// fetched just-in-time, and only the new Bech32m address returns. The seed
    /// never crosses into the UI layer.
    pub(super) fn queue_platform_address_request(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        let seed_hash = match wallet.read() {
            Ok(w) => w.seed_hash(),
            Err(_) => {
                self.receive_dialog.status =
                    Some("Could not read the selected wallet. Please retry.".to_string());
                return;
            }
        };
        self.receive_dialog.pending_platform_address_request = Some(seed_hash);
        self.receive_dialog.status = Some("Generating a new address…".to_string());
    }

    /// Queue a "generate a new Core receive address" request for the wallet.
    ///
    /// The derivation runs in the backend via `WalletTask::GenerateReceiveAddress`
    /// (→ upstream `next_unused`), so the returned address is always inside the
    /// SPV-watched gap-limit window. The `ui()` loop drains
    /// `pending_core_address_request` into the backend task; the new address
    /// returns via `GeneratedReceiveAddress`. Deriving DET-side here would hand
    /// out an address past the watched window and lose deposits sent to it.
    pub(super) fn queue_core_address_request(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        let seed_hash = match wallet.read() {
            Ok(w) => w.seed_hash(),
            Err(_) => {
                self.receive_dialog.status =
                    Some("Could not read the selected wallet. Please retry.".to_string());
                return;
            }
        };
        self.receive_dialog.pending_core_address_request = Some(seed_hash);
        self.receive_dialog.status = Some("Generating a new address…".to_string());
    }

    /// Opens a funded-address dialog with deterministic inputs for UI tests.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn open_fund_platform_dialog_for_test(&mut self, platform_addresses: Vec<(String, u64)>) {
        use dash_sdk::dpp::dashcore::hashes::Hash;

        self.fund_platform_dialog = FundPlatformAddressDialogState {
            is_open: true,
            selected_asset_lock_out_point: Some(dash_sdk::dpp::dashcore::OutPoint::new(
                dash_sdk::dpp::dashcore::Txid::from_byte_array([0; 32]),
                0,
            )),
            platform_addresses,
            ..Default::default()
        };
    }

    /// Render the Fund Platform Address from Asset Lock dialog
    pub(super) fn render_fund_platform_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.fund_platform_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.fund_platform_dialog.is_open;
        let dark_mode = ctx.global_style().visuals.dark_mode;

        // Draw dark overlay behind the popup
        Self::draw_modal_overlay(ctx, "fund_platform_dialog_overlay");

        egui::Window::new("Fund Platform Address from Asset Lock")
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
                            && self.fund_platform_dialog.selected_asset_lock_out_point.is_some()
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

        let dark_mode = ctx.global_style().visuals.dark_mode;
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

        let Some(out_point) = self.fund_platform_dialog.selected_asset_lock_out_point else {
            self.fund_platform_dialog.status = Some("No asset lock selected".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        let seed_hash = match wallet_arc.read() {
            Ok(guard) => guard.seed_hash(),
            Err(e) => {
                self.fund_platform_dialog.status = Some(e.to_string());
                self.fund_platform_dialog.status_is_error = true;
                return AppAction::None;
            }
        };

        // Parse the Platform address (Bech32m format: dash1.../tdash1... per DIP-18)
        let platform_addr = if crate::ui::helpers::is_platform_address_string(selected_addr) {
            match PlatformAddress::from_bech32m_string(selected_addr) {
                Ok(addr) => {
                    // `from_bech32m_string` no longer returns the network. Derive
                    // the mainnet/non-mainnet class from the HRP and synthesise a
                    // representative `Network` for `networks_address_compatible`:
                    // mainnet HRP ("dash1…") → `Mainnet`, anything else → `Testnet`
                    // (testnet and all non-mainnet networks share the "tdash1…" HRP).
                    let addr_is_mainnet =
                        PlatformAddress::is_mainnet_bech32m(selected_addr).unwrap_or(false);
                    let addr_network = if addr_is_mainnet {
                        Network::Mainnet
                    } else {
                        Network::Testnet
                    };
                    if !crate::model::wallet::networks_address_compatible(
                        &addr_network,
                        &self.app_context.network,
                    ) {
                        let addr_net_label = if addr_is_mainnet {
                            "mainnet"
                        } else {
                            "testnet"
                        };
                        self.fund_platform_dialog.status = Some(format!(
                            "Address network mismatch: address is for {} but app is on {:?}",
                            addr_net_label, self.app_context.network
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

        let mut outputs: BTreeMap<PlatformAddress, Option<u64>> = BTreeMap::new();
        outputs.insert(platform_addr, None);

        self.fund_platform_dialog.is_processing = true;
        self.fund_platform_dialog.status = Some("Processing...".to_string());
        self.fund_platform_dialog.status_is_error = false;

        AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromAssetLock {
                seed_hash,
                out_point,
                outputs,
            },
        ))
    }

    pub(super) fn open_receive_dialog(&mut self, _ctx: &Context) -> AppAction {
        let Some(wallet) = self.selected_wallet.clone() else {
            self.receive_dialog.status = Some("Select a wallet first".to_string());
            self.receive_dialog.core_addresses.clear();
            self.receive_dialog.platform_addresses.clear();
            self.receive_dialog.qr_texture = None;
            self.receive_dialog.qr_address = None;
            self.receive_dialog.open();
            return AppAction::None;
        };

        self.receive_dialog.open();
        self.receive_dialog.qr_texture = None;
        self.receive_dialog.qr_address = None;

        // Load Core addresses (works with locked wallet - uses existing addresses)
        self.load_core_addresses_for_receive(&wallet);

        // Load Platform addresses (works with locked wallet - uses existing addresses)
        self.load_platform_addresses_for_receive(&wallet);

        AppAction::None
    }

    /// Load the SPV-watched BIP44 external (receive) addresses with balances.
    ///
    /// Sourced from the lock-free `WalletSnapshot` monitored set, so the Receive
    /// list shows exactly the addresses SPV watches — never a DET-side index
    /// past the gap window. Empty before the first sync publishes a snapshot;
    /// the caller then asks the backend to derive a watched address.
    fn load_bip44_external_addresses(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<Vec<(String, u64)>, String> {
        let seed_hash = wallet.read().map_err(|e| e.to_string())?.seed_hash();
        let backend = self
            .app_context
            .wallet_backend()
            .map_err(|e| e.to_string())?;
        let address_balances = self.app_context.snapshot_address_balances(&seed_hash);
        let addresses: Vec<(String, u64)> = backend
            .snapshot_monitored_receive_addresses(&seed_hash)
            .into_iter()
            .map(|addr_str| {
                let balance = addr_str
                    .parse::<Address<_>>()
                    .ok()
                    .and_then(|addr| address_balances.get(&addr.assume_checked()).copied())
                    .unwrap_or(0);
                (addr_str, balance)
            })
            .collect();
        Ok(addresses)
    }

    /// Load Core addresses into the receive dialog
    fn load_core_addresses_for_receive(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        match self.load_bip44_external_addresses(wallet) {
            Ok(addresses) if !addresses.is_empty() => {
                self.receive_dialog.core_addresses = addresses;
                self.receive_dialog.selected_core_index = 0;
            }
            // Empty list or the wallet isn't watched yet: ask the backend to
            // derive an address from the SPV-watched pool. The result arrives
            // via `GeneratedReceiveAddress`; this is self-healing once the
            // wallet finishes registering with the backend.
            _ => {
                self.receive_dialog.core_addresses.clear();
                self.receive_dialog.selected_core_index = 0;
                self.queue_core_address_request(wallet);
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
        // Use platform_addresses() which checks watched_addresses, not just platform_address_info
        // This includes addresses that have been derived but may not have been synced yet
        let network = self.app_context.network;
        let platform_addresses: Vec<(String, u64)> = wallet_guard
            .platform_addresses(network)
            .into_iter()
            .map(|(core_addr, platform_addr)| {
                let balance = wallet_guard
                    .get_platform_address_info(&core_addr)
                    .map(|info| info.balance)
                    .unwrap_or(0);
                (platform_addr.to_bech32m_string(network), balance)
            })
            .collect();

        drop(wallet_guard);

        if platform_addresses.is_empty() {
            // No address yet: queue a backend generation request. The seed is
            // fetched just-in-time and the new address arrives via
            // `display_task_result`.
            self.receive_dialog.platform_addresses.clear();
            self.queue_platform_address_request(wallet);
        } else {
            self.receive_dialog.platform_addresses = platform_addresses;
            self.receive_dialog.selected_platform_index = 0;
        }
    }

    /// Queue a private-key-display request for the given path and address.
    ///
    /// The actual derivation runs in the backend via
    /// `WalletTask::DeriveKeyForDisplay` — the `ui()` loop drains
    /// `pending_view_key_request` into a backend task, the seed is fetched
    /// just-in-time, and only the WIF (wrapped in `Secret`) returns. The seed
    /// never crosses into the UI layer.
    pub(super) fn queue_view_key_request(
        &mut self,
        path: &DerivationPath,
        display_address: String,
    ) {
        let Some(wallet_arc) = self.selected_wallet.clone() else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Select a wallet first",
                MessageType::Error,
            );
            return;
        };
        let seed_hash = match wallet_arc.read() {
            Ok(w) => w.seed_hash(),
            Err(_) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Could not read the selected wallet. Please retry.",
                    MessageType::Error,
                );
                return;
            }
        };
        self.private_key_dialog.pending_view_key_request =
            Some((seed_hash, path.clone(), display_address));
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

        let seed_hash = wallet.read().map(|g| g.seed_hash()).unwrap_or_default();
        let balances = self.app_context.snapshot_address_balances(&seed_hash);
        let paths = self.app_context.snapshot_address_paths(&seed_hash);
        let address_input = AddressInput::new(self.app_context.network)
            .with_label("Mine to address:")
            .with_address_kinds(&[AddressKind::Core])
            .with_wallets(&[(wallet, balances, paths)])
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
        let dark_mode = ctx.global_style().visuals.dark_mode;

        Self::draw_modal_overlay(ctx, "mine_dialog_overlay");

        egui::Window::new("Mine Blocks")
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

        if !open || !self.mine_dialog.is_open {
            self.mine_dialog = MineDialogState::default();
        }
        action
    }
}
