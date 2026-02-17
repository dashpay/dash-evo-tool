use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::shielded::ShieldedTask;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use crate::ui::ScreenType;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use std::sync::Arc;

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
    /// Pending backend task to dispatch on next ui() call (e.g., auto-sync after init).
    pending_task: Option<BackendTask>,
    /// Wallet unlock popup for the initialize flow.
    wallet_unlock_popup: WalletUnlockPopup,
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
            wallet_unlock_popup: WalletUnlockPopup::new(),
        }
    }

    pub fn update_seed_hash(&mut self, seed_hash: WalletSeedHash) {
        if self.seed_hash != seed_hash {
            self.seed_hash = seed_hash;
            self.is_initialized = false;
            self.tree_synced = false;
            self.shielded_balance = 0;
            self.error_message = None;
            self.success_message = None;
        }
    }

    pub fn update_app_context(&mut self, app_context: &Arc<AppContext>) {
        self.app_context = app_context.clone();
    }

    /// Handle backend task results for shielded operations.
    pub fn handle_result(
        &mut self,
        result: &crate::backend_task::BackendTaskSuccessResult,
    ) -> bool {
        use crate::backend_task::BackendTaskSuccessResult;
        match result {
            BackendTaskSuccessResult::ShieldedInitialized { seed_hash, balance }
                if *seed_hash == self.seed_hash =>
            {
                self.initializing = false;
                self.is_initialized = true;
                self.shielded_balance = *balance;
                // Auto-sync notes after initialization
                self.syncing = true;
                self.pending_task = Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                    seed_hash: self.seed_hash,
                }));
                true
            }
            BackendTaskSuccessResult::ShieldedNotesSynced {
                seed_hash,
                new_notes,
                balance,
            } if *seed_hash == self.seed_hash => {
                self.syncing = false;
                self.tree_synced = true;
                self.shielded_balance = *balance;
                if *new_notes > 0 {
                    self.success_message = Some(format!("Synced {} new note(s)", new_notes));
                } else {
                    self.success_message = Some("Notes are up to date".to_string());
                }
                true
            }
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
            BackendTaskSuccessResult::ShieldedNullifiersChecked {
                seed_hash,
                spent_count,
            } if *seed_hash == self.seed_hash => {
                if *spent_count > 0 {
                    self.success_message = Some(format!("Detected {} spent note(s)", spent_count));
                }
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

    /// Render the shielded tab content.
    pub fn ui(&mut self, ui: &mut Ui) -> AppAction {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut action = self
            .pending_task
            .take()
            .map(AppAction::BackendTask)
            .unwrap_or(AppAction::None);

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

        // --- Not yet initialized: show Initialize button + unlock popup ---
        if !self.is_initialized {
            if self.initializing {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                    ui.label("Initializing shielded wallet (deriving ZIP32 keys)...");
                });
            } else {
                ui.add_space(20.0);
                ui.label(
                    RichText::new(
                        "Initialize your shielded wallet to enable private transactions.",
                    )
                    .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(10.0);

                let init_btn = egui::Button::new(
                    RichText::new("Initialize Shielded Wallet")
                        .color(Color32::WHITE)
                        .size(16.0),
                )
                .fill(DashColors::DASH_BLUE);

                if ui.add(init_btn).clicked() {
                    // Get the wallet Arc
                    let wallet_arc = {
                        let wallets = self.app_context.wallets.read().unwrap();
                        wallets.get(&self.seed_hash).cloned()
                    };

                    if let Some(wallet) = &wallet_arc {
                        if wallet_needs_unlock(wallet) {
                            // Wallet is locked — open unlock popup
                            self.wallet_unlock_popup.open();
                        } else {
                            // Try open without password (for passwordless wallets)
                            let _ = try_open_wallet_no_password(wallet);
                            // Proceed to initialize
                            self.initializing = true;
                            action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                                ShieldedTask::InitializeShieldedWallet {
                                    seed_hash: self.seed_hash,
                                },
                            ));
                        }
                    }
                }
            }

            // Show unlock popup if open
            if self.wallet_unlock_popup.is_open() {
                let wallet_arc = {
                    let wallets = self.app_context.wallets.read().unwrap();
                    wallets.get(&self.seed_hash).cloned()
                };

                if let Some(wallet) = &wallet_arc {
                    let unlock_result =
                        self.wallet_unlock_popup
                            .show(ui.ctx(), wallet, &self.app_context);
                    match unlock_result {
                        WalletUnlockResult::Unlocked => {
                            // Wallet is now open — proceed to initialize
                            self.initializing = true;
                            action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                                ShieldedTask::InitializeShieldedWallet {
                                    seed_hash: self.seed_hash,
                                },
                            ));
                        }
                        WalletUnlockResult::Cancelled => {
                            // User cancelled — do nothing
                        }
                        WalletUnlockResult::Pending => {
                            // Still showing popup
                        }
                    }
                }
            }

            return action;
        }

        // --- Initialized: show balance, address, actions ---

        // Balance display
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
            });

        ui.add_space(10.0);

        // Payment address
        let address_str = {
            let states = self.app_context.shielded_states.lock().unwrap();
            states.get(&self.seed_hash).map(|state| {
                let raw = state.keys.default_address.to_raw_address_bytes();
                hex::encode(raw)
            })
        };

        if let Some(addr) = &address_str {
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(16, 12))
                .corner_radius(8.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Shielded Payment Address")
                            .size(14.0)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let truncated = if addr.len() > 20 {
                            format!("{}...{}", &addr[..10], &addr[addr.len() - 10..])
                        } else {
                            addr.clone()
                        };
                        ui.monospace(&truncated);
                        if ui.small_button("Copy").clicked() {
                            let _ = copy_text_to_clipboard(addr);
                        }
                    });
                });
        }

        ui.add_space(10.0);

        // Action buttons
        ui.horizontal(|ui| {
            let shield_btn =
                egui::Button::new(RichText::new("Shield").color(Color32::WHITE).size(14.0))
                    .fill(DashColors::DASH_BLUE);
            if ui
                .add_enabled(!self.syncing, shield_btn)
                .on_hover_text("Shield credits from platform address into the shielded pool")
                .clicked()
            {
                action |= AppAction::AddScreen(
                    ScreenType::ShieldCreditsScreen(self.seed_hash)
                        .create_screen(&self.app_context),
                );
            }

            let can_spend = !self.syncing && self.tree_synced && self.shielded_balance > 0;

            let send_btn = egui::Button::new(
                RichText::new("Send (Private)")
                    .color(Color32::WHITE)
                    .size(14.0),
            )
            .fill(DashColors::DASH_BLUE);
            if ui
                .add_enabled(can_spend, send_btn)
                .on_hover_text(if self.tree_synced {
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
                .on_hover_text(if self.tree_synced {
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

            ui.add_space(10.0);

            if self.syncing {
                ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                ui.label("Syncing...");
            } else if ui.button("Sync Notes").clicked() {
                self.syncing = true;
                self.success_message = None;
                self.error_message = None;
                action |=
                    AppAction::BackendTask(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                        seed_hash: self.seed_hash,
                    }));
            }
        });

        ui.add_space(15.0);

        // Notes table
        let notes_info: Vec<(u64, u64, bool)> = {
            let states = self.app_context.shielded_states.lock().unwrap();
            states
                .get(&self.seed_hash)
                .map(|state| {
                    state
                        .notes
                        .iter()
                        .map(|n| (n.value, n.block_height, n.is_spent))
                        .collect()
                })
                .unwrap_or_default()
        };

        if !notes_info.is_empty() {
            ui.label(
                RichText::new("Shielded Notes")
                    .size(16.0)
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(5.0);

            egui::Grid::new("shielded_notes_grid")
                .num_columns(3)
                .striped(true)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Value").strong());
                    ui.label(RichText::new("Block").strong());
                    ui.label(RichText::new("Status").strong());
                    ui.end_row();

                    for (value, height, is_spent) in &notes_info {
                        ui.label(format_credits(*value));
                        ui.label(if *height > 0 {
                            height.to_string()
                        } else {
                            "-".to_string()
                        });
                        if *is_spent {
                            ui.label(
                                RichText::new("Spent").color(DashColors::text_secondary(dark_mode)),
                            );
                        } else {
                            ui.label(RichText::new("Unspent").color(Color32::DARK_GREEN));
                        }
                        ui.end_row();
                    }
                });
        } else {
            ui.label(
                RichText::new("No shielded notes yet. Shield some credits to get started.")
                    .color(DashColors::text_secondary(dark_mode)),
            );
        }

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
