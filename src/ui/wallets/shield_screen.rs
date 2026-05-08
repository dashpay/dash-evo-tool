use crate::app::AppAction;
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::Amount;
use crate::model::fee_estimation::shielded_fee_for_actions;
use crate::model::wallet::WalletSeedHash;
use crate::ui::components::ComponentResponse;
use crate::ui::components::MessageBanner;
use crate::ui::components::address_input::AddressInput;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::version::PlatformVersion;
use eframe::egui::{self, Context};
use egui::RichText;
use std::sync::Arc;

#[derive(PartialEq)]
enum Status {
    NotStarted,
    WaitingForResult,
    Complete,
}

/// Which kind of source funds the shield, and how the funds are selected.
///
/// This is a routing choice, not coin control: `Core` shields the whole wallet
/// (the asset lock spends the full live UTXO set — no per-address selection
/// exists), while `Platform` shields from the wallet's platform balance (the
/// upstream coordinator selects the input addresses).
#[derive(PartialEq, Clone, Copy)]
enum ShieldSourceKind {
    /// Type 18 asset-lock shield of the whole Core wallet.
    Core,
    /// Type 15 shield from the wallet's platform balance.
    Platform,
}

pub struct ShieldScreen {
    pub app_context: Arc<AppContext>,
    pub seed_hash: WalletSeedHash,
    /// Whether the shield draws from the whole Core wallet or platform balance.
    source_kind: ShieldSourceKind,
    /// Platform-address picker — only shown when `source_kind` is `Platform`.
    /// The chosen address sizes the available-balance display; the upstream
    /// coordinator selects the actual spend inputs.
    address_input: Option<AddressInput>,
    /// The chosen platform address, set by `address_input`. `None` for the Core
    /// path, which always shields the whole wallet.
    validated_source: Option<ValidatedAddress>,
    amount_input: Option<AmountInput>,
    amount: Option<Amount>,
    status: Status,
    // Cached wallet data to avoid per-frame RwLock reads (CODE-007)
    cached_base_nonce: Option<u32>,
    cached_platform_balance: Option<u64>,
    cached_core_balance: Option<u64>,
}

impl ShieldScreen {
    pub fn new(seed_hash: WalletSeedHash, app_context: &Arc<AppContext>) -> Self {
        let mut screen = Self {
            app_context: app_context.clone(),
            seed_hash,
            source_kind: ShieldSourceKind::Core,
            address_input: None,
            validated_source: None,
            amount_input: None,
            amount: None,
            status: Status::NotStarted,
            cached_base_nonce: None,
            cached_platform_balance: None,
            cached_core_balance: None,
        };
        screen.refresh_cached_balances();
        screen
    }

    /// Reset the source and amount inputs — called when AppContext switches network.
    pub(crate) fn invalidate_address_input(&mut self) {
        self.source_kind = ShieldSourceKind::Core;
        self.address_input = None;
        self.validated_source = None;
        self.amount_input = None;
        self.amount = None;
        self.cached_base_nonce = None;
        self.cached_platform_balance = None;
        self.cached_core_balance = None;
    }

    /// Returns the selected platform address, if a platform source is selected.
    fn selected_platform_address(&self) -> Option<PlatformAddress> {
        self.validated_source
            .as_ref()
            .and_then(|v| v.as_platform().copied())
    }

    /// The selected source kind as an [`AddressKind`], for the downstream
    /// per-kind rendering and dispatch matches.
    fn source_kind(&self) -> AddressKind {
        match self.source_kind {
            ShieldSourceKind::Core => AddressKind::Core,
            ShieldSourceKind::Platform => AddressKind::Platform,
        }
    }

    /// Whether the source is fully specified and the amount/confirm controls may
    /// show. Core shields the whole wallet so it is always ready; Platform needs
    /// a chosen address (to size the available-balance display).
    fn source_is_ready(&self) -> bool {
        match self.source_kind {
            ShieldSourceKind::Core => true,
            ShieldSourceKind::Platform => self.validated_source.is_some(),
        }
    }

    /// Whether this wallet has any funded platform address to shield from. Gates
    /// the Platform source option — without one there is nothing to pick.
    fn has_platform_addresses(&self) -> bool {
        self.app_context
            .wallets
            .read()
            .ok()
            .and_then(|wallets| {
                let wallet = wallets.get(&self.seed_hash)?;
                let guard = wallet.read().ok()?;
                Some(guard.platform_address_info.values().any(|i| i.balance > 0))
            })
            .unwrap_or(false)
    }

    /// Refresh cached wallet data (balance, nonce) from the RwLock-protected wallet.
    fn refresh_cached_balances(&mut self) {
        // Clone the wallet Arc while holding the wallets map read lock, then
        // drop the map lock before acquiring the per-wallet lock to avoid
        // lock-order deadlocks with code that holds a wallet lock and needs
        // wallets write access.
        let wallet_arc = self
            .app_context
            .wallets
            .read()
            .ok()
            .and_then(|w| w.get(&self.seed_hash).cloned());
        let Some(wallet_arc) = wallet_arc else {
            return;
        };
        let wallet_guard = wallet_arc.read().ok();

        if let Some(wallet) = &wallet_guard {
            // Platform nonce and balance for selected address
            if let Some(from_address) = self.selected_platform_address() {
                let info = wallet
                    .platform_address_info
                    .iter()
                    .find_map(|(addr, info)| {
                        let platform_addr = PlatformAddress::try_from(addr.clone()).ok()?;
                        (platform_addr == from_address).then_some(info)
                    });
                self.cached_base_nonce = info.map(|i| i.nonce);
                self.cached_platform_balance = info.map(|i| i.balance);
            } else {
                self.cached_base_nonce = None;
                self.cached_platform_balance = None;
            }

            // Core balance — whole-wallet total from the display-only
            // WalletBackend snapshot. The asset lock spends from the wallet's
            // full live UTXO set, so the shieldable amount is the wallet total.
            self.cached_core_balance =
                Some(self.app_context.snapshot_balance(&self.seed_hash).total);
        } else {
            self.cached_base_nonce = None;
            self.cached_platform_balance = None;
            self.cached_core_balance = Some(0);
        }
    }

    /// Return the cached nonce for the selected platform address.
    fn read_base_nonce(&self) -> Option<u32> {
        self.cached_base_nonce
    }

    /// Return the cached balance (credits) for the selected platform address.
    fn read_platform_balance(&self) -> Option<u64> {
        self.cached_platform_balance
    }

    /// Return the cached core wallet balance in duffs.
    fn read_core_balance_duffs(&self) -> u64 {
        self.cached_core_balance.unwrap_or(0)
    }
}

impl ScreenLike for ShieldScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::PopScreen),
                ("Shield", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        island_central_panel(ctx, |ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.heading("Shield");
            ui.add_space(10.0);
            ui.label("Move funds from a platform or core address into the shielded pool.");
            ui.add_space(15.0);

            // When complete, show a Done button below the banner
            if self.status == Status::Complete {
                ui.add_space(10.0);
                if ui.button("Done").clicked() {
                    action = AppAction::PopScreen;
                }
                return;
            }

            let is_busy = self.status == Status::WaitingForResult;

            // Source selection and amount inputs (disabled while a shield is in flight)
            let source_kind = ui
                .add_enabled_ui(!is_busy, |ui| {
                    // Source kind: whole Core wallet (Type 18 asset lock) vs the
                    // wallet's platform balance (Type 15). This routes the shield;
                    // it is not coin control. The Core path has no per-address
                    // selection — the asset lock always spends the whole wallet.
                    let has_platform = self.has_platform_addresses();
                    ui.label("Shield from:");
                    ui.horizontal(|ui| {
                        if ui
                            .radio_value(
                                &mut self.source_kind,
                                ShieldSourceKind::Core,
                                "Core wallet (whole balance)",
                            )
                            .changed()
                        {
                            self.address_input = None;
                            self.validated_source = None;
                            self.amount_input = None;
                            self.amount = None;
                            self.refresh_cached_balances();
                        }
                        ui.add_enabled_ui(has_platform, |ui| {
                            if ui
                                .radio_value(
                                    &mut self.source_kind,
                                    ShieldSourceKind::Platform,
                                    "Platform balance",
                                )
                                .changed()
                            {
                                self.address_input = None;
                                self.validated_source = None;
                                self.amount_input = None;
                                self.amount = None;
                                self.refresh_cached_balances();
                            }
                        });
                    });
                    ui.add_space(5.0);

                    match self.source_kind {
                        ShieldSourceKind::Platform => {
                            // The chosen platform address sizes the available-
                            // balance display; the upstream coordinator selects
                            // the actual input addresses for the Type 15 shield.
                            let addr_input = self.address_input.get_or_insert_with(|| {
                                let mut builder = AddressInput::new(self.app_context.network)
                                    .with_address_kinds(&[AddressKind::Platform])
                                    .with_label("Platform address")
                                    .with_hint_text("Select a platform address to shield from")
                                    .with_selection_only(true)
                                    .with_balance_range(1..)
                                    .with_exclude_change(true);

                                if let Ok(wallets) = self.app_context.wallets.read()
                                    && let Some(wallet) = wallets.get(&self.seed_hash)
                                {
                                    let balances =
                                        self.app_context.snapshot_address_balances(&self.seed_hash);
                                    builder = builder.with_wallets(&[(wallet.clone(), balances)]);
                                }

                                builder
                            });
                            let resp = addr_input.show(ui);
                            if resp.inner.has_changed() {
                                resp.inner.update(&mut self.validated_source);
                                self.amount_input = None;
                                self.amount = None;
                                self.refresh_cached_balances();
                            }
                            ui.add_space(5.0);

                            if let Some(balance_credits) = self.read_platform_balance() {
                                let balance_dash =
                                    balance_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "Available: {:.8} DASH",
                                            balance_dash
                                        ))
                                        .color(DashColors::success_color(dark_mode)),
                                    );
                                    if self.app_context.is_developer_mode()
                                        && let Some(nonce) = self.read_base_nonce()
                                    {
                                        ui.label(
                                            RichText::new(format!("(nonce: {})", nonce))
                                                .color(DashColors::muted_color(dark_mode))
                                                .small(),
                                        );
                                    }
                                });
                                ui.add_space(5.0);
                            }
                        }
                        ShieldSourceKind::Core => {
                            // The asset lock spends the whole wallet's live UTXO
                            // set, so there is nothing per-address to pick.
                            let balance_duffs = self.read_core_balance_duffs();
                            let dash_balance = balance_duffs as f64 / 1e8;
                            ui.label(
                                RichText::new(format!(
                                    "Available core wallet balance: {:.8} DASH",
                                    dash_balance
                                ))
                                .color(DashColors::success_color(dark_mode)),
                            );
                            ui.add_space(5.0);
                        }
                    }

                    // `Some` only when the source is fully specified, so the
                    // downstream amount/confirm controls gate on readiness.
                    let source_kind = self.source_is_ready().then(|| self.source_kind());

                    // Amount input (only when the source is ready)
                    if self.source_is_ready() {
                        let max_credits = match source_kind {
                            Some(AddressKind::Platform) => {
                                let base_fee =
                                    shielded_fee_for_actions(2, PlatformVersion::latest())
                                        .unwrap_or(0);
                                let multiplier =
                                    self.app_context.fee_multiplier_permille().max(1000);
                                let fee_headroom = base_fee.saturating_mul(multiplier) / 1000;
                                self.read_platform_balance()
                                    .map(|b| b.saturating_sub(fee_headroom))
                            }
                            Some(AddressKind::Core) => {
                                let balance_duffs = self.read_core_balance_duffs();
                                let (platform_fee_duffs, l1_tx_fee_duffs) = self
                                    .app_context
                                    .fee_estimator()
                                    .estimate_shield_from_core_fees_duffs();
                                let shieldable_duffs = balance_duffs
                                    .saturating_sub(platform_fee_duffs)
                                    .saturating_sub(l1_tx_fee_duffs);
                                Some(shieldable_duffs * CREDITS_PER_DUFF)
                            }
                            _ => None,
                        };

                        let amount_input = self.amount_input.get_or_insert_with(|| {
                            let mut builder = AmountInput::new(Amount::new_dash(0.0))
                                .with_label("Amount (DASH):")
                                .with_hint_text("Enter amount")
                                .with_desired_width(150.0);
                            if source_kind == Some(AddressKind::Core) {
                                builder = builder.with_max_button(true);
                            }
                            builder
                        });
                        if let Some(max) = max_credits {
                            amount_input.set_max_amount(Some(max));
                        }
                        let response = amount_input.show(ui);
                        response.inner.update(&mut self.amount);
                        ui.add_space(5.0);
                    }

                    source_kind
                })
                .inner;

            ui.add_space(15.0);

            // Progress display
            if self.status == Status::WaitingForResult {
                let spinner_msg = match source_kind {
                    Some(AddressKind::Core) => {
                        "Creating asset lock and shielding... (this may take a few minutes)"
                    }
                    _ => "Shielding credits...",
                };
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(spinner_msg);
                });
            }

            // Buttons (only when not busy and the source is ready)
            if !is_busy && self.status == Status::NotStarted && self.source_is_ready() {
                let can_confirm = self
                    .amount
                    .as_ref()
                    .map(|a| a.value())
                    .is_some_and(|v| v > 0);

                ui.horizontal(|ui| {
                    let button_label = match source_kind {
                        Some(AddressKind::Core) => "Shield from Core",
                        _ => "Shield",
                    };

                    if ui
                        .add_enabled(
                            can_confirm,
                            egui::Button::new(
                                RichText::new(button_label)
                                    .color(DashColors::WHITE)
                                    .size(16.0),
                            )
                            .fill(DashColors::DASH_BLUE),
                        )
                        .clicked()
                        && let Some(amount) = self.amount.as_ref().map(|a| a.value())
                    {
                        match source_kind {
                            Some(AddressKind::Platform) => {
                                // Balance check against the selected address.
                                if let Some(balance) = self.read_platform_balance()
                                    && amount > balance
                                {
                                    let amount_dash =
                                        amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                    let balance_dash =
                                        balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                    MessageBanner::set_global(
                                        ctx,
                                        format!(
                                            "Insufficient balance: {:.8} DASH requested but only {:.8} DASH available. Try a smaller amount.",
                                            amount_dash, balance_dash,
                                        ),
                                        MessageType::Error,
                                    );
                                    return;
                                }

                                self.status = Status::WaitingForResult;
                                action = AppAction::BackendTask(BackendTask::ShieldedTask(
                                    ShieldedTask::ShieldFromBalance {
                                        seed_hash: self.seed_hash,
                                        amount,
                                    },
                                ));
                            }
                            Some(AddressKind::Core) => {
                                let amount_duffs = amount / CREDITS_PER_DUFF;
                                self.status = Status::WaitingForResult;
                                action = AppAction::BackendTask(BackendTask::ShieldedTask(
                                    ShieldedTask::ShieldFromAssetLock {
                                        seed_hash: self.seed_hash,
                                        amount_duffs,
                                    },
                                ));
                            }
                            _ => {}
                        }
                    }

                    ui.add_space(10.0);
                    if ui.button("Cancel").clicked() {
                        action = AppAction::PopScreen;
                    }
                });
            }
        });

        action
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh_cached_balances();
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        self.refresh_cached_balances();
        let ctx = self.app_context.egui_ctx().clone();
        match result {
            BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                MessageBanner::set_global(
                    &ctx,
                    format!("Successfully shielded {:.8} DASH", dash),
                    MessageType::Success,
                );
            }
            BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                self.status = Status::Complete;
                let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                MessageBanner::set_global(
                    &ctx,
                    format!("Successfully shielded {:.8} DASH from core wallet", dash),
                    MessageType::Success,
                );
            }
            _ => {}
        }
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        if message_type == MessageType::Error && self.status == Status::WaitingForResult {
            self.status = Status::NotStarted;
        }
        // If status is Complete, leave it — the shield succeeded.
    }
}
