use crate::app::AppAction;
use crate::backend_task::shielded::ShieldedTask;
use crate::backend_task::shielded::bundle::ShieldStage;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::Amount;
use crate::model::wallet::WalletSeedHash;
use crate::ui::components::ComponentResponse;
use crate::ui::components::address_input::AddressInput;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::serialization::PlatformSerializable;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use eframe::egui::{self, Context};
use egui::{Color32, RichText};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    error_message: Option<String>,
    success_message: Option<String>,
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
}

impl ShieldScreen {
    pub fn new(seed_hash: WalletSeedHash, app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            seed_hash,
            address_input: None,
            validated_source: None,
            amount_input: None,
            amount: None,
            status: Status::NotStarted,
            error_message: None,
            success_message: None,
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
        }
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

    /// Read the current nonce for the selected platform address from the wallet.
    fn read_base_nonce(&self) -> Option<u32> {
        let from_address = self.selected_platform_address()?;
        let wallets = self.app_context.wallets.read().unwrap();
        let wallet_arc = wallets.get(&self.seed_hash)?;
        let wallet = wallet_arc.read().unwrap();
        wallet
            .platform_address_info
            .iter()
            .find_map(|(addr, info)| {
                let platform_addr = PlatformAddress::try_from(addr.clone()).ok()?;
                if platform_addr == from_address {
                    Some(info.nonce)
                } else {
                    None
                }
            })
    }

    /// Read the current balance (in credits) for the selected platform address.
    fn read_platform_balance(&self) -> Option<u64> {
        let from_address = self.selected_platform_address()?;
        let wallets = self.app_context.wallets.read().unwrap();
        let wallet_arc = wallets.get(&self.seed_hash)?;
        let wallet = wallet_arc.read().unwrap();
        wallet
            .platform_address_info
            .iter()
            .find_map(|(addr, info)| {
                let platform_addr = PlatformAddress::try_from(addr.clone()).ok()?;
                if platform_addr == from_address {
                    Some(info.balance)
                } else {
                    None
                }
            })
    }

    /// Read the core wallet balance in duffs.
    fn read_core_balance_duffs(&self) -> u64 {
        let wallets = self.app_context.wallets.read().unwrap();
        wallets
            .get(&self.seed_hash)
            .map(|w| {
                let wallet = w.read().unwrap();
                wallet.total_balance_duffs()
            })
            .unwrap_or(0)
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

    /// Queue the next sequential batch task if any remain.
    fn queue_next_sequential(&mut self) {
        if self.batch_remaining > 0
            && let (Some(amount), Some(addr)) = (
                self.amount.as_ref().map(|a| a.value()),
                self.selected_platform_address(),
            )
        {
            self.batch_remaining -= 1;
            self.pending_next_task = Some(self.make_shield_credits_task(amount, addr, None));
        }
    }

    /// Check if the sequential batch is complete and update status accordingly.
    fn check_batch_complete(&mut self) {
        if self.batch_stages.is_none()
            && self.batch_succeeded + self.batch_failed >= self.batch_total
        {
            self.status = Status::Complete;
            self.success_message = Some(format!(
                "Batch complete: {} succeeded, {} failed out of {}",
                self.batch_succeeded, self.batch_failed, self.batch_total,
            ));
        }
    }

    /// Spawn parallel batch: build proofs in parallel, broadcast in nonce order.
    fn spawn_parallel_batch(&mut self, amount: u64, addr: PlatformAddress, repeat: u32) {
        let base_nonce = match self.read_base_nonce() {
            Some(n) => n,
            None => {
                self.error_message = Some("Could not read nonce from wallet".to_string());
                return;
            }
        };
        let default_address = match self.app_context.shielded_default_address(&self.seed_hash) {
            Some(a) => a,
            None => {
                self.error_message = Some("Shielded wallet not initialized".to_string());
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
                        *stage.lock().unwrap() = ShieldStage::BuildingProof { nonce };

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
                                *stage.lock().unwrap() = ShieldStage::WaitingToBroadcast;
                            }
                            Err(e) => {
                                *stage.lock().unwrap() = ShieldStage::Failed {
                                    error: e.clone(),
                                    st_json: None,
                                };
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
                        *stage.lock().unwrap() = ShieldStage::Broadcasting;

                        let st_repr: Option<String> =
                            serde_json::to_string_pretty(&state_transition)
                                .ok()
                                .or_else(|| {
                                    state_transition.serialize_to_bytes().map(hex::encode).ok()
                                });

                        match state_transition.broadcast(&sdk, None).await {
                            Ok(_) => {
                                let wait_ok = state_transition
                                    .wait_for_response::<StateTransitionProofResult>(&sdk, None)
                                    .await
                                    .is_ok();
                                if !wait_ok {
                                    tokio::time::sleep(Duration::from_secs(3)).await;
                                }
                                app_ctx.bump_platform_address_nonce(&seed_hash, &addr);
                                *stage.lock().unwrap() = ShieldStage::Complete;
                            }
                            Err(e) => {
                                *stage.lock().unwrap() = ShieldStage::Failed {
                                    error: format!("Broadcast failed: {e}"),
                                    st_json: st_repr,
                                };
                                for remaining in stages.iter().skip(i + 1) {
                                    let mut s = remaining.lock().unwrap();
                                    if !s.is_terminal() {
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
                            let mut s = remaining.lock().unwrap();
                            if !s.is_terminal() {
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
        let stages_snapshot = self.batch_stages.clone();
        if let Some(stages) = stages_snapshot {
            let all_done = stages.iter().all(|s| s.lock().unwrap().is_terminal());

            if !all_done {
                ctx.request_repaint_after(Duration::from_millis(100));
            }

            let succeeded = stages
                .iter()
                .filter(|s| matches!(*s.lock().unwrap(), ShieldStage::Complete))
                .count();
            let failed = stages
                .iter()
                .filter(|s| matches!(*s.lock().unwrap(), ShieldStage::Failed { .. }))
                .count();

            if all_done {
                if failed > 0 {
                    ui.colored_label(
                        Color32::from_rgb(255, 100, 100),
                        format!(
                            "Batch complete: {} succeeded, {} failed out of {}",
                            succeeded,
                            failed,
                            stages.len(),
                        ),
                    );
                } else {
                    ui.colored_label(
                        Color32::from_rgb(50, 180, 50),
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
                    let s = s.lock().unwrap().clone();
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

                        let color = match stage {
                            ShieldStage::Queued => Color32::GRAY,
                            ShieldStage::BuildingProof { .. } => {
                                crate::ui::theme::DashColors::DASH_BLUE
                            }
                            ShieldStage::WaitingToBroadcast => Color32::from_rgb(100, 180, 255),
                            ShieldStage::Broadcasting => Color32::from_rgb(255, 165, 0),
                            ShieldStage::Complete => Color32::from_rgb(50, 180, 50),
                            ShieldStage::Failed { .. } => Color32::from_rgb(220, 60, 60),
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
                                    RichText::new("View JSON").color(Color32::WHITE).size(12.0),
                                )
                                .fill(Color32::from_rgb(80, 80, 80));
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
            action = AppAction::BackendTask(task);
        }

        // Dispatch pending refresh task (sync notes after successful shield)
        if let Some(task) = self.pending_refresh_task.take() {
            action |= AppAction::BackendTask(task);
        }

        island_central_panel(ctx, |ui| {
            ui.heading("Shield");
            ui.add_space(10.0);
            ui.label("Move funds from a platform or core address into the shielded pool.");
            ui.add_space(15.0);

            // Error/success messages
            if let Some(err) = &self.error_message {
                ui.colored_label(Color32::from_rgb(255, 100, 100), err);
                ui.add_space(5.0);
            }
            if let Some(msg) = &self.success_message {
                ui.colored_label(Color32::DARK_GREEN, msg);
                ui.add_space(10.0);
                if ui.button("Done").clicked() {
                    action = AppAction::PopScreen;
                }
                return;
            }

            // Source address selection via AddressInput
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
            }
            ui.add_space(5.0);

            // Show source-specific info based on selected address type
            let source_kind = self.validated_source.as_ref().map(|v| v.kind());

            match source_kind {
                Some(AddressKind::Platform) => {
                    // Platform flow: show balance and nonce
                    if let Some(balance_credits) = self.read_platform_balance() {
                        let balance_dash = balance_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Available: {:.8} DASH", balance_dash))
                                    .color(Color32::from_rgb(100, 180, 100)),
                            );
                            if self.app_context.is_developer_mode()
                                && let Some(nonce) = self.read_base_nonce()
                            {
                                ui.label(
                                    RichText::new(format!("(nonce: {})", nonce))
                                        .color(Color32::GRAY)
                                        .small(),
                                );
                            }
                        });
                        ui.add_space(5.0);
                    }
                }
                Some(AddressKind::Core) => {
                    // Core flow: show wallet balance
                    let balance_duffs = self.read_core_balance_duffs();
                    let dash_balance = balance_duffs as f64 / 1e8;
                    ui.label(
                        RichText::new(format!(
                            "Available core wallet balance: {:.8} DASH",
                            dash_balance
                        ))
                        .color(Color32::from_rgb(100, 180, 100)),
                    );
                    ui.add_space(5.0);
                }
                _ => {}
            }

            // Amount input (only when a source address is selected)
            if self.validated_source.is_some() {
                let max_credits = match source_kind {
                    Some(AddressKind::Platform) => self.read_platform_balance(),
                    Some(AddressKind::Core) => {
                        Some(self.read_core_balance_duffs() * CREDITS_PER_DUFF)
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
                if self.app_context.is_developer_mode()
                    && source_kind == Some(AddressKind::Platform)
                    && self.status == Status::NotStarted
                {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Repeat");
                        let te = egui::TextEdit::singleline(&mut self.repeat_count_str)
                            .desired_width(50.0);
                        ui.add(te);
                        ui.label("times");
                    });
                    ui.checkbox(&mut self.parallel, "Parallel");
                }
            }

            ui.add_space(15.0);

            // Progress display
            let is_busy =
                self.status == Status::WaitingForResult || self.status == Status::BatchInProgress;

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
                let can_confirm = self.amount.as_ref().map(|a| a.value()).is_some();

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
                                    .color(Color32::WHITE)
                                    .size(16.0),
                            )
                            .fill(crate::ui::theme::DashColors::DASH_BLUE),
                        )
                        .clicked()
                        && let Some(amount) = self.amount.as_ref().map(|a| a.value())
                    {
                        self.error_message = None;

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
                                        self.error_message = Some(format!(
                                            "Insufficient balance: {repeat}x {:.8} DASH = {:.8} DASH total, but only {:.8} DASH available",
                                            amount as f64 / CREDITS_PER_DUFF as f64 / 1e8,
                                            total_dash,
                                            balance_dash,
                                        ));
                                        return;
                                    }
                                }

                                if repeat <= 1 {
                                    self.status = Status::WaitingForResult;
                                    action = AppAction::BackendTask(
                                        self.make_shield_credits_task(amount, addr, None),
                                    );
                                } else if self.parallel {
                                    self.spawn_parallel_batch(amount, addr, repeat);
                                } else {
                                    self.batch_total = repeat;
                                    self.batch_succeeded = 0;
                                    self.batch_failed = 0;
                                    self.batch_remaining = repeat - 1;
                                    self.status = Status::BatchInProgress;
                                    action = AppAction::BackendTask(
                                        self.make_shield_credits_task(amount, addr, None),
                                    );
                                }
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

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount }
                if seed_hash == self.seed_hash =>
            {
                if self.status == Status::BatchInProgress {
                    self.batch_succeeded += 1;
                    self.check_batch_complete();
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
                    self.success_message = Some(format!("Successfully shielded {:.8} DASH", dash));
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
                self.success_message = Some(format!(
                    "Successfully shielded {:.8} DASH from core wallet",
                    dash
                ));
                self.pending_refresh_task =
                    Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                        seed_hash: self.seed_hash,
                    }));
            }
            _ => {}
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Error => {
                if self.status == Status::BatchInProgress {
                    self.batch_failed += 1;
                    self.check_batch_complete();
                    if self.status == Status::BatchInProgress {
                        self.queue_next_sequential();
                    }
                } else {
                    self.status = Status::NotStarted;
                    self.error_message = Some(message.to_string());
                }
            }
            _ => {
                self.success_message = Some(message.to_string());
            }
        }
    }
}
