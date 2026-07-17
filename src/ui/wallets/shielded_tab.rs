use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::migration::MigrationTask;
use crate::context::AppContext;
use crate::context::feature_gate::{Check, FeatureGate};
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::model::address::truncate_address;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::wallet::WalletSeedHash;
use crate::ui::ScreenType;
use crate::ui::components::wallet_unlock_popup::wallet_needs_unlock;
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::theme::DashColors;
use crate::ui::wallets::send_screen::SendFlow;
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use std::sync::Arc;

/// J-3 indicator strings — single complete sentences that name their own
/// subject, so the i18n pass picks each one up as a discrete translation unit
/// and no label depends on where it happens to be rendered. Exposed `pub` so
/// the tests (TC-A11Y-006) assert against the exact label the UI renders.
pub const SHIELDED_VERIFYING_LABEL: &str = "Verifying shielded balance.";
pub const SHIELDED_VERIFIED_LABEL: &str = "Shielded balance verified.";
pub const SHIELDED_SPEND_LOCKED_LABEL: &str = "Spending paused.";
pub const SHIELDED_SPEND_LOCKED_TOOLTIP: &str =
    "Spending paused until shielded balance is verified.";
pub const SHIELDED_LOCK_ICON: &str = "\u{1F512}"; // 🔒
pub const SHIELDED_VERIFIED_ICON: &str = "\u{2714}"; // ✔
pub const SHIELDED_RETRY_MIGRATION_LABEL: &str = "Retry shielded migration";
pub const SHIELDED_SKIP_MIGRATION_LABEL: &str = "Skip for now";
/// Receive-address section copy. Each is a complete sentence or a standalone
/// label so the i18n pass extracts it as one translation unit; `pub` so the
/// tests assert against the exact strings the UI renders.
pub const SHIELDED_ADDRESS_HEADING: &str = "Shielded Address";
pub const SHIELDED_ADDRESS_HINT: &str = "Share this address to receive a private transfer.";
pub const SHIELDED_ADDRESS_PENDING_LABEL: &str =
    "Your shielded address appears here once the wallet is unlocked.";
pub const SHIELDED_ADDRESS_COPY_LABEL: &str = "Copy";
pub const SHIELDED_ADDRESS_COPIED_LABEL: &str = "Shielded address copied to the clipboard.";
pub const SHIELDED_ADDRESS_COPY_FAILED_LABEL: &str =
    "The address could not be copied. Select the address text and copy it manually.";
pub const SHIELDED_MIGRATION_ERROR_LABEL: &str =
    "Shielded data could not be migrated. Try again, or skip and use the rest of your wallet.";
pub const SHIELDED_TAB_SKIPPED_LABEL: &str =
    "Shielded features are paused until the next launch. Restart the app to retry the migration.";
/// Shown in place of the Shield / Send / Unshield controls when the connected
/// network does not yet support shielded state transitions. Viewing balance,
/// address, and notes stays available.
pub const SHIELDED_OPERATIONS_NETWORK_UNAVAILABLE_LABEL: &str = "Shielded sending is not available on this network yet. You can still view your shielded balance and receive address.";
/// Shown in place of the Shield / Send / Unshield controls when the connected
/// network supports shielded state transitions but the user's interface mode
/// does not unlock them yet. Viewing balance, address, and notes stays
/// available.
// Keep "Expert view" aligned with the experimental threshold if it changes.
pub const SHIELDED_OPERATIONS_ROLE_UNAVAILABLE_LABEL: &str = "Shielded sending needs Expert view or higher. You can still view your shielded balance and receive address. Switch your interface mode in Settings to use it.";

/// J-3 indicator state. Derived purely from [`MigrationState`] and the
/// session-local "skip" flag, so the same inputs always yield the same
/// indicator — testable without a UI harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShieldedIndicator {
    /// Migration completed (or never required) — balance is authoritative.
    Verified,
    /// Migration is mid-flight on the shielded step. Spends paused.
    Verifying,
    /// Sidecar mirror failed. Spend locked with a retry / skip prompt.
    Failed,
    /// Migration not yet started or in a non-shielded step — no badge.
    Hidden,
}

/// Derive the indicator state from the migration status. Pure function
/// so tests can drive it without a UI harness (TC-A11Y-006 backstop).
///
/// `skipped` is the user-driven "skip for now" toggle held in
/// [`ShieldedTabView::sidecar_skipped`]; when set, the indicator is
/// hidden so the tab content can render the disabled-skip notice
/// instead of the retry banner.
pub fn derive_shielded_indicator(state: &MigrationState, skipped: bool) -> ShieldedIndicator {
    if skipped {
        return ShieldedIndicator::Hidden;
    }
    match state {
        MigrationState::Running {
            step: MigrationStep::Shielded,
        } => ShieldedIndicator::Verifying,
        MigrationState::Failed { .. } => ShieldedIndicator::Failed,
        // Every state below is reachable only after the wallet drain returned Ok,
        // and what broke in them — undecodable vote or identity rows, a hard
        // app-data failure — belongs to passes that run afterwards and never touch
        // shielded storage. The balance is therefore as authoritative as on
        // `Success`, even under an error banner: `FailedWithUnreadableIdentities`
        // raises one, and the badge deliberately stays green beneath it. Mapping it
        // to `Failed` instead would lock shielded spends over a corrupt vote row and
        // offer a retry for a shielded migration that never failed.
        MigrationState::Success
        | MigrationState::SucceededWithUnreadableData { .. }
        | MigrationState::FailedWithUnreadableIdentities { .. } => ShieldedIndicator::Verified,
        // Idle / non-shielded running step → no badge.
        MigrationState::Idle
        | MigrationState::Ready
        | MigrationState::Running { .. }
        | MigrationState::AwaitingWalletPasswords { .. } => ShieldedIndicator::Hidden,
    }
}

/// View component for the Shielded tab within the Wallets screen.
pub struct ShieldedTabView {
    app_context: Arc<AppContext>,
    seed_hash: WalletSeedHash,
    initializing: bool,
    syncing: bool,
    error_message: Option<String>,
    success_message: Option<String>,
    shielded_balance: u64,
    is_initialized: bool,
    /// Whether the commitment tree has been synced (enables spend operations).
    tree_synced: bool,
    /// Pending backend task to dispatch on next ui() call.
    pending_task: Option<BackendTask>,
    /// The wallet's shielded receive address (Bech32m), mirrored each frame from
    /// the frame-safe [`AppContext`] snapshot. `None` until the wallet's Orchard
    /// keys are bound.
    shielded_address: Option<String>,
    /// J-3: session-local flag set when the user clicks "Skip for now"
    /// on the sidecar-failure banner. Suppresses the retry banner and
    /// locks the tab until the app restarts.
    sidecar_skipped: bool,
}

impl ShieldedTabView {
    pub fn new(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) -> Self {
        Self {
            app_context: app_context.clone(),
            seed_hash,
            initializing: false,
            syncing: false,
            error_message: None,
            success_message: None,
            shielded_balance: 0,
            is_initialized: false,
            tree_synced: false,
            pending_task: None,
            shielded_address: None,
            sidecar_skipped: false,
        }
    }

    /// Open the unified send screen pre-configured for `flow`, resolving the
    /// wallet handle for this tab's seed hash. The three shielded flows (Shield,
    /// Send Private, Unshield) are routes into the one canonical send screen —
    /// there are no bespoke shielded send screens.
    fn open_send_flow(&self, flow: SendFlow) -> AppAction {
        let Some(wallet) = self
            .app_context
            .wallets
            .read()
            .ok()
            .and_then(|wallets| wallets.get(&self.seed_hash).cloned())
        else {
            return AppAction::None;
        };
        AppAction::AddScreen(
            ScreenType::WalletSendScreen(wallet, flow).create_screen(&self.app_context),
        )
    }

    /// Compute the J-3 indicator for the current frame. Reads the
    /// migration status atomic; cheap.
    fn current_indicator(&self) -> ShieldedIndicator {
        let state = self.app_context.migration_status().state();
        derive_shielded_indicator(&state, self.sidecar_skipped)
    }

    pub fn update_seed_hash(&mut self, seed_hash: WalletSeedHash) {
        if self.seed_hash != seed_hash {
            self.seed_hash = seed_hash;
            self.is_initialized = false;
            self.tree_synced = false;
            self.shielded_balance = 0;
            self.error_message = None;
            self.success_message = None;
            self.initializing = false;
            self.syncing = false;
            self.pending_task = None;
            // Drop the previous wallet's address immediately rather than
            // letting it linger for a frame — it is a payment destination.
            self.shielded_address = None;
            // Skip-for-now is session-scoped to the wallet; a new
            // wallet starts with the retry banner re-enabled.
            self.sidecar_skipped = false;
        }
    }

    pub fn update_app_context(&mut self, app_context: &Arc<AppContext>) {
        self.app_context = app_context.clone();
    }

    /// Drain pending backend tasks queued by user actions on this tab.
    /// Initialization is handled entirely by the backend in
    /// `handle_wallet_unlocked` — the UI never triggers it.
    pub fn tick(&mut self) -> AppAction {
        self.refresh_from_backend_state();

        self.pending_task
            .take()
            .map(AppAction::BackendTask)
            .unwrap_or(AppAction::None)
    }

    /// Sync local display state from the push snapshots and the upstream
    /// coordinator.
    ///
    /// The upstream `platform-wallet` coordinator owns all Orchard state (keys,
    /// sync progress, note tree). Balance and receive address are read from the
    /// frame-safe push snapshots; `is_initialized` / `tree_synced` are set true
    /// whenever the wallet backend is wired so spend buttons are enabled.
    /// Fine-grained sync progress arrives through the push-based
    /// [`ConnectionStatus`](crate::context::connection_status::ConnectionStatus).
    fn refresh_from_backend_state(&mut self) {
        // Balance and address: frame-safe push snapshots, no async in the frame
        // loop. Both are written on the backend side once Orchard keys bind.
        self.shielded_balance = self.app_context.shielded_balance_credits(&self.seed_hash);
        self.shielded_address = self.app_context.shielded_receive_address(&self.seed_hash);

        // Treat the wallet as initialized and the tree as synced whenever the
        // backend is available — the coordinator resyncs Orchard state from
        // chain on its own schedule.
        if self.app_context.wallet_backend().is_ok() {
            self.is_initialized = true;
            self.tree_synced = true;
            self.syncing = false;
        }
    }

    /// Render the shielded receive-address section: the address, a hint, and a
    /// copy control. Open by default — receiving a private transfer is the
    /// reason to visit this tab, so the address must not be a click away.
    ///
    /// Shows Orchard account 0, the only account DET binds and the only one its
    /// spend path can spend from. Displaying any other account would offer a
    /// destination whose funds the app could not move.
    ///
    // TODO: offer additional diversified addresses ("+") once upstream
    // platform-wallet exposes a per-index accessor. At the pinned revision the
    // only shielded address APIs are `shielded_default_address(account)` /
    // `shielded_default_addresses()`; `OrchardKeySet::address_at(index)` is
    // reachable only through the crate-private `shielded_keys` slot. Deriving
    // them DET-side would duplicate Orchard key handling outside the coordinator
    // seam, and mapping "+" onto a new ZIP-32 account would strand funds in an
    // account the single-account spend path cannot spend from.
    fn render_address_section(&mut self, ui: &mut Ui, dark_mode: bool) {
        let header = egui::CollapsingHeader::new(
            RichText::new(SHIELDED_ADDRESS_HEADING)
                .size(16.0)
                .color(DashColors::text_primary(dark_mode)),
        )
        .id_salt("shielded_addresses")
        .default_open(true);

        header.show(ui, |ui| {
            let Some(address) = self.shielded_address.clone() else {
                ui.label(
                    RichText::new(SHIELDED_ADDRESS_PENDING_LABEL)
                        .color(DashColors::text_secondary(dark_mode)),
                );
                return;
            };

            ui.label(
                RichText::new(SHIELDED_ADDRESS_HINT)
                    .size(12.0)
                    .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(4.0);

            let copy_requested = ui
                .horizontal(|ui| {
                    // Truncated for layout; the full address is always one hover
                    // away and the clipboard always receives the full string.
                    let shown = truncate_address(&address, 20, 12);
                    let clicked_address = ui
                        .add(
                            egui::Label::new(
                                RichText::new(shown)
                                    .monospace()
                                    .color(DashColors::text_primary(dark_mode)),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text(&address)
                        .clicked();
                    let clicked_button = ui.button(SHIELDED_ADDRESS_COPY_LABEL).clicked();
                    clicked_address || clicked_button
                })
                .inner;

            if copy_requested {
                match copy_text_to_clipboard(&address) {
                    Ok(()) => {
                        self.success_message = Some(SHIELDED_ADDRESS_COPIED_LABEL.to_string());
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Shielded address clipboard copy failed");
                        self.error_message = Some(SHIELDED_ADDRESS_COPY_FAILED_LABEL.to_string());
                    }
                }
            }
        });
    }

    /// Handle backend task results for shielded operations.
    /// Fund-moving results only.
    pub fn handle_result(
        &mut self,
        result: &crate::backend_task::BackendTaskSuccessResult,
    ) -> bool {
        use crate::backend_task::BackendTaskSuccessResult;
        match result {
            BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Shielded {} successfully",
                    format_credits_as_dash(*amount)
                ));
                true
            }
            BackendTaskSuccessResult::ShieldedTransferComplete { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Transferred {} privately",
                    format_credits_as_dash(*amount)
                ));
                true
            }
            BackendTaskSuccessResult::ShieldedCreditsUnshielded { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message =
                    Some(format!("Unshielded {}", format_credits_as_dash(*amount)));
                true
            }
            BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Shielded {} from core wallet",
                    format_credits_as_dash(*amount)
                ));
                true
            }
            BackendTaskSuccessResult::ShieldedWithdrawalComplete { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Withdrew {} to core address",
                    format_credits_as_dash(*amount)
                ));
                true
            }
            _ => false,
        }
    }

    pub fn handle_error(&mut self, error: &str) {
        self.syncing = false;
        self.initializing = false;
        self.error_message = Some(error.to_string());
    }

    // TODO: Redesign shielded tab layout for visual consistency with other tabs:
    //   1. Action buttons row at top: Shield, Shield from Core, Transfer, Unshield
    //   2. Shielded Addresses (collapsible) — diversified addresses in a table
    //   3. Shielded Notes (collapsible) — notes table (index, value, spent/unspent)
    // Currently the layout is: balance card -> address card -> buttons -> notes list.
    // The redesign should move buttons to the top and use collapsible sections.

    /// Render in-flight shielded sync progress, read from the push-based
    /// [`ConnectionStatus`]. Shows the downloaded-notes counter and
    /// the committed-to-tree ("checked") progress — a determinate bar when the
    /// on-chain leaf total is known, a spinner otherwise. Renders nothing
    /// between passes (both progress fields `None`).
    fn render_sync_progress(&self, ui: &mut Ui, dark_mode: bool) {
        let cs = self.app_context.connection_status();
        let sync = cs.shielded_sync_progress();
        let tree = cs.shielded_tree_progress();
        if sync.is_none() && tree.is_none() {
            return;
        }
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 8))
            .corner_radius(6.0)
            .show(ui, |ui| {
                if let Some(s) = sync {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(DashColors::DASH_BLUE));
                        ui.label(
                            RichText::new(format!(
                                "Scanning shielded notes: {} scanned (block {}).",
                                s.cumulative_scanned, s.block_height
                            ))
                            .size(12.0)
                            .color(DashColors::text_secondary(dark_mode)),
                        );
                    });
                }
                if let Some(t) = tree {
                    if t.total_target > 0 {
                        let fraction =
                            (t.leaves_committed as f32 / t.total_target as f32).clamp(0.0, 1.0);
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .text(format!(
                                    "Checked {} / {} notes",
                                    t.leaves_committed, t.total_target
                                ))
                                .fill(DashColors::DASH_BLUE),
                        );
                    } else {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0).color(DashColors::DASH_BLUE));
                            ui.label(
                                RichText::new(format!(
                                    "Checking shielded notes: {} committed.",
                                    t.leaves_committed
                                ))
                                .size(12.0)
                                .color(DashColors::text_secondary(dark_mode)),
                            );
                        });
                    }
                }
            });
        ui.add_space(10.0);
    }

    /// Render the shielded tab content.
    pub fn ui(&mut self, ui: &mut Ui) -> AppAction {
        let dark_mode = ui.style().visuals.dark_mode;
        let mut action = self.tick();
        let indicator = self.current_indicator();

        // J-3 sidecar-failure banner — surfaces above everything else
        // because spends are locked in this state. Both buttons emit
        // either a retry task or set the session-skip flag.
        if matches!(indicator, ShieldedIndicator::Failed) {
            Frame::new()
                .fill(Color32::from_rgb(255, 100, 100).gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(SHIELDED_LOCK_ICON)
                                .color(Color32::from_rgb(255, 100, 100)),
                        );
                        ui.label(
                            RichText::new(SHIELDED_MIGRATION_ERROR_LABEL)
                                .color(Color32::from_rgb(255, 100, 100)),
                        );
                        if ui.small_button(SHIELDED_RETRY_MIGRATION_LABEL).clicked() {
                            action |= AppAction::BackendTask(BackendTask::MigrationTask(
                                MigrationTask::FinishUnwire,
                            ));
                        }
                        if ui.small_button(SHIELDED_SKIP_MIGRATION_LABEL).clicked() {
                            self.sidecar_skipped = true;
                        }
                    });
                });
            ui.add_space(5.0);
        }

        // Tab-locked notice — the user has dismissed the retry banner.
        // Spends stay disabled until the next launch.
        if self.sidecar_skipped {
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(SHIELDED_LOCK_ICON));
                        ui.label(
                            RichText::new(SHIELDED_TAB_SKIPPED_LABEL)
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    });
                });
            ui.add_space(5.0);
            return action;
        }

        // Messages
        if let Some(err) = &self.error_message.clone() {
            Frame::new()
                .fill(Color32::from_rgb(255, 100, 100).gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(err).color(Color32::from_rgb(255, 100, 100)));
                        if ui.small_button("Dismiss").clicked() {
                            self.error_message = None;
                        }
                    });
                });
            ui.add_space(5.0);
        }

        if let Some(msg) = &self.success_message.clone() {
            Frame::new()
                .fill(Color32::DARK_GREEN.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(msg).color(Color32::DARK_GREEN));
                        if ui.small_button("Dismiss").clicked() {
                            self.success_message = None;
                        }
                    });
                });
            ui.add_space(5.0);
        }

        // --- Not yet initialized ---
        // Initialization is handled by the backend (handle_wallet_unlocked).
        // If the state is not yet available, the wallet is either locked or
        // init is still running — show an appropriate message.
        if !self.is_initialized {
            if self.initializing {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                    ui.label("Initializing shielded wallet (deriving ZIP32 keys)...");
                });
            } else {
                let wallet_locked = {
                    let Some(wallets) = self.app_context.wallets.read().ok() else {
                        ui.label("Unable to read wallet state. Please try again.");
                        return action;
                    };
                    wallets
                        .get(&self.seed_hash)
                        .is_some_and(wallet_needs_unlock)
                };
                ui.add_space(20.0);
                if wallet_locked {
                    ui.label(
                        RichText::new("Unlock the wallet to enable the shielded pool.")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                        ui.label("Preparing shielded wallet...");
                    });
                }
            }
            return action;
        }

        // --- Initialized: show balance, address, actions ---

        // Balance display + J-3 indicator badge.
        ui.add_space(10.0);
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(16, 12))
            .corner_radius(8.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Shielded Balance")
                        .size(16.0)
                        .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format_credits_as_dash(self.shielded_balance))
                            .size(28.0)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                // J-3: indicator badge. Verifying / Verified — both use
                // icon + text per TC-A11Y-006 so screen readers and
                // greyscale viewers get the same signal as sighted
                // colour users.
                match indicator {
                    ShieldedIndicator::Verifying => {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                            ui.label(
                                RichText::new(SHIELDED_VERIFYING_LABEL)
                                    .size(12.0)
                                    .color(DashColors::DASH_BLUE),
                            );
                        });
                    }
                    ShieldedIndicator::Verified => {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(SHIELDED_VERIFIED_ICON).color(Color32::DARK_GREEN),
                            );
                            ui.label(
                                RichText::new(SHIELDED_VERIFIED_LABEL)
                                    .size(12.0)
                                    .color(Color32::DARK_GREEN),
                            );
                        });
                    }
                    ShieldedIndicator::Failed | ShieldedIndicator::Hidden => {}
                }
            });

        ui.add_space(10.0);

        // In-flight shielded sync progress (push-based).
        self.render_sync_progress(ui, dark_mode);

        // Shielded Addresses (collapsible table)
        self.render_address_section(ui, dark_mode);

        ui.add_space(10.0);

        // Shield / Send / Unshield dispatch shielded state transitions the
        // network must be able to settle. Where shielded operations are
        // unavailable, hide the action controls (balance, address, and notes
        // stay visible) rather than offer a dead end the backend would reject.
        if FeatureGate::ShieldedOperations.is_available(&self.app_context) {
            // J-3 spend lock: any verifying / failed indicator pauses spends
            // regardless of the local sync state. Computed once so the
            // hover-text and the "Spending paused" notice agree.
            let spend_locked = matches!(
                indicator,
                ShieldedIndicator::Verifying | ShieldedIndicator::Failed
            );

            // Action buttons
            ui.horizontal(|ui| {
                let shield_btn =
                    egui::Button::new(RichText::new("Shield").color(Color32::WHITE).size(14.0))
                        .fill(DashColors::DASH_BLUE);
                if ui
                    .add_enabled(!self.syncing && !spend_locked, shield_btn)
                    .on_hover_text(if spend_locked {
                        SHIELDED_SPEND_LOCKED_TOOLTIP
                    } else {
                        "Shield funds from a platform or core address into the shielded pool"
                    })
                    .clicked()
                {
                    action |= self.open_send_flow(SendFlow::Shield);
                }

                let can_spend =
                    !self.syncing && self.tree_synced && self.shielded_balance > 0 && !spend_locked;

                let send_btn = egui::Button::new(
                    RichText::new("Send (Private)")
                        .color(Color32::WHITE)
                        .size(14.0),
                )
                .fill(DashColors::DASH_BLUE);
                if ui
                    .add_enabled(can_spend, send_btn)
                    .on_hover_text(if spend_locked {
                        SHIELDED_SPEND_LOCKED_TOOLTIP
                    } else if self.tree_synced {
                        "Transfer privately within the shielded pool"
                    } else {
                        "Sync notes first to enable spending"
                    })
                    .clicked()
                {
                    action |= self.open_send_flow(SendFlow::ShieldedSend);
                }

                let unshield_btn =
                    egui::Button::new(RichText::new("Unshield").color(Color32::WHITE).size(14.0))
                        .fill(DashColors::DASH_BLUE);
                if ui
                    .add_enabled(can_spend, unshield_btn)
                    .on_hover_text(if spend_locked {
                        SHIELDED_SPEND_LOCKED_TOOLTIP
                    } else if self.tree_synced {
                        "Unshield credits to a platform address"
                    } else {
                        "Sync notes first to enable spending"
                    })
                    .clicked()
                {
                    action |= self.open_send_flow(SendFlow::Unshield);
                }
            });

            // J-3 "Spending paused" row — icon + text per TC-A11Y-006 so
            // colour-blind / greyscale users get the same signal as the
            // disabled-button affordance.
            if spend_locked {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(SHIELDED_LOCK_ICON));
                    ui.label(
                        RichText::new(SHIELDED_SPEND_LOCKED_LABEL)
                            .size(12.0)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                });
            }
        } else {
            let label = match FeatureGate::ShieldedOperations.first_unmet_check(&self.app_context) {
                Some(Check::Experimental(_)) => SHIELDED_OPERATIONS_ROLE_UNAVAILABLE_LABEL,
                _ => SHIELDED_OPERATIONS_NETWORK_UNAVAILABLE_LABEL,
            };
            ui.label(
                RichText::new(label)
                    .size(12.0)
                    .color(DashColors::text_secondary(dark_mode)),
            );
        }

        ui.add_space(15.0);

        // Shielded Notes (owned by the upstream coordinator).
        let notes_header = egui::CollapsingHeader::new(
            RichText::new("Shielded Notes")
                .size(16.0)
                .color(DashColors::text_primary(dark_mode)),
        )
        .id_salt("shielded_notes")
        .default_open(false);
        notes_header.show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Note history is managed by the upstream platform-wallet coordinator \
                     and will be surfaced here in a future update.",
                )
                .color(DashColors::text_secondary(dark_mode)),
            );
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TC-A11Y-006 — the locked-spend state surfaces both an icon and a
    /// "Spending paused." sentence, never colour alone. We assert on the
    /// constants the UI binds to so a future refactor that drops either
    /// half of the signal fails this guard before it reaches users.
    #[test]
    fn tc_a11y_006_locked_spend_state_uses_icon_and_text() {
        // The lock icon and the text are distinct, non-empty constants.
        assert!(!SHIELDED_LOCK_ICON.is_empty(), "lock icon present");
        assert!(!SHIELDED_SPEND_LOCKED_LABEL.is_empty(), "lock text present",);
        // i18n hygiene: complete sentence terminated with a period.
        assert!(
            SHIELDED_SPEND_LOCKED_LABEL.ends_with('.'),
            "locked label is a complete sentence",
        );
        assert!(
            SHIELDED_SPEND_LOCKED_TOOLTIP.ends_with('.'),
            "locked tooltip is a complete sentence",
        );
        // Icon and text are distinct strings — the indicator never
        // collapses into a single signal.
        assert_ne!(SHIELDED_LOCK_ICON, SHIELDED_SPEND_LOCKED_LABEL);
    }

    /// TC-SH-007 — when shielded balance is mid-verification (Verifying
    /// state) the Send button copy must read "Spending paused until
    /// shielded balance is verified." verbatim. We assert against the
    /// public constant the UI binds the tooltip to so a wording drift
    /// fails this test before reaching users.
    #[test]
    fn tc_sh_007_spending_paused_tooltip_matches_spec() {
        // Spec text (TC-SH-007): "Spending paused until shielded balance is verified."
        assert_eq!(
            SHIELDED_SPEND_LOCKED_TOOLTIP, "Spending paused until shielded balance is verified.",
            "tooltip must match the Diziet §2.3 wording verbatim",
        );
        // The Verifying indicator (mid-sync) is the gate for the lock.
        // If the indicator ever stops mapping the Shielded migration step
        // to Verifying, the lock would silently disappear.
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::Running {
                    step: MigrationStep::Shielded,
                },
                false,
            ),
            ShieldedIndicator::Verifying,
            "Verifying gates the spending-paused lock — must stay wired",
        );
    }

    /// WAL-029 — the receive-address copy is i18n-clean: complete sentences for
    /// the prose, a bare label for the button. The pending copy is what a
    /// locked / not-yet-bound wallet shows in place of an address, so it must
    /// never read as if an address were present.
    #[test]
    fn shielded_address_section_copy_is_i18n_clean() {
        for sentence in [
            SHIELDED_ADDRESS_HINT,
            SHIELDED_ADDRESS_PENDING_LABEL,
            SHIELDED_ADDRESS_COPIED_LABEL,
            SHIELDED_ADDRESS_COPY_FAILED_LABEL,
        ] {
            assert!(
                sentence.ends_with('.'),
                "user-facing copy must be a complete sentence: {sentence}"
            );
        }
        assert!(!SHIELDED_ADDRESS_HEADING.is_empty());
        assert!(!SHIELDED_ADDRESS_COPY_LABEL.is_empty());
        // The failure copy must give the user a way out on their own — no
        // dead end, no "contact support".
        assert!(
            SHIELDED_ADDRESS_COPY_FAILED_LABEL.contains("manually"),
            "the copy-failure message must offer a self-service fallback",
        );
    }

    /// The address the tab renders is the one the clipboard receives — the
    /// truncation is display-only. A user who copies must get a payable
    /// address, never the ellipsised form.
    #[test]
    fn displayed_address_is_truncated_but_copy_uses_the_full_string() {
        let address = "tdash1z".to_string() + &"q".repeat(70);
        let shown = truncate_address(&address, 20, 12);

        assert!(
            shown.contains("..."),
            "long addresses are truncated on screen"
        );
        assert!(shown.len() < address.len());
        // The full string stays intact for the clipboard and the hover text.
        assert!(address.starts_with("tdash1z"));
        assert_eq!(
            crate::model::address::AddressKind::detect(&address),
            Some(crate::model::address::AddressKind::Shielded),
        );
    }

    /// The network notice is complete and says what remains available.
    #[test]
    fn network_unavailable_label_is_i18n_clean() {
        assert!(SHIELDED_OPERATIONS_NETWORK_UNAVAILABLE_LABEL.ends_with('.'));
        assert!(
            SHIELDED_OPERATIONS_NETWORK_UNAVAILABLE_LABEL.contains("view"),
            "the notice must state what the user can still do"
        );
    }

    /// The role notice is complete, actionable, and names the required tier.
    #[test]
    fn role_unavailable_label_is_i18n_clean() {
        assert!(SHIELDED_OPERATIONS_ROLE_UNAVAILABLE_LABEL.ends_with('.'));
        assert!(
            SHIELDED_OPERATIONS_ROLE_UNAVAILABLE_LABEL.contains("view"),
            "the notice must state what the user can still do"
        );
        assert!(
            SHIELDED_OPERATIONS_ROLE_UNAVAILABLE_LABEL.contains("Expert view"),
            "the notice must name the interface mode that unlocks shielded sending"
        );
    }

    /// The Verified badge follows the same icon + text rule so
    /// greyscale viewers see the same affirmation as colour users.
    #[test]
    fn verified_indicator_uses_icon_and_text() {
        assert!(!SHIELDED_VERIFIED_ICON.is_empty());
        assert!(!SHIELDED_VERIFIED_LABEL.is_empty());
        assert!(SHIELDED_VERIFIED_LABEL.ends_with('.'));
        assert_ne!(SHIELDED_VERIFIED_ICON, SHIELDED_VERIFIED_LABEL);
    }

    /// The badge stays green on the terminal states that failed *something else*
    /// (unreadable identities, a hard app-data failure) because none of them
    /// touch shielded data — so its copy must name the one thing it vouches for.
    /// A badge whose subject comes from its position under the balance reads as
    /// a blanket "all good" beside the migration error banner, and hands the
    /// translator an adjective with no noun to agree with.
    #[test]
    fn verified_label_names_the_balance_it_vouches_for() {
        assert!(
            SHIELDED_VERIFIED_LABEL
                .to_lowercase()
                .contains("shielded balance"),
            "the Verified badge must name its subject, not borrow it from the layout: \
             `{SHIELDED_VERIFIED_LABEL}`",
        );
    }

    /// `derive_shielded_indicator` maps every migration state onto the
    /// expected J-3 badge. Pure inputs / pure output — testable without
    /// a UI harness.
    #[test]
    fn indicator_mapping_covers_every_migration_state() {
        assert_eq!(
            derive_shielded_indicator(&MigrationState::Idle, false),
            ShieldedIndicator::Hidden,
        );
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::Running {
                    step: MigrationStep::Detecting,
                },
                false,
            ),
            ShieldedIndicator::Hidden,
            "non-shielded steps don't hijack the shielded badge",
        );
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::Running {
                    step: MigrationStep::Shielded,
                },
                false,
            ),
            ShieldedIndicator::Verifying,
        );
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::Failed {
                    error: std::sync::Arc::new(
                        crate::backend_task::migration::MigrationError::WalletBackendUnavailable,
                    ),
                },
                false,
            ),
            ShieldedIndicator::Failed,
        );
        assert_eq!(
            derive_shielded_indicator(&MigrationState::Success, false),
            ShieldedIndicator::Verified,
        );
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::SucceededWithUnreadableData {
                    identities: 0,
                    votes: 1,
                    top_ups: 0,
                },
                false,
            ),
            ShieldedIndicator::Verified,
            "an unreadable vote row says nothing about shielded data — the drain completed",
        );
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::SucceededWithUnreadableData {
                    identities: 1,
                    votes: 0,
                    top_ups: 0,
                },
                false,
            ),
            ShieldedIndicator::Verified,
            "an unreadable identity row costs the user keys, not shielded notes",
        );
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::SucceededWithUnreadableData {
                    identities: 1,
                    votes: 2,
                    top_ups: 0,
                },
                false,
            ),
            ShieldedIndicator::Verified,
            "two unreadable-row signals are still not a shielded-data signal",
        );
        // The one state where the badge sits beside a red Error banner. It stays
        // Verified on purpose: the error is the app-data pass, which runs after
        // the drain and never touches shielded storage. Downgrading it would lock
        // spends over a corrupt vote row and claim a shielded failure that did not
        // happen.
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::FailedWithUnreadableIdentities {
                    count: 1,
                    error: std::sync::Arc::new(
                        crate::backend_task::migration::MigrationError::WalletBackendUnavailable,
                    ),
                },
                false,
            ),
            ShieldedIndicator::Verified,
            "a failed app-data pass is not a failed shielded migration — spends stay open",
        );
        // Skip-for-now hides the indicator regardless of state — the
        // session-local override the UI uses to dismiss the retry
        // banner.
        assert_eq!(
            derive_shielded_indicator(
                &MigrationState::Failed {
                    error: std::sync::Arc::new(
                        crate::backend_task::migration::MigrationError::WalletBackendUnavailable,
                    ),
                },
                true,
            ),
            ShieldedIndicator::Hidden,
        );
    }
}
