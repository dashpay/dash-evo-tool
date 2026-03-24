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
    /// Currently selected diversified address index.
    selected_address_index: u32,
    /// Number of diversified addresses generated (always >= 1).
    address_count: u32,
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
            selected_address_index: 0,
            address_count: 1,
        }
    }

    pub fn is_syncing(&self) -> bool {
        self.syncing
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
                self.tree_synced = true;
                self.shielded_balance = *balance;
                if *new_notes > 0 {
                    self.success_message = Some(format!("Synced {} new note(s)", new_notes));
                }
                // Auto-check nullifiers after sync to detect spent notes
                self.pending_task =
                    Some(BackendTask::ShieldedTask(ShieldedTask::CheckNullifiers {
                        seed_hash: self.seed_hash,
                    }));
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
            BackendTaskSuccessResult::ShieldedFromAssetLock { seed_hash, amount }
                if *seed_hash == self.seed_hash =>
            {
                self.success_message = Some(format!(
                    "Shielded {} from core wallet",
                    format_credits(*amount)
                ));
                true
            }
            BackendTaskSuccessResult::ShieldedNullifiersChecked {
                seed_hash,
                spent_count,
            } if *seed_hash == self.seed_hash => {
                self.syncing = false;
                // Update balance from state after nullifier check
                let states = self.app_context.shielded_states.lock().unwrap();
                if let Some(state) = states.get(&self.seed_hash) {
                    self.shielded_balance = state.shielded_balance;
                }
                drop(states);
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

    // TODO: Redesign shielded tab layout for visual consistency with other tabs:
    //   1. Action buttons row at top: Shield, Shield from Core, Transfer, Unshield
    //   2. Shielded Addresses (collapsible) — diversified addresses in a table
    //   3. Shielded Notes (collapsible) — notes table (index, value, spent/unspent)
    // Currently the layout is: balance card -> address card -> buttons -> notes list.
    // The redesign should move buttons to the top and use collapsible sections.

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

        // --- Not yet initialized ---
        // Auto-initialize if the wallet is already open (no user click needed)
        if !self.is_initialized && !self.initializing {
            let wallet_arc = {
                let wallets = self.app_context.wallets.read().unwrap();
                wallets.get(&self.seed_hash).cloned()
            };
            if let Some(wallet) = &wallet_arc
                && !wallet_needs_unlock(wallet)
            {
                let _ = try_open_wallet_no_password(wallet);
                self.initializing = true;
                action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                    ShieldedTask::InitializeShieldedWallet {
                        seed_hash: self.seed_hash,
                    },
                ));
            }
        }

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

        // Payment address (bech32m encoded: dash1z... or tdash1z...)
        let address_str = {
            let states = self.app_context.shielded_states.lock().unwrap();
            states.get(&self.seed_hash).and_then(|state| {
                use dash_sdk::dpp::address_funds::OrchardAddress;
                use dash_sdk::grovedb_commitment_tree::Scope;
                let addr = state
                    .keys
                    .fvk
                    .address_at(self.selected_address_index, Scope::External);
                let raw = addr.to_raw_address_bytes();
                let orchard_addr = OrchardAddress::from_raw_bytes(&raw).ok()?;
                Some(orchard_addr.to_bech32m_string(self.app_context.network))
            })
        };

        if let Some(addr) = &address_str {
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(16, 12))
                .corner_radius(8.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Shielded Payment Address ({})",
                                self.selected_address_index
                            ))
                            .size(14.0)
                            .color(DashColors::text_secondary(dark_mode)),
                        );

                        // Address selector: prev/next arrows
                        if self.selected_address_index > 0 && ui.small_button("<").clicked() {
                            self.selected_address_index -= 1;
                        }
                        if self.selected_address_index + 1 < self.address_count
                            && ui.small_button(">").clicked()
                        {
                            self.selected_address_index += 1;
                        }

                        // Generate new diversified address
                        if ui
                            .small_button("+")
                            .on_hover_text("Generate new diversified address")
                            .clicked()
                        {
                            self.selected_address_index = self.address_count;
                            self.address_count += 1;
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.monospace(addr);
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

            let shield_core_btn = egui::Button::new(
                RichText::new("Shield from Core")
                    .color(Color32::WHITE)
                    .size(14.0),
            )
            .fill(DashColors::DASH_BLUE);
            if ui
                .add_enabled(!self.syncing, shield_core_btn)
                .on_hover_text("Shield core DASH directly into the shielded pool via asset lock")
                .clicked()
            {
                action |= AppAction::AddScreen(
                    ScreenType::ShieldFromAssetLockScreen(self.seed_hash)
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
        });

        ui.add_space(15.0);

        // Notes section header with sync status and buttons
        let (notes_info, synced_index): (Vec<(u64, u64, bool)>, u64) = {
            let states = self.app_context.shielded_states.lock().unwrap();
            states
                .get(&self.seed_hash)
                .map(|state| {
                    let notes = state
                        .notes
                        .iter()
                        .map(|n| (n.value, n.block_height, n.is_spent))
                        .collect();
                    (notes, state.last_synced_index)
                })
                .unwrap_or_default()
        };

        // Shielded Notes (collapsible)
        let notes_label = if notes_info.is_empty() {
            "Shielded Notes".to_string()
        } else {
            format!(
                "Shielded Notes (synced to index {}, {} notes)",
                synced_index,
                notes_info.len()
            )
        };
        let notes_header = egui::CollapsingHeader::new(
            RichText::new(notes_label)
                .size(16.0)
                .color(DashColors::text_primary(dark_mode)),
        )
        .id_salt("shielded_notes")
        .default_open(true);
        notes_header.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Sync status indicator
                if self.syncing {
                    ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                    ui.label(
                        RichText::new("Syncing...")
                            .size(12.0)
                            .color(DashColors::DASH_BLUE),
                    );
                } else if self.tree_synced {
                    ui.label(
                        RichText::new("Synced")
                            .size(12.0)
                            .color(Color32::DARK_GREEN),
                    );
                }

                // Sync buttons
                if !self.syncing {
                    if ui.small_button("Sync Notes").clicked() {
                        self.syncing = true;
                        self.success_message = None;
                        self.error_message = None;
                        action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                            ShieldedTask::SyncNotes {
                                seed_hash: self.seed_hash,
                            },
                        ));
                    }

                    if self.app_context.is_developer_mode()
                        && ui.small_button("Resync Notes").clicked()
                    {
                        {
                            let mut states = self.app_context.shielded_states.lock().unwrap();
                            states.remove(&self.seed_hash);
                        }
                        let network_str = self.app_context.network.to_string();
                        let _ = self
                            .app_context
                            .db
                            .delete_shielded_notes(&self.seed_hash, &network_str);
                        let _ = self.app_context.db.clear_commitment_tree_tables();

                        self.shielded_balance = 0;
                        self.tree_synced = false;
                        self.is_initialized = false;
                        self.initializing = true;
                        self.syncing = false;
                        self.success_message = None;
                        self.error_message = None;
                        action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                            ShieldedTask::InitializeShieldedWallet {
                                seed_hash: self.seed_hash,
                            },
                        ));
                    }
                }
            });
            ui.add_space(5.0);

            if !notes_info.is_empty() {
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
                                    RichText::new("Spent")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            } else {
                                ui.label(RichText::new("Unspent").color(Color32::DARK_GREEN));
                            }
                            ui.end_row();
                        }
                    });
            } else if !self.syncing {
                ui.label(
                    RichText::new("No shielded notes yet. Shield some credits to get started.")
                        .color(DashColors::text_secondary(dark_mode)),
                );
            }

            // Sync status indicator
            if self.syncing {
                ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
                ui.label(
                    RichText::new("Syncing...")
                        .size(12.0)
                        .color(DashColors::DASH_BLUE),
                );
            } else if self.tree_synced {
                ui.label(
                    RichText::new("Synced")
                        .size(12.0)
                        .color(Color32::DARK_GREEN),
                );
            }

            // Sync buttons
            if !self.syncing {
                if ui.small_button("Sync Notes").clicked() {
                    self.syncing = true;
                    self.success_message = None;
                    self.error_message = None;
                    action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                        ShieldedTask::SyncNotes {
                            seed_hash: self.seed_hash,
                        },
                    ));
                }

                if self.app_context.is_developer_mode() && ui.small_button("Resync Notes").clicked()
                {
                    // Remove in-memory state entirely (will be recreated by init)
                    {
                        let mut states = self.app_context.shielded_states.lock().unwrap();
                        states.remove(&self.seed_hash);
                    }
                    // Clear persisted notes and commitment tree data
                    let network_str = self.app_context.network.to_string();
                    let _ = self
                        .app_context
                        .db
                        .delete_shielded_notes(&self.seed_hash, &network_str);
                    let _ = self.app_context.db.clear_commitment_tree_tables();

                    self.shielded_balance = 0;
                    self.tree_synced = false;
                    self.is_initialized = false;
                    self.initializing = true;
                    self.syncing = false;
                    self.success_message = None;
                    self.error_message = None;
                    // Re-initialize (creates fresh persistent tree) then auto-syncs
                    action |= AppAction::BackendTask(BackendTask::ShieldedTask(
                        ShieldedTask::InitializeShieldedWallet {
                            seed_hash: self.seed_hash,
                        },
                    ));
                }
            }
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
