use crate::app::AppAction;
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::shielded::bundle::ShieldStage;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::Amount;
use crate::model::feature_gate::{FeatureGate, FeatureGateUiExt};
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
use dash_sdk::dpp::serialization::PlatformSerializable;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::version::PlatformVersion;
use eframe::egui::{self, Context};
use egui::RichText;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Extract the expected nonce from an `AddressInvalidNonceError` buried in an SDK error.
///
/// The error can arrive via two paths:
/// 1. `Error::StateTransitionBroadcastError` → `cause: ConsensusError` → `AddressInvalidNonceError`
/// 2. `Error::Protocol(ProtocolError::ConsensusError(...))` → `AddressInvalidNonceError`
fn extract_expected_nonce(error: &dash_sdk::Error) -> Option<u32> {
    use dash_sdk::dpp::ProtocolError;
    use dash_sdk::dpp::consensus::ConsensusError;
    use dash_sdk::dpp::consensus::state::state_error::StateError;

    // Helper: extract from a ConsensusError
    let from_consensus = |c: &ConsensusError| -> Option<u32> {
        match c {
            ConsensusError::StateError(StateError::AddressInvalidNonceError(e)) => {
                Some(e.expected_nonce())
            }
            _ => None,
        }
    };

    match error {
        // Path 1: broadcast error with consensus cause
        dash_sdk::Error::StateTransitionBroadcastError(e) => from_consensus(e.cause.as_ref()?),
        // Path 2: protocol error wrapping consensus error (boxed)
        dash_sdk::Error::Protocol(ProtocolError::ConsensusError(c)) => from_consensus(c.as_ref()),
        _ => None,
    }
}

#[derive(PartialEq)]
enum Status {
    NotStarted,
    WaitingForResult,
    BatchInProgress,
    Complete,
}

pub struct ShieldScreen {
    pub app_context: Arc<AppContext>,
    pub seed_hash: WalletSeedHash,
    address_input: Option<AddressInput>,
    validated_source: Option<ValidatedAddress>,
    amount_input: Option<AmountInput>,
    amount: Option<Amount>,
    status: Status,
    // Batch mode (dev only, Platform flow only)
    repeat_count_str: String,
    parallel: bool,
    batch_total: u32,
    batch_succeeded: u32,
    batch_failed: u32,
    batch_remaining: u32,
    /// Queued task to dispatch on next frame (for sequential batch mode).
    pending_next_task: Option<BackendTask>,
    /// Queued sync task to dispatch on next frame after successful operation.
    pending_refresh_task: Option<BackendTask>,
    /// Per-operation progress for parallel batch mode.
    batch_stages: Option<Vec<Arc<Mutex<ShieldStage>>>>,
    /// JSON of a failed state transition to show in the popup.
    json_preview: Option<String>,
    /// Frozen amount for the current batch (set at batch start, cleared on completion).
    batch_amount: Option<u64>,
    /// Frozen platform address for the current batch.
    batch_address: Option<PlatformAddress>,
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
            address_input: None,
            validated_source: None,
            amount_input: None,
            amount: None,
            status: Status::NotStarted,
            repeat_count_str: "1".to_string(),
            parallel: false,
            batch_total: 0,
            batch_succeeded: 0,
            batch_failed: 0,
            batch_remaining: 0,
            pending_next_task: None,
            pending_refresh_task: None,
            batch_stages: None,
            json_preview: None,
            batch_amount: None,
            batch_address: None,
            cached_base_nonce: None,
            cached_platform_balance: None,
            cached_core_balance: None,
        };
        screen.refresh_cached_balances();
        screen
    }

    /// Reset the address and amount inputs — called when AppContext switches network.
    pub(crate) fn invalidate_address_input(&mut self) {
        self.address_input = None;
        self.validated_source = None;
        self.amount_input = None;
        self.amount = None;
        self.cached_base_nonce = None;
        self.cached_platform_balance = None;
        self.cached_core_balance = None;
    }

    fn parse_repeat_count(&self) -> u32 {
        self.repeat_count_str
            .trim()
            .parse::<u32>()
            .unwrap_or(1)
            .clamp(1, 1000)
    }

    /// Returns the selected platform address, if a platform source is selected.
    fn selected_platform_address(&self) -> Option<PlatformAddress> {
        self.validated_source
            .as_ref()
            .and_then(|v| v.as_platform().copied())
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

            // Core balance — read from PlatformWallet (blocking_wallet_info for
            // per-address lookup, lock-free WalletBalance for total).
            if let Some(addr) = self.validated_source.as_ref().and_then(|v| v.as_core()) {
                let per_addr_balance = wallet.platform_wallet.as_ref().map(|pw| {
                    let info = pw.core().blocking_wallet_info();
                    platform_wallet::CoreAddressInfo::all_from_wallet_info(&info)
                        .into_iter()
                        .find(|a| &a.address == addr)
                        .map(|a| a.balance)
                        .unwrap_or(0)
                });
                self.cached_core_balance = Some(per_addr_balance.unwrap_or(0));
            } else {
                let pw_balance = wallet
                    .platform_wallet
                    .as_ref()
                    .map(|pw| pw.core().balance().total());
                self.cached_core_balance = Some(pw_balance.unwrap_or(0));
            }
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

    /// Build a single ShieldCredits task with optional nonce override.
    fn make_shield_credits_task(
        &self,
        amount: u64,
        addr: PlatformAddress,
        nonce_override: Option<u32>,
    ) -> BackendTask {
        BackendTask::ShieldedTask(ShieldedTask::ShieldCredits {
            seed_hash: self.seed_hash,
            amount,
            from_address: addr,
            nonce_override,
        })
    }

    /// Queue the next sequential batch task if any remain, using frozen batch parameters.
    fn queue_next_sequential(&mut self) {
        if self.batch_remaining > 0
            && let (Some(amount), Some(addr)) = (self.batch_amount, self.batch_address)
        {
            self.batch_remaining -= 1;
            self.pending_next_task = Some(self.make_shield_credits_task(amount, addr, None));
        }
    }

    /// Check if the sequential batch is complete and update status accordingly.
    fn check_batch_complete(&mut self, ctx: &Context) {
        if self.batch_stages.is_none()
            && self.batch_succeeded + self.batch_failed >= self.batch_total
        {
            self.status = Status::Complete;
            self.batch_amount = None;
            self.batch_address = None;
            MessageBanner::set_global(
                ctx,
                format!(
                    "Batch complete: {} succeeded, {} failed out of {}",
                    self.batch_succeeded, self.batch_failed, self.batch_total,
                ),
                if self.batch_failed > 0 {
                    MessageType::Warning
                } else {
                    MessageType::Success
                },
            );
        }
    }

    /// Spawn parallel batch: build proofs in parallel, broadcast in nonce order.
    fn spawn_parallel_batch(
        &mut self,
        ctx: &Context,
        amount: u64,
        addr: PlatformAddress,
        repeat: u32,
    ) {
        let base_nonce = match self.read_base_nonce() {
            Some(n) => n,
            None => {
                MessageBanner::set_global(
                    ctx,
                    "Could not read wallet data. Please try again.",
                    MessageType::Error,
                );
                return;
            }
        };
        let default_address = match self.app_context.shielded_default_address(&self.seed_hash) {
            Some(a) => a,
            None => {
                MessageBanner::set_global(
                    ctx,
                    "Shielded wallet not initialized. Please set up the shielded wallet first.",
                    MessageType::Error,
                );
                return;
            }
        };

        self.batch_total = repeat;
        self.batch_succeeded = 0;
        self.batch_failed = 0;
        self.batch_remaining = 0;
        self.status = Status::BatchInProgress;

        let stages: Vec<Arc<Mutex<ShieldStage>>> = (0..repeat)
            .map(|_| Arc::new(Mutex::new(ShieldStage::Queued)))
            .collect();
        self.batch_stages = Some(stages.clone());

        let app_ctx = self.app_context.clone();
        let seed_hash = self.seed_hash;

        tokio::spawn(async move {
            use crate::backend_task::shielded::bundle;
            use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;

            let build_futures: Vec<_> = (0..repeat)
                .map(|i| {
                    let app_ctx = app_ctx.clone();
                    let stage = stages[i as usize].clone();
                    let nonce = base_nonce + 1 + i;

                    async move {
                        if let Ok(mut guard) = stage.lock() {
                            *guard = ShieldStage::BuildingProof { nonce };
                        }

                        let result = tokio::task::spawn_blocking(move || {
                            bundle::build_shield_credit(
                                &app_ctx,
                                &seed_hash,
                                &default_address,
                                amount,
                                addr,
                                nonce,
                            )
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()));

                        match &result {
                            Ok(_) => {
                                if let Ok(mut guard) = stage.lock() {
                                    *guard = ShieldStage::WaitingToBroadcast;
                                }
                            }
                            Err(e) => {
                                if let Ok(mut guard) = stage.lock() {
                                    *guard = ShieldStage::Failed {
                                        error: e.clone(),
                                        st_json: None,
                                    };
                                }
                            }
                        }

                        result
                    }
                })
                .collect();

            let build_results = futures::future::join_all(build_futures).await;

            let sdk = { app_ctx.sdk.load().as_ref().clone() };

            for (i, result) in build_results.into_iter().enumerate() {
                let stage = &stages[i];
                match result {
                    Ok(state_transition) => {
                        if let Ok(mut guard) = stage.lock() {
                            *guard = ShieldStage::Broadcasting;
                        }

                        let st_repr: Option<String> =
                            serde_json::to_string_pretty(&state_transition)
                                .ok()
                                .or_else(|| {
                                    state_transition.serialize_to_bytes().map(hex::encode).ok()
                                });

                        let our_nonce = base_nonce + 1 + i as u32;
                        match state_transition.broadcast(&sdk, None).await {
                            Ok(_) => {
                                // Address nonces are strictly sequential — Platform
                                // requires block confirmation before accepting the next
                                // nonce. Retry wait_for_response to ensure the state
                                // transition is included in a block before proceeding.
                                let mut confirmed = false;
                                for attempt in 0..3 {
                                    match state_transition
                                        .wait_for_response::<StateTransitionProofResult>(&sdk, None)
                                        .await
                                    {
                                        Ok(_) => {
                                            confirmed = true;
                                            break;
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Batch item {} wait_for_response attempt {}: {e}",
                                                i + 1,
                                                attempt + 1
                                            );
                                            if attempt < 2 {
                                                tokio::time::sleep(Duration::from_secs(5)).await;
                                            }
                                        }
                                    }
                                }

                                if confirmed {
                                    app_ctx.bump_platform_address_nonce(&seed_hash, &addr);
                                    if let Ok(mut guard) = stage.lock() {
                                        *guard = ShieldStage::Complete;
                                    }
                                } else {
                                    // Cannot confirm — nonce chain is broken, cascade-fail
                                    tracing::error!(
                                        "Batch item {} broadcast succeeded but confirmation failed after 3 attempts",
                                        i + 1
                                    );
                                    if let Ok(mut guard) = stage.lock() {
                                        *guard = ShieldStage::Failed {
                                            error: "Broadcast succeeded but could not confirm. \
                                                 Remaining items skipped to avoid nonce errors."
                                                .to_string(),
                                            st_json: st_repr,
                                        };
                                    }
                                    for remaining in stages.iter().skip(i + 1) {
                                        if let Ok(mut s) = remaining.lock()
                                            && !s.is_terminal()
                                        {
                                            *s = ShieldStage::Failed {
                                                error: "Skipped: previous item not confirmed"
                                                    .to_string(),
                                                st_json: None,
                                            };
                                        }
                                    }
                                    break;
                                }
                            }
                            Err(e) => {
                                // Check for AddressInvalidNonceError via typed error chain.
                                // On any nonce mismatch, fail this item but continue to the
                                // next — Platform may catch up (nonce-ahead) or the next item
                                // may have a valid nonce (stale). Only cascade on non-nonce errors.
                                if let Some(expected) = extract_expected_nonce(&e) {
                                    tracing::warn!(
                                        "Batch item {} nonce mismatch: ours={}, Platform expects {}",
                                        i + 1,
                                        our_nonce,
                                        expected
                                    );
                                    if let Ok(mut guard) = stage.lock() {
                                        *guard = ShieldStage::Failed {
                                            error: format!(
                                                "Nonce mismatch: sent {}, Platform expects {}",
                                                our_nonce, expected
                                            ),
                                            st_json: st_repr,
                                        };
                                    }
                                    continue;
                                }

                                // Non-nonce error — fail and cascade
                                if let Ok(mut guard) = stage.lock() {
                                    *guard = ShieldStage::Failed {
                                        error: format!("Broadcast failed: {e}"),
                                        st_json: st_repr,
                                    };
                                }
                                for remaining in stages.iter().skip(i + 1) {
                                    if let Ok(mut s) = remaining.lock()
                                        && !s.is_terminal()
                                    {
                                        *s = ShieldStage::Failed {
                                            error: "Skipped: earlier nonce failed".to_string(),
                                            st_json: None,
                                        };
                                    }
                                }
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        for remaining in stages.iter().skip(i + 1) {
                            if let Ok(mut s) = remaining.lock()
                                && !s.is_terminal()
                            {
                                *s = ShieldStage::Failed {
                                    error: "Skipped: earlier nonce failed".to_string(),
                                    st_json: None,
                                };
                            }
                        }
                        break;
                    }
                }
            }
        });
    }

    /// Render the batch progress UI (used for Platform batch mode).
    fn render_batch_progress(&mut self, ui: &mut egui::Ui, ctx: &Context, action: &mut AppAction) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let stages_snapshot = self.batch_stages.clone();
        if let Some(stages) = stages_snapshot {
            let lock_stage = |s: &Arc<Mutex<ShieldStage>>| -> ShieldStage {
                s.lock()
                    .ok()
                    .map(|guard| guard.clone())
                    .unwrap_or(ShieldStage::Failed {
                        error: "Internal error: lock poisoned".to_string(),
                        st_json: None,
                    })
            };

            let all_done = stages.iter().all(|s| lock_stage(s).is_terminal());

            if !all_done {
                ctx.request_repaint_after(Duration::from_millis(100));
            }

            let succeeded = stages
                .iter()
                .filter(|s| matches!(lock_stage(s), ShieldStage::Complete))
                .count();
            let failed = stages
                .iter()
                .filter(|s| matches!(lock_stage(s), ShieldStage::Failed { .. }))
                .count();

            if all_done {
                if failed > 0 {
                    ui.colored_label(
                        DashColors::error_color(dark_mode),
                        format!(
                            "Batch complete: {} succeeded, {} failed out of {}",
                            succeeded,
                            failed,
                            stages.len(),
                        ),
                    );
                } else {
                    ui.colored_label(
                        DashColors::success_color(dark_mode),
                        format!("Batch complete: all {} succeeded", stages.len()),
                    );
                }
            } else {
                ui.label(format!(
                    "Succeeded {}/{}  Failed {}/{}",
                    succeeded,
                    stages.len(),
                    failed,
                    stages.len(),
                ));
            }
            ui.add_space(5.0);

            let rows: Vec<(ShieldStage, Option<String>)> = stages
                .iter()
                .map(|s| {
                    let s = lock_stage(s);
                    let json = if let ShieldStage::Failed { ref st_json, .. } = s {
                        st_json.clone()
                    } else {
                        None
                    };
                    (s, json)
                })
                .collect();

            let total = rows.len();
            let mut pending_json: Option<String> = None;

            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for (i, (stage, st_json)) in rows.iter().enumerate() {
                        let fraction = stage.progress_fraction();
                        let text = format!("[{}/{}] {}", i + 1, total, stage.label());

                        // Progress bar fills need vibrant, saturated colors for contrast
                        // against bar background — use static constants, not theme-aware
                        // text colors (which are muted/dark for readability on backgrounds).
                        let color = match stage {
                            ShieldStage::Queued => DashColors::GRAY,
                            ShieldStage::BuildingProof { .. } => DashColors::DASH_BLUE,
                            ShieldStage::WaitingToBroadcast => DashColors::INFO,
                            ShieldStage::Broadcasting => DashColors::WARNING,
                            ShieldStage::Complete => DashColors::SUCCESS,
                            ShieldStage::Failed { .. } => DashColors::ERROR,
                        };

                        if let Some(json_str) = st_json {
                            ui.horizontal(|ui| {
                                let btn_width = 100.0_f32;
                                let bar_width = (ui.available_width() - btn_width - 6.0).max(100.0);
                                ui.add_sized(
                                    [bar_width, 20.0],
                                    egui::ProgressBar::new(fraction).text(text).fill(color),
                                );
                                let btn = egui::Button::new(
                                    RichText::new("View JSON")
                                        .color(DashColors::WHITE)
                                        .size(12.0),
                                )
                                .fill(DashColors::BUTTON_DISABLED);
                                if ui
                                    .add_sized([btn_width, 20.0], btn)
                                    .on_hover_text("View state transition JSON")
                                    .clicked()
                                {
                                    pending_json = Some(json_str.clone());
                                }
                            });
                        } else {
                            let bar = egui::ProgressBar::new(fraction).text(text).fill(color);
                            if matches!(stage, ShieldStage::BuildingProof { .. }) {
                                ui.add(bar.animate(true));
                            } else {
                                ui.add(bar);
                            }
                        }
                    }
                });

            if let Some(json) = pending_json {
                self.json_preview = Some(json);
            }

            if all_done {
                ui.add_space(10.0);
                if ui.button("Done").clicked() {
                    *action = AppAction::PopScreen;
                }
            }
        } else {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(format!(
                    "Succeeded {}/{}  Failed {}/{}",
                    self.batch_succeeded, self.batch_total, self.batch_failed, self.batch_total,
                ));
            });
        }
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

        // Dispatch pending sequential task from previous frame
        if let Some(task) = self.pending_next_task.take() {
            action |= AppAction::BackendTask(task);
        }

        // Dispatch pending refresh task (sync notes after successful shield)
        if let Some(task) = self.pending_refresh_task.take() {
            action |= AppAction::BackendTask(task);
        }

        island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
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

            let is_busy =
                self.status == Status::WaitingForResult || self.status == Status::BatchInProgress;

            // Source address and amount inputs (disabled during batch)
            let source_kind = ui
                .add_enabled_ui(!is_busy, |ui| {
                    let addr_input = self.address_input.get_or_insert_with(|| {
                        let mut builder = AddressInput::new(self.app_context.network)
                            .with_address_kinds(&[AddressKind::Core, AddressKind::Platform])
                            .with_label("From address")
                            .with_hint_text("Select a platform or core wallet address")
                            .with_selection_only(true)
                            .with_balance_range(1..)
                            .with_exclude_change(true);

                        if let Ok(wallets) = self.app_context.wallets.read()
                            && let Some(wallet) = wallets.get(&self.seed_hash)
                        {
                            builder = builder.with_wallets(std::slice::from_ref(wallet));
                        }

                        builder
                    });
                    let resp = addr_input.show(ui);
                    if resp.inner.has_changed() {
                        resp.inner.update(&mut self.validated_source);
                        // Reset amount input when source changes (different balance constraints)
                        self.amount_input = None;
                        self.amount = None;
                        self.refresh_cached_balances();
                    }
                    ui.add_space(5.0);

                    // Show source-specific info based on selected address type
                    let source_kind = self.validated_source.as_ref().map(|v| v.kind());

                    match source_kind {
                        Some(AddressKind::Platform) => {
                            // Platform flow: show balance and nonce
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
                        Some(AddressKind::Core) => {
                            // Core flow: show balance (per-address or whole wallet)
                            let balance_duffs = self.read_core_balance_duffs();
                            let dash_balance = balance_duffs as f64 / 1e8;
                            let label = if self
                                .validated_source
                                .as_ref()
                                .and_then(|v| v.as_core())
                                .is_some()
                            {
                                format!("Available address balance: {:.8} DASH", dash_balance)
                            } else {
                                format!("Available core wallet balance: {:.8} DASH", dash_balance)
                            };
                            ui.label(
                                RichText::new(label).color(DashColors::success_color(dark_mode)),
                            );
                            ui.add_space(5.0);
                        }
                        _ => {}
                    }

                    // Amount input (only when a source address is selected)
                    if self.validated_source.is_some() {
                        let max_credits = match source_kind {
                            Some(AddressKind::Platform) => {
                                let base_fee =
                                    shielded_fee_for_actions(2, PlatformVersion::latest());
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

                        // Dev-mode batch controls (Platform flow only)
                        if source_kind == Some(AddressKind::Platform)
                            && self.status == Status::NotStarted
                        {
                            ui.feature_gated(&self.app_context, FeatureGate::DeveloperMode, |ui| {
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.label("Repeat");
                                    let te = egui::TextEdit::singleline(&mut self.repeat_count_str)
                                        .desired_width(50.0);
                                    ui.add(te);
                                    ui.label("times");
                                });
                                ui.checkbox(&mut self.parallel, "Parallel");
                            });
                        }
                    }

                    source_kind
                })
                .inner;

            ui.add_space(15.0);

            // Progress display
            if self.status == Status::BatchInProgress {
                self.render_batch_progress(ui, ctx, &mut action);
            } else if self.status == Status::WaitingForResult {
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

            // Buttons (only when not busy and source is selected)
            if !is_busy && self.status == Status::NotStarted && self.validated_source.is_some() {
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
                                let addr = self.selected_platform_address().unwrap();
                                let repeat = if self.app_context.is_developer_mode() {
                                    self.parse_repeat_count()
                                } else {
                                    1
                                };

                                // Balance check
                                if let Some(balance) = self.read_platform_balance() {
                                    let total = amount.saturating_mul(repeat as u64);
                                    if total > balance {
                                        let total_dash =
                                            total as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                        let balance_dash =
                                            balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                        MessageBanner::set_global(
                                            ctx,
                                            format!(
                                                "Insufficient balance: {repeat}x {:.8} DASH = {:.8} DASH total, but only {:.8} DASH available. Try a smaller amount.",
                                                amount as f64 / CREDITS_PER_DUFF as f64 / 1e8,
                                                total_dash,
                                                balance_dash,
                                            ),
                                            MessageType::Error,
                                        );
                                        return;
                                    }
                                }

                                if repeat <= 1 {
                                    self.status = Status::WaitingForResult;
                                    action = AppAction::BackendTask(
                                        self.make_shield_credits_task(amount, addr, None),
                                    );
                                } else if self.parallel {
                                    self.batch_amount = Some(amount);
                                    self.batch_address = Some(addr);
                                    self.spawn_parallel_batch(ctx, amount, addr, repeat);
                                } else {
                                    self.batch_total = repeat;
                                    self.batch_succeeded = 0;
                                    self.batch_failed = 0;
                                    self.batch_remaining = repeat - 1;
                                    self.batch_amount = Some(amount);
                                    self.batch_address = Some(addr);
                                    self.status = Status::BatchInProgress;
                                    action = AppAction::BackendTask(
                                        self.make_shield_credits_task(amount, addr, None),
                                    );
                                }
                            }
                            Some(AddressKind::Core) => {
                                let amount_duffs = amount / CREDITS_PER_DUFF;
                                let source_address = self
                                    .validated_source
                                    .as_ref()
                                    .and_then(|v| v.as_core())
                                    .cloned();
                                self.status = Status::WaitingForResult;
                                action = AppAction::BackendTask(BackendTask::ShieldedTask(
                                    ShieldedTask::ShieldFromAssetLock {
                                        seed_hash: self.seed_hash,
                                        amount_duffs,
                                        source_address,
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

        // JSON preview popup
        if let Some(json) = self.json_preview.clone() {
            let mut is_open = true;
            egui::Window::new("State Transition JSON")
                .collapsible(false)
                .resizable(true)
                .max_height(500.0)
                .max_width(800.0)
                .scroll(true)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .open(&mut is_open)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.monospace(&json);
                    });
                    ui.add_space(10.0);
                    if ui.button("Copy").clicked() {
                        let _ = crate::ui::helpers::copy_text_to_clipboard(&json);
                    }
                });
            if !is_open {
                self.json_preview = None;
            }
        }

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
                if self.status == Status::BatchInProgress {
                    self.batch_succeeded += 1;
                    self.check_batch_complete(&ctx);
                    if self.status == Status::BatchInProgress {
                        self.queue_next_sequential();
                    } else {
                        self.pending_refresh_task =
                            Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                                seed_hash: self.seed_hash,
                            }));
                    }
                } else {
                    self.status = Status::Complete;
                    let dash = amount as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                    MessageBanner::set_global(
                        &ctx,
                        format!("Successfully shielded {:.8} DASH", dash),
                        MessageType::Success,
                    );
                    self.pending_refresh_task =
                        Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                            seed_hash: self.seed_hash,
                        }));
                }
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
                self.pending_refresh_task =
                    Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                        seed_hash: self.seed_hash,
                    }));
            }
            _ => {}
        }
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        let ctx = self.app_context.egui_ctx().clone();
        if message_type == MessageType::Error {
            if self.status == Status::BatchInProgress {
                self.batch_failed += 1;
                self.check_batch_complete(&ctx);
                if self.status == Status::BatchInProgress {
                    self.queue_next_sequential();
                }
            } else if self.status == Status::WaitingForResult {
                self.status = Status::NotStarted;
            }
            // If status is Complete, leave it — the shield succeeded, a post-success
            // refresh failure (e.g. SyncNotes) is non-critical.
        }
    }
}
