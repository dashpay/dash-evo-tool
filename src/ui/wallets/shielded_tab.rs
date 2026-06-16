use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::migration::MigrationTask;
use crate::context::AppContext;
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::model::wallet::WalletSeedHash;
use crate::ui::ScreenType;
use crate::ui::components::wallet_unlock_popup::wallet_needs_unlock;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use std::sync::Arc;

/// J-3 indicator strings — single complete sentences so the i18n
/// extraction pass picks each one up as a discrete translation unit.
/// Exposed `pub` so kittest coverage (TC-A11Y-006) can assert against
/// the exact label the UI renders.
pub const SHIELDED_VERIFYING_LABEL: &str = "Verifying shielded balance.";
pub const SHIELDED_VERIFIED_LABEL: &str = "Verified.";
pub const SHIELDED_SPEND_LOCKED_LABEL: &str = "Spending paused.";
pub const SHIELDED_SPEND_LOCKED_TOOLTIP: &str =
    "Spending paused until shielded balance is verified.";
pub const SHIELDED_LOCK_ICON: &str = "\u{1F512}"; // 🔒
pub const SHIELDED_VERIFIED_ICON: &str = "\u{2714}"; // ✔
pub const SHIELDED_RETRY_MIGRATION_LABEL: &str = "Retry shielded migration";
pub const SHIELDED_SKIP_MIGRATION_LABEL: &str = "Skip for now";
pub const SHIELDED_MIGRATION_ERROR_LABEL: &str =
    "Shielded data could not be migrated. Try again, or skip and use the rest of your wallet.";
pub const SHIELDED_TAB_SKIPPED_LABEL: &str =
    "Shielded features are paused until the next launch. Restart the app to retry the migration.";

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
        MigrationState::Success => ShieldedIndicator::Verified,
        // Idle / non-shielded running step → no badge.
        MigrationState::Idle | MigrationState::Running { .. } => ShieldedIndicator::Hidden,
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
    /// Pending backend task to dispatch on next ui() call (e.g., sync after Resync).
    pending_task: Option<BackendTask>,
    /// Number of diversified addresses generated (always >= 1).
    address_count: u32,
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
            address_count: 1,
            sidecar_skipped: false,
        }
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
            self.address_count = 1;
            // Skip-for-now is session-scoped to the wallet; a new
            // wallet starts with the retry banner re-enabled.
            self.sidecar_skipped = false;
        }
    }

    pub fn update_app_context(&mut self, app_context: &Arc<AppContext>) {
        self.app_context = app_context.clone();
    }

    /// Drain pending backend tasks (from explicit user actions like Resync).
    /// Initialization is handled entirely by the backend in
    /// `handle_wallet_unlocked` — the UI never triggers it.
    pub fn tick(&mut self) -> AppAction {
        self.refresh_from_backend_state();

        self.pending_task
            .take()
            .map(AppAction::BackendTask)
            .unwrap_or(AppAction::None)
    }

    /// Sync local display state from the push balance snapshot and the
    /// upstream coordinator.
    ///
    /// Phase D: the upstream `platform-wallet` coordinator owns all Orchard
    /// state (keys, sync progress, note tree). Balance is read from the
    /// frame-safe push snapshot; `is_initialized` / `tree_synced` are set
    /// true whenever the wallet backend is wired so spend buttons are
    /// enabled. Phase E will wire a push notification for fine-grained
    /// sync progress.
    fn refresh_from_backend_state(&mut self) {
        // Balance: use the frame-safe push snapshot (no lock in frame loop).
        self.shielded_balance = self.app_context.shielded_balance_credits(&self.seed_hash);

        // Treat the wallet as initialized and the tree as synced whenever the
        // backend is available — the coordinator resyncs Orchard state from
        // chain on its own schedule.
        if self.app_context.wallet_backend().is_ok() {
            self.is_initialized = true;
            self.tree_synced = true;
            self.syncing = false;
        }
    }

    /// Render the collapsible shielded addresses section with a table of all
    /// diversified addresses.
    fn render_address_section(&mut self, ui: &mut Ui, dark_mode: bool) {
        let dev_mode = self.app_context.is_developer_mode();

        let header = egui::CollapsingHeader::new(
            RichText::new("Shielded Addresses")
                .size(16.0)
                .color(DashColors::text_primary(dark_mode)),
        )
        .id_salt("shielded_addresses")
        .default_open(dev_mode);

        header.show(ui, |ui| {
            // Phase D: shielded addresses are now derived by the upstream
            // platform-wallet coordinator. The default address is available
            // via the async WalletBackend::shielded_default_address API;
            // a synchronous display path will be wired in Phase E via the
            // push snapshot.
            ui.label(
                RichText::new("Shielded address available after wallet unlock and sync.")
                    .color(DashColors::text_secondary(dark_mode)),
            );
        });
    }

    /// Handle backend task results for shielded operations.
    ///
    /// Phase D: variants for the retired DET-owned subsystem
    /// (`ShieldedInitialized`, `ShieldedNotesSynced`, `ShieldedNullifiersChecked`)
    /// have been removed. Only fund-moving results remain.
    pub fn handle_result(
        &mut self,
        result: &crate::backend_task::BackendTaskSuccessResult,
    ) -> bool {
        use crate::backend_task::BackendTaskSuccessResult;
        match result {
            BackendTaskSuccessResult::ShieldedCreditsShielded { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message =
                    Some(format!("Shielded {} successfully", format_credits(*amount)));
                true
            }
            BackendTaskSuccessResult::ShieldedTransferComplete { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message =
                    Some(format!("Transferred {} privately", format_credits(*amount)));
                true
            }
            BackendTaskSuccessResult::ShieldedCreditsUnshielded { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!("Unshielded {}", format_credits(*amount)));
                true
            }
            BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Shielded {} from core wallet",
                    format_credits(*amount)
                ));
                true
            }
            BackendTaskSuccessResult::ShieldedWithdrawalComplete { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Withdrew {} to core address",
                    format_credits(*amount)
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

    /// Render the shielded tab content.
    pub fn ui(&mut self, ui: &mut Ui) -> AppAction {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                        RichText::new(format_credits(self.shielded_balance))
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

        // Shielded Addresses (collapsible table)
        self.render_address_section(ui, dark_mode);

        ui.add_space(10.0);

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
                action |= AppAction::AddScreen(
                    ScreenType::ShieldScreen(self.seed_hash).create_screen(&self.app_context),
                );
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
                action |= AppAction::AddScreen(
                    ScreenType::ShieldedSendScreen(self.seed_hash).create_screen(&self.app_context),
                );
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
                action |= AppAction::AddScreen(
                    ScreenType::UnshieldCreditsScreen(self.seed_hash)
                        .create_screen(&self.app_context),
                );
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

        ui.add_space(15.0);

        // Shielded Notes (Phase D: notes are now owned by the upstream coordinator)
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

fn format_credits(credits: u64) -> String {
    let dash = credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
    if dash >= 0.01 {
        format!("{:.4} DASH", dash)
    } else {
        format!("{} credits", credits)
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

    /// The Verified badge follows the same icon + text rule so
    /// greyscale viewers see the same affirmation as colour users.
    #[test]
    fn verified_indicator_uses_icon_and_text() {
        assert!(!SHIELDED_VERIFIED_ICON.is_empty());
        assert!(!SHIELDED_VERIFIED_LABEL.is_empty());
        assert!(SHIELDED_VERIFIED_LABEL.ends_with('.'));
        assert_ne!(SHIELDED_VERIFIED_ICON, SHIELDED_VERIFIED_LABEL);
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
