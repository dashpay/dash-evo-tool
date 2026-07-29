mod by_platform_address;
mod by_receive_deposit;
mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod success_screen;

use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::core::CoreItem;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{
    IdentityKeyEntry, IdentityKeySpecs, IdentityRegistrationInfo, IdentityTask,
    RegisterIdentityFundingMethod, default_identity_key_specs,
};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::asset_lock::{AssetLockAmountError, validate_asset_lock_amount};
use crate::model::fee_estimation::{format_credits_as_dash, format_duffs_as_dash};
use crate::model::secret::Secret;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::MessageBanner;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::funding_common::{
    FundingMethod, WalletFundedScreenStep, default_funding_state, deposit_event_outcome,
    funding_method_after_switch, max_amount_after_fee_reserve, receive_deposit_ceiling_duffs,
    spendable_covers_minimum, step_after_task_failure, wallet_selection_combo,
};
use crate::ui::state::{AssetLockBalanceCache, TrackedAssetLockCache};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use crate::wallet_backend::poison::RwLockRecover;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::platform::Identifier;
use egui::{Align, Button, Color32, ComboBox, ScrollArea, Ui};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::collections::HashSet;

use crate::model::amount::Amount;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

pub const MAX_IDENTITY_INDEX: u32 = 30;

/// Compose a wallet-picker entry as `alias — spendable-balance in DASH`.
///
/// The balance shown is always the wallet's **spendable** amount (never the
/// total): only spendable funds can pay for identity creation, so surfacing
/// the total here would invite the very insufficient-funds surprise this
/// label exists to prevent. A pure function so the wording is testable
/// without constructing a real wallet/balance snapshot.
fn format_wallet_picker_label(alias: &str, spendable_duffs: u64) -> String {
    format!("{alias} — {}", Amount::dash_from_duffs(spendable_duffs))
}

pub struct AddNewIdentityScreen {
    identity_id_number: u32,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    /// Outpoint of an asset lock tracked by the upstream `AssetLockManager`,
    /// chosen by the user from the picker. Routed to the backend as
    /// `RegisterIdentityFundingMethod::UseAssetLock`; the upstream signer
    /// re-derives the credit-output key from the seed.
    funding_asset_lock: Option<OutPoint>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    funding_address: Option<Address>,
    /// A queued deposit-address derivation for the "Receive a new deposit"
    /// method. Set when the QR view needs an address; drained at the end of
    /// `ui()` into a [`WalletTask::GenerateReceiveAddress`] task.
    pending_funding_address_request: Option<WalletSeedHash>,
    /// True after the queued receive-address request is dispatched and until
    /// its correlated success or failure result returns.
    funding_address_request_in_flight: bool,
    /// Set when deposit-address generation or parsing fails, so the QR view
    /// offers a manual retry instead of spinning forever.
    funding_address_request_failed: bool,
    /// Spendable duffs currently held at the address shown by the deposit flow.
    funding_address_balance_duffs: u64,
    /// Set on the transition to `FundsReceived` so the amount field pre-fills
    /// the fee-reserve-capped received balance on the next render.
    prefill_funding_amount: bool,
    funding_method: Arc<RwLock<FundingMethod>>,
    /// Whether the user has explicitly picked a funding method (as opposed to
    /// the screen's own default pre-selection). Once true, a wallet switch
    /// preserves the chosen method instead of recomputing the default.
    user_chose_funding_method: bool,
    funding_amount: Option<Amount>,
    funding_amount_input: Option<AmountInput>,
    alias_input: String,
    copied_to_clipboard: Option<Option<String>>,
    /// The chosen key set, public-only. Populated from the identity-auth
    /// public-key cache (D4b); `master` is `None` until the cache is warm,
    /// which gates registration (fail-closed, RK-2).
    identity_keys: IdentityKeySpecs,
    /// `true` while a [`WalletTask::WarmIdentityAuthPubkeys`] task is in flight
    /// for the current identity index, so the warm is not re-dispatched every
    /// frame and the UI can show a "preparing keys" hint.
    warming_identity_keys: bool,
    /// A queued cache-warm request: (seed hash, identity index). Set when the
    /// chooser reads a cold cache; drained at the end of `ui()` into a
    /// [`WalletTask::WarmIdentityAuthPubkeys`] task.
    pending_warm_request: Option<([u8; 32], u32)>,
    /// Per-key-id revealed WIFs (advanced mode "Show WIF"), derived on demand
    /// via [`WalletTask::DeriveKeyForDisplay`]. Id 0 is the master key. Each is
    /// zeroize-on-drop and cleared when the key set is rebuilt.
    revealed_wifs: HashMap<u32, Secret>,
    /// A queued "derive WIF for display" request: (key id, derivation path).
    /// Drained at the end of `ui()` into a `DeriveKeyForDisplay` task so the
    /// seed is fetched just-in-time and only the WIF returns.
    pending_wif_request: Option<(u32, DerivationPath)>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    show_pop_up_info: Option<String>,
    in_key_selection_advanced_mode: bool,
    pub app_context: Arc<AppContext>,
    successful_qualified_identity_id: Option<Identifier>,
    /// Selected Platform address for funding  with the amount in credits
    selected_platform_address_for_funding: Option<(
        dash_sdk::dpp::address_funds::PlatformAddress,
        dash_sdk::dpp::fee::Credits,
    )>,
    /// Amount input for Platform address funding
    platform_funding_amount: Option<Amount>,
    platform_funding_amount_input: Option<AmountInput>,
    /// Whether to show advanced options
    show_advanced_options: bool,
    /// Fee result from completed identity registration
    completed_fee_result: Option<FeeResult>,
    /// Tracked asset locks for the selected wallet, fetched off the UI thread
    /// via the App Task System. Backs both the funding-method gate and the
    /// asset-lock picker.
    asset_lock_cache: TrackedAssetLockCache,
    asset_lock_balance: AssetLockBalanceCache,
}

impl AddNewIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self::new_with_wallet(app_context, None)
    }

    pub fn new_with_wallet(
        app_context: &Arc<AppContext>,
        wallet_seed_hash: Option<[u8; 32]>,
    ) -> Self {
        let mut selected_wallet = None;

        if app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = &app_context.wallets.read_recover();
            // If a specific wallet seed hash is provided, use that wallet
            if let Some(seed_hash) = wallet_seed_hash
                && let Some(wallet) = wallets.get(&seed_hash)
            {
                selected_wallet = Some(wallet.clone());
            }
            // Otherwise, select the first available wallet
            if selected_wallet.is_none()
                && let Some(wallet) = wallets.values().next()
            {
                selected_wallet = Some(wallet.clone());
            }
        }

        // The funding-method pre-selection is applied by `update_wallet` below
        // (and re-applied on every later wallet switch while the user has not
        // made an explicit choice), so the chooser always tracks a wallet the
        // pre-selection can actually be funded from.
        let mut created = Self {
            identity_id_number: 0, // updated later
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            selected_wallet: None, // updated later
            funding_address: None,
            pending_funding_address_request: None,
            funding_address_request_in_flight: false,
            funding_address_request_failed: false,
            funding_address_balance_duffs: 0,
            prefill_funding_amount: false,
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            user_chose_funding_method: false,
            funding_amount: None,
            funding_amount_input: None,
            alias_input: String::new(),
            copied_to_clipboard: None,
            // updated later
            identity_keys: IdentityKeySpecs::empty(),
            warming_identity_keys: false,
            pending_warm_request: None,
            revealed_wifs: HashMap::new(),
            pending_wif_request: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            show_pop_up_info: None,
            in_key_selection_advanced_mode: false,
            app_context: app_context.clone(),
            successful_qualified_identity_id: None,
            selected_platform_address_for_funding: None,
            platform_funding_amount: None,
            platform_funding_amount_input: None,
            show_advanced_options: false,
            completed_fee_result: None,
            asset_lock_cache: TrackedAssetLockCache::default(),
            asset_lock_balance: AssetLockBalanceCache::default(),
        };

        if let Some(wallet) = selected_wallet {
            created.update_wallet(wallet);
        };

        created
    }

    /// Default number of keys (master + additional) the chooser warms and reads
    /// from the auth-pubkey cache.
    fn default_key_count(&self) -> u32 {
        let dashpay_contract_id = self.app_context.dashpay_contract.id();
        // master (index 0) + the default additional keys.
        default_identity_key_specs(dashpay_contract_id).len() as u32 + 1
    }

    /// Read the chosen identity keys from the auth-pubkey cache (D4b),
    /// seed-free, for the current wallet + identity index.
    ///
    /// The chooser shows and submits **public** keys; the private keys are
    /// derived just-in-time at registration through the JIT chokepoint. On a
    /// cache hit this builds the [`IdentityKeySpecs`] entirely without the seed.
    /// On a miss it leaves the key set empty (registration stays disabled,
    /// fail-closed RK-2) and queues a cache warm (drained at the end of `ui()`
    /// into a [`WalletTask::WarmIdentityAuthPubkeys`] task); the next frame
    /// reads the now-warm cache.
    pub fn ensure_correct_identity_keys(&mut self) {
        self.revealed_wifs.clear();

        let Some(wallet_lock) = self.selected_wallet.clone() else {
            self.identity_keys = IdentityKeySpecs::empty();
            return;
        };

        let (seed_hash, is_open) = {
            let wallet = wallet_lock.read_recover();
            (wallet.seed_hash(), wallet.is_open())
        };
        if !is_open {
            self.identity_keys = IdentityKeySpecs::empty();
            return;
        }

        let network = self.app_context.network;
        let identity_index = self.identity_id_number;
        let dashpay_contract_id = self.app_context.dashpay_contract.id();
        let default_keys = default_identity_key_specs(dashpay_contract_id);

        let Ok(backend) = self.app_context.wallet_backend() else {
            self.identity_keys = IdentityKeySpecs::empty();
            return;
        };
        let cache = backend.auth_pubkey_cache().get(network, &seed_hash);

        // Master key at index 0.
        let Some(master_pk) = cache.get(network, identity_index, 0) else {
            self.queue_warm_identity_keys(seed_hash, identity_index);
            return;
        };
        let master = IdentityKeyEntry::from_cached_public_key(
            master_pk,
            network,
            identity_index,
            0,
            KeyType::ECDSA_HASH160,
            Purpose::AUTHENTICATION,
            SecurityLevel::MASTER,
            None,
        );

        let mut others = Vec::with_capacity(default_keys.len());
        for (i, (key_type, purpose, security_level, contract_bounds)) in
            default_keys.into_iter().enumerate()
        {
            let key_index = (i + 1) as u32;
            let Some(pk) = cache.get(network, identity_index, key_index) else {
                self.queue_warm_identity_keys(seed_hash, identity_index);
                return;
            };
            others.push(IdentityKeyEntry::from_cached_public_key(
                pk,
                network,
                identity_index,
                key_index,
                key_type,
                purpose,
                security_level,
                contract_bounds,
            ));
        }

        self.identity_keys = IdentityKeySpecs::new(Some(master), others);
        self.warming_identity_keys = false;
    }

    /// Queue a cache warm for the current identity index and mark the key set
    /// not-ready so registration stays disabled (fail-closed). The request is
    /// dispatched once at the end of `ui()`; `warming_identity_keys` prevents
    /// re-dispatch on subsequent frames while it is in flight.
    fn queue_warm_identity_keys(&mut self, seed_hash: [u8; 32], identity_index: u32) {
        self.identity_keys = IdentityKeySpecs::empty();
        if self.warming_identity_keys {
            return;
        }
        self.warming_identity_keys = true;
        self.pending_warm_request = Some((seed_hash, identity_index));
    }

    fn render_identity_index_input(&mut self, ui: &mut egui::Ui) {
        let mut index_changed = false; // Track if the index has changed

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(15.0);
                ui.label("Identity Index:");
            });

            // Check if we have access to the selected wallet
            if let Some(wallet_guard) = self.selected_wallet.as_ref() {
                let wallet = wallet_guard.read_recover();
                let used_indices: HashSet<u32> = wallet.identities.keys().cloned().collect();

                // Modify the selected text to include "(used)" if the current index is used
                let selected_text = {
                    let is_used = used_indices.contains(&self.identity_id_number);
                    if is_used {
                        format!("{} (used)", self.identity_id_number)
                    } else {
                        format!("{}", self.identity_id_number)
                    }
                };

                // Render a ComboBox to select the identity index
                ComboBox::from_id_salt("identity_index")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        // Provide up to 30 entries for selection
                        for i in 0..MAX_IDENTITY_INDEX {
                            let is_used = used_indices.contains(&i);
                            let label = if is_used {
                                format!("{} (used)", i)
                            } else {
                                format!("{}", i)
                            };

                            let is_selected = self.identity_id_number == i;

                            // Enable the option if it's not used or if it's the currently selected index
                            let enabled = !is_used || is_selected;

                            // Use `add_enabled` to disable used indices
                            let response =
                                ui.add_enabled(enabled, Button::selectable(is_selected, label));

                            // Only allow selection if the index is not used
                            if response.clicked() && !is_used {
                                self.identity_id_number = i;
                                index_changed = true;
                            }
                        }
                    });
            } else {
                ui.label("No wallet selected");
            }
        });

        // If the index has changed, refresh the identity keys from the cache.
        if index_changed {
            self.ensure_correct_identity_keys();
        }
    }

    /// Build the wallet-picker label (`alias — spendable balance`) for one
    /// wallet, reading its spendable balance from the display snapshot.
    ///
    /// Poison-tolerant: if the wallet lock is poisoned, falls back to a plain
    /// "Unnamed Wallet" label rather than panicking. Takes `&AppContext`
    /// (not `&self`) so the ComboBox closure can call it via a field-level
    /// borrow, leaving the closure's other `self` field writes undisturbed.
    fn wallet_picker_label(app_context: &AppContext, wallet: &Arc<RwLock<Wallet>>) -> String {
        let Some((seed_hash, alias)) = wallet.read().ok().map(|w| {
            let alias = w
                .alias
                .clone()
                .unwrap_or_else(|| "Unnamed Wallet".to_string());
            (w.seed_hash(), alias)
        }) else {
            return "Unnamed Wallet".to_string();
        };
        let spendable_duffs = app_context.snapshot_balance(&seed_hash).spendable();
        format_wallet_picker_label(&alias, spendable_duffs)
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        let mut clicked_wallet = None;
        let rendered = if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets: Vec<_> = self
                .app_context
                .wallets
                .read()
                .map(|guard| guard.values().cloned().collect())
                .unwrap_or_default();

            if wallets.len() > 1 {
                ui.heading("1. Choose which wallet this identity's keys will come from.");

                // Show each wallet's spendable balance next to its alias so
                // funding sufficiency is visible before choosing.
                let app_context = self.app_context.clone();
                clicked_wallet = wallet_selection_combo(
                    ui,
                    "select_wallet",
                    &wallets,
                    self.selected_wallet.as_ref(),
                    |wallet| Self::wallet_picker_label(&app_context, wallet),
                    |_| true,
                );
                true
            } else if let Some(wallet) = wallets.first() {
                if self.selected_wallet.is_none() {
                    // Automatically select the only available wallet.
                    clicked_wallet = Some(wallet.clone());
                }
                false
            } else {
                false
            }
        } else {
            false
        };

        if let Some(wallet) = clicked_wallet {
            // A wallet switch invalidates funding chosen for the previous
            // wallet; `update_wallet` re-derives the funding method/step.
            self.funding_address = None;
            self.pending_funding_address_request = None;
            self.funding_address_request_in_flight = false;
            self.funding_address_request_failed = false;
            self.funding_address_balance_duffs = 0;
            self.prefill_funding_amount = false;
            self.funding_asset_lock = None;
            self.copied_to_clipboard = None;
            self.update_wallet(wallet);
        }

        rendered
    }

    /// Whether the loaded builder ceiling covers the same minimum as the
    /// "not enough Dash" banner. An unloaded quote does not block the option.
    fn wallet_can_afford_creation(&self, wallet: &Arc<RwLock<Wallet>>) -> bool {
        let Ok(w) = wallet.read() else {
            return false;
        };
        let key_count = self.identity_keys.others.len() + 1;
        let minimum_credits = self
            .app_context
            .fee_estimator()
            .estimate_identity_create(key_count);
        self.asset_lock_balance
            .get(&w.seed_hash())
            .is_none_or(|ceiling| spendable_covers_minimum(ceiling, minimum_credits))
    }

    /// Update selected wallet and trigger all dependent actions, like updating
    /// identity keys and identity index.
    ///
    /// Called whenever the wallet changes in the UI or is unlocked. While the
    /// user has not explicitly chosen a funding method, the default
    /// pre-selection is recomputed for the new wallet; an explicit choice is
    /// preserved across the switch.
    fn update_wallet(&mut self, wallet: Arc<RwLock<Wallet>>) {
        let is_open = wallet.read().is_ok_and(|w| w.is_open());

        self.selected_wallet = Some(wallet);
        self.asset_lock_balance.invalidate();
        self.wallet_open_attempted = false;
        self.identity_id_number = self.next_identity_id();

        let can_afford = self
            .selected_wallet
            .as_ref()
            .is_some_and(|wallet| self.wallet_can_afford_creation(wallet));
        let current = (
            self.funding_method
                .read()
                .map(|m| *m)
                .unwrap_or(FundingMethod::NoSelection),
            self.step
                .read()
                .map(|s| *s)
                .unwrap_or(WalletFundedScreenStep::ChooseFundingMethod),
        );
        let (method, step) =
            funding_method_after_switch(self.user_chose_funding_method, current, can_afford);
        if let Ok(mut m) = self.funding_method.write() {
            *m = method;
        }
        if let Ok(mut s) = self.step.write() {
            *s = step;
        }

        if is_open {
            // A new wallet/index resets any in-flight warm so the cold cache
            // for the new selection is read fresh.
            self.warming_identity_keys = false;
            self.ensure_correct_identity_keys();
        }
    }

    /// Generate next identity ID that can be used for the new identity.
    ///
    /// TODO: This function is not working in a reliable way, because it relies on the
    /// `identities` map in the wallet, which may not be up to date (user can remove
    /// identities from the wallet while they still are stored on the Platform).
    fn next_identity_id(&self) -> u32 {
        self.selected_wallet
            .as_ref()
            .unwrap()
            .read_recover()
            .identities
            .keys()
            .copied()
            .max()
            .map(|max| max + 1)
            .unwrap_or_default()
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            return;
        };
        let funding_method_arc = self.funding_method.clone();
        let Ok(mut funding_method) = funding_method_arc.write() else {
            return;
        };

        ComboBox::from_id_salt("funding_method")
            .selected_text(format!("{}", *funding_method))
            .height(200.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::NoSelection,
                        format!("{}", FundingMethod::NoSelection),
                    )
                    .changed()
                {
                    // Deselecting returns to auto-default behavior so a later
                    // wallet switch may re-recommend a method.
                    self.user_chose_funding_method = false;
                    if let Ok(mut step) = self.step.write() {
                        *step = WalletFundedScreenStep::ChooseFundingMethod;
                    }
                    self.funding_amount = None;
                    self.funding_amount_input = None;
                }

                let (has_unused_asset_lock, has_balance) = {
                    let wallet = selected_wallet.read_recover();
                    let seed_hash = wallet.seed_hash();
                    // Offer the option on a failed fetch too, so the user can
                    // reach the picker's Retry rather than the option vanishing.
                    (
                        self.asset_lock_cache.has_unused(&seed_hash)
                            || self.asset_lock_cache.is_failed(&seed_hash),
                        self.asset_lock_balance
                            .get(&seed_hash)
                            .is_none_or(|ceiling| {
                                let key_count = self.identity_keys.others.len() + 1;
                                let minimum = self
                                    .app_context
                                    .fee_estimator()
                                    .estimate_identity_create(key_count);
                                spendable_covers_minimum(ceiling, minimum)
                            }),
                    )
                };

                if has_unused_asset_lock
                    && ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseUnusedAssetLock,
                            format!("{}", FundingMethod::UseUnusedAssetLock),
                        )
                        .changed()
                {
                    self.user_chose_funding_method = true;
                    self.ensure_correct_identity_keys();
                    if let Ok(mut step) = self.step.write() {
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                    self.funding_amount = None;
                    self.funding_amount_input = None;
                }
                if has_balance
                    && ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseWalletBalance,
                            format!("{}", FundingMethod::UseWalletBalance),
                        )
                        .changed()
                {
                    self.user_chose_funding_method = true;
                    self.funding_amount = None;
                    self.funding_amount_input = None;
                    if let Ok(mut step) = self.step.write() {
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                }
                // Check if wallet has Platform address balance
                let has_platform_balance = {
                    let wallet = selected_wallet.read_recover();
                    wallet
                        .platform_address_info
                        .values()
                        .any(|info| info.balance > 0)
                };
                if has_platform_balance
                    && ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UsePlatformAddress,
                            format!("{}", FundingMethod::UsePlatformAddress),
                        )
                        .changed()
                {
                    self.user_chose_funding_method = true;
                    self.ensure_correct_identity_keys();
                    if let Ok(mut step) = self.step.write() {
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                    self.platform_funding_amount = None;
                    self.platform_funding_amount_input = None;
                    self.selected_platform_address_for_funding = None;
                }
                // "Receive a new deposit" is always offered: it needs no existing
                // balance or asset lock, it creates the funds the wizard will use.
                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::ReceiveDeposit,
                        format!("{}", FundingMethod::ReceiveDeposit),
                    )
                    .changed()
                {
                    self.user_chose_funding_method = true;
                    self.ensure_correct_identity_keys();
                    // Await the deposit; the QR view derives the address lazily.
                    if let Ok(mut step) = self.step.write() {
                        *step = WalletFundedScreenStep::WaitingOnFunds;
                    }
                    self.funding_address = None;
                    self.pending_funding_address_request = None;
                    self.funding_address_request_in_flight = false;
                    self.funding_address_request_failed = false;
                    self.funding_address_balance_duffs = 0;
                    self.prefill_funding_amount = false;
                    self.funding_amount = None;
                    self.funding_amount_input = None;
                }
            });
    }

    /// Return the deposit chooser to its initial state so the user is never
    /// trapped in the waiting/received sub-steps. Clears the shown address and
    /// any pending derivation; the wallet keeps any deposit already received.
    fn reset_to_choose_funding(&mut self) {
        let (method, step) = default_funding_state(false);
        if let Ok(mut m) = self.funding_method.write() {
            *m = method;
        }
        if let Ok(mut s) = self.step.write() {
            *s = step;
        }
        self.user_chose_funding_method = false;
        self.funding_address = None;
        self.pending_funding_address_request = None;
        self.funding_address_request_in_flight = false;
        self.funding_address_request_failed = false;
        self.funding_address_balance_duffs = 0;
        self.prefill_funding_amount = false;
        self.funding_amount = None;
        self.funding_amount_input = None;
    }

    // Function to render the key selection mode (Default or Advanced)
    fn render_key_selection(&mut self, ui: &mut egui::Ui) {
        // Provide the selection toggle for Default or Advanced mode
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(15.0);
                ui.label("Key Selection Mode:");
            });

            ComboBox::from_id_salt("key_selection_mode")
                .selected_text(if self.in_key_selection_advanced_mode {
                    "Advanced"
                } else {
                    "Default"
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            !self.in_key_selection_advanced_mode,
                            "Default (Recommended)",
                        )
                        .clicked()
                    {
                        self.in_key_selection_advanced_mode = false;
                    }
                    if ui
                        .selectable_label(self.in_key_selection_advanced_mode, "Advanced")
                        .clicked()
                    {
                        self.in_key_selection_advanced_mode = true;
                    }
                });
        });

        ui.add_space(10.0);

        // Render additional key options only if "Advanced" mode is selected
        if self.in_key_selection_advanced_mode {
            if self.warming_identity_keys && !self.identity_keys.has_master() {
                ui.label("Preparing identity keys…");
            }
            // Render all keys in one grid
            self.render_keys_input(ui);
        } else {
            ui.colored_label(Color32::DARK_GREEN, "Default allows for most operations on Platform: updating the identity, interacting with data contracts, transferring credits to other identities, and withdrawing to the Core payment chain. More keys can always be added later.".to_string());
        }
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];
        // Per-row "Show WIF" requests collected inside the table closure and
        // applied after, to avoid borrowing `self` while the table borrows
        // `self.identity_keys`. Each is (key id, derivation path).
        let mut wif_requests: Vec<(u32, DerivationPath)> = vec![];
        let has_master_key = self.identity_keys.master.is_some();
        let has_other_keys = !self.identity_keys.others.is_empty();

        if has_master_key || has_other_keys {
            let row_height = 30.0;

            // Use a lighter stripe color that doesn't clash with comboboxes
            let original_stripe_color = ui.visuals().faint_bg_color;
            let dark_mode = ui.style().visuals.dark_mode;
            ui.visuals_mut().faint_bg_color = DashColors::stripe(dark_mode);

            let revealed_wifs = &self.revealed_wifs;

            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .vscroll(false)
                .cell_layout(egui::Layout::left_to_right(Align::Center))
                .column(Column::auto().at_least(80.0)) // Key
                .column(Column::auto().at_least(200.0)) // WIF
                .column(Column::auto().at_least(120.0)) // Purpose
                .column(Column::auto().at_least(120.0)) // Type
                .column(Column::auto().at_least(100.0)) // Security
                .column(Column::auto().at_least(30.0)) // Delete
                .header(row_height, |mut header| {
                    header.col(|ui| {
                        ui.label("Key");
                    });
                    header.col(|ui| {
                        ui.label("WIF");
                    });
                    header.col(|ui| {
                        ui.label("Purpose");
                    });
                    header.col(|ui| {
                        ui.label("Type");
                    });
                    header.col(|ui| {
                        ui.label("Security");
                    });
                    header.col(|_ui| {});
                })
                .body(|mut body| {
                    // Render master key first
                    if let Some(master) = self.identity_keys.master.as_mut() {
                        body.row(row_height, |mut row| {
                            row.col(|ui| {
                                ui.label("Master Key");
                            });
                            row.col(|ui| {
                                Self::render_wif_cell(
                                    ui,
                                    0,
                                    &master.derivation_path,
                                    revealed_wifs,
                                    &mut wif_requests,
                                );
                            });
                            row.col(|_ui| {
                                // No purpose for master key
                            });
                            row.col(|ui| {
                                ui.vertical(|ui| {
                                    ComboBox::from_id_salt("master_key_type")
                                        .selected_text(format!("{:?}", master.key_type))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut master.key_type,
                                                KeyType::ECDSA_SECP256K1,
                                                "ECDSA_SECP256K1",
                                            );
                                            ui.selectable_value(
                                                &mut master.key_type,
                                                KeyType::ECDSA_HASH160,
                                                "ECDSA_HASH160",
                                            );
                                        });
                                });
                            });
                            row.col(|_ui| {
                                // No security level for master key
                            });
                            row.col(|_ui| {
                                // No delete for master key
                            });
                        });
                    }

                    // Render other keys
                    for (i, entry) in self.identity_keys.others.iter_mut().enumerate() {
                        let key_id = (i + 1) as u32;
                        body.row(row_height, |mut row| {
                            row.col(|ui| {
                                ui.label(format!("Key {}", i + 1));
                            });
                            row.col(|ui| {
                                Self::render_wif_cell(
                                    ui,
                                    key_id,
                                    &entry.derivation_path,
                                    revealed_wifs,
                                    &mut wif_requests,
                                );
                            });
                            row.col(|ui| {
                                ui.vertical(|ui| {
                                    let prev_purpose = entry.purpose;
                                    ComboBox::from_id_salt(format!("purpose_combo_{}", i))
                                        .selected_text(format!("{:?}", entry.purpose))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut entry.purpose,
                                                Purpose::AUTHENTICATION,
                                                "AUTHENTICATION",
                                            );
                                            ui.selectable_value(
                                                &mut entry.purpose,
                                                Purpose::TRANSFER,
                                                "TRANSFER",
                                            );
                                            ui.selectable_value(
                                                &mut entry.purpose,
                                                Purpose::ENCRYPTION,
                                                "ENCRYPTION",
                                            );
                                            ui.selectable_value(
                                                &mut entry.purpose,
                                                Purpose::DECRYPTION,
                                                "DECRYPTION",
                                            );
                                        });
                                    // Auto-set security level when purpose changes
                                    if entry.purpose != prev_purpose {
                                        match entry.purpose {
                                            Purpose::TRANSFER => {
                                                entry.security_level = SecurityLevel::CRITICAL;
                                            }
                                            Purpose::ENCRYPTION | Purpose::DECRYPTION => {
                                                entry.security_level = SecurityLevel::MEDIUM;
                                            }
                                            Purpose::AUTHENTICATION => {
                                                if entry.security_level != SecurityLevel::CRITICAL
                                                    && entry.security_level != SecurityLevel::HIGH
                                                    && entry.security_level != SecurityLevel::MEDIUM
                                                {
                                                    entry.security_level = SecurityLevel::CRITICAL;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                });
                            });
                            row.col(|ui| {
                                ui.vertical(|ui| {
                                    ComboBox::from_id_salt(format!("key_type_combo_{}", i))
                                        .selected_text(format!("{:?}", entry.key_type))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut entry.key_type,
                                                KeyType::ECDSA_HASH160,
                                                "ECDSA_HASH160",
                                            );
                                            ui.selectable_value(
                                                &mut entry.key_type,
                                                KeyType::ECDSA_SECP256K1,
                                                "ECDSA_SECP256K1",
                                            );
                                        });
                                });
                            });
                            row.col(|ui| {
                                ui.vertical(|ui| {
                                    ComboBox::from_id_salt(format!("security_level_combo_{}", i))
                                        .selected_text(format!("{:?}", entry.security_level))
                                        .show_ui(ui, |ui| {
                                            if entry.purpose == Purpose::TRANSFER {
                                                entry.security_level = SecurityLevel::CRITICAL;
                                                ui.label("Locked to CRITICAL");
                                            } else if entry.purpose == Purpose::ENCRYPTION
                                                || entry.purpose == Purpose::DECRYPTION
                                            {
                                                entry.security_level = SecurityLevel::MEDIUM;
                                                ui.label("Locked to MEDIUM");
                                            } else {
                                                ui.selectable_value(
                                                    &mut entry.security_level,
                                                    SecurityLevel::CRITICAL,
                                                    "CRITICAL",
                                                );
                                                ui.selectable_value(
                                                    &mut entry.security_level,
                                                    SecurityLevel::HIGH,
                                                    "HIGH",
                                                );
                                                ui.selectable_value(
                                                    &mut entry.security_level,
                                                    SecurityLevel::MEDIUM,
                                                    "MEDIUM",
                                                );
                                            }
                                        });
                                });
                            });
                            row.col(|ui| {
                                if ui.button("-").clicked() {
                                    keys_to_remove.push(i);
                                }
                            });
                        });
                    }
                });

            // Restore original stripe color
            ui.visuals_mut().faint_bg_color = original_stripe_color;
        }

        // Apply any "Show WIF" request — only the most recent click matters.
        if let Some(request) = wif_requests.pop() {
            self.pending_wif_request = Some(request);
        }

        // Remove keys marked for deletion (revealed WIFs become stale).
        if !keys_to_remove.is_empty() {
            self.revealed_wifs.clear();
        }
        for i in keys_to_remove.iter().rev() {
            self.identity_keys.others.remove(*i);
        }

        // Add new key input entry
        ui.add_space(15.0);
        if ui.button("+ Add Key").clicked() {
            self.add_identity_key(
                KeyType::ECDSA_HASH160,  // Default key type
                Purpose::AUTHENTICATION, // Default purpose
                SecurityLevel::HIGH,     // Default security level
            );
        }
    }

    /// Render the advanced-mode WIF cell for one key: the revealed WIF if
    /// already derived, otherwise a "Show WIF" button that queues a
    /// just-in-time backend derivation. The seed never reaches `ui()` — only
    /// the derived WIF (wrapped in [`Secret`]) comes back via a backend task.
    fn render_wif_cell(
        ui: &mut egui::Ui,
        key_id: u32,
        derivation_path: &DerivationPath,
        revealed_wifs: &HashMap<u32, Secret>,
        wif_requests: &mut Vec<(u32, DerivationPath)>,
    ) {
        if let Some(wif) = revealed_wifs.get(&key_id) {
            // WIF displayed as plaintext label — user-initiated key view.
            // Secret wrapper provides zeroize-on-drop for the Rust-side variable.
            ui.label(wif.expose_secret());
        } else if ui.button("Show WIF").clicked() {
            wif_requests.push((key_id, derivation_path.clone()));
        }
    }

    fn register_identity_clicked(&mut self, funding_method: FundingMethod) -> AppAction {
        let Some(selected_wallet) = &self.selected_wallet else {
            return AppAction::None;
        };
        // Fail-closed: the key set is only populated once the auth-pubkey cache
        // is warm. A cold cache leaves `master` empty, so registration is
        // blocked until the keys are ready (RK-2).
        if !self.identity_keys.has_master() {
            return AppAction::None;
        };
        match funding_method {
            FundingMethod::UseUnusedAssetLock => {
                if let Some(out_point) = self.funding_asset_lock {
                    let identity_input = IdentityRegistrationInfo {
                        alias_input: self.alias_input.clone(),
                        keys: self.identity_keys.clone(),
                        wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                        wallet_identity_index: self.identity_id_number,
                        identity_funding_method: RegisterIdentityFundingMethod::UseAssetLock {
                            out_point,
                            identity_index: self.identity_id_number,
                        },
                    };

                    let mut step = self.step.write_recover();
                    *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;

                    AppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::RegisterIdentity(identity_input),
                    ))
                } else {
                    AppAction::None
                }
            }
            // A received deposit lands in the wallet balance, so it funds
            // through the same wallet-balance path once it arrives.
            FundingMethod::UseWalletBalance | FundingMethod::ReceiveDeposit => {
                // Get the funding amount in duffs from the Amount
                let amount = self
                    .funding_amount
                    .as_ref()
                    .map(|a| a.value() / 1000) // Convert credits to duffs
                    .unwrap_or(0);

                if amount == 0 {
                    return AppAction::None;
                }
                let seed_hash = selected_wallet.read_recover().seed_hash();
                let Some(max_amount) = self.asset_lock_balance.get(&seed_hash) else {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Your wallet's available amount is still being checked. Wait a moment and try again.",
                        MessageType::Warning,
                    );
                    return AppAction::None;
                };
                let key_count = self.identity_keys.others.len() + 1;
                let identity_fee_duffs = self
                    .app_context
                    .fee_estimator()
                    .estimate_identity_create(key_count)
                    .div_ceil(CREDITS_PER_DUFF);
                if let Err(error) =
                    validate_asset_lock_amount(amount, identity_fee_duffs, max_amount)
                {
                    let maximum_amount_duffs = match error {
                        AssetLockAmountError::Overflow => max_amount,
                        AssetLockAmountError::ExceedsMaximum {
                            maximum_amount_duffs,
                        } => maximum_amount_duffs,
                    };
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        format!(
                            "You can transfer up to {} right now. Choose a smaller amount or wait for more funds.",
                            format_duffs_as_dash(maximum_amount_duffs)
                        ),
                        MessageType::Warning,
                    );
                    return AppAction::None;
                }

                let wallet_seed_hash = hex::encode(selected_wallet.read_recover().seed_hash());
                tracing::debug!(wallet_seed_hash, "funding with wallet balance");
                let identity_input = IdentityRegistrationInfo {
                    alias_input: self.alias_input.clone(),
                    keys: self.identity_keys.clone(),
                    wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                    wallet_identity_index: self.identity_id_number,
                    identity_funding_method: RegisterIdentityFundingMethod::FundWithWallet(
                        amount,
                        self.identity_id_number,
                    ),
                };

                let mut step = self.step.write_recover();
                *step = WalletFundedScreenStep::WaitingForAssetLock;

                // Create the backend task to register the identity
                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                    identity_input,
                )))
            }
            FundingMethod::UsePlatformAddress => {
                // Get selected Platform address and amount from the input fields
                let Some((platform_addr, amount)) = self.selected_platform_address_for_funding
                else {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Please select a Platform address",
                        MessageType::Error,
                    );
                    return AppAction::None;
                };

                if amount == 0 {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Amount must be greater than 0",
                        MessageType::Error,
                    );
                    return AppAction::None;
                }

                let wallet_seed_hash = selected_wallet.read_recover().seed_hash();

                let mut inputs = std::collections::BTreeMap::new();
                inputs.insert(platform_addr, amount);

                let identity_input = IdentityRegistrationInfo {
                    alias_input: self.alias_input.clone(),
                    keys: self.identity_keys.clone(),
                    wallet: Arc::clone(selected_wallet),
                    wallet_identity_index: self.identity_id_number,
                    identity_funding_method:
                        RegisterIdentityFundingMethod::FundWithPlatformAddresses {
                            inputs,
                            wallet_seed_hash,
                        },
                };

                let mut step = self.step.write_recover();
                *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;

                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                    identity_input,
                )))
            }
            _ => AppAction::None,
        }
    }

    fn render_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = *self.funding_method.read_recover();

        let wallet_ceiling_duffs = || {
            self.selected_wallet
                .as_ref()
                .and_then(|wallet| wallet.read().ok())
                .and_then(|wallet| self.asset_lock_balance.get(&wallet.seed_hash()))
                .unwrap_or(0)
        };
        let available_ceiling_duffs = match funding_method {
            FundingMethod::UseWalletBalance => Some(wallet_ceiling_duffs()),
            FundingMethod::ReceiveDeposit => Some(receive_deposit_ceiling_duffs(
                wallet_ceiling_duffs(),
                self.funding_address_balance_duffs,
            )),
            _ => None,
        };

        // Reserve the estimated identity-creation fee from the relevant ceiling.
        let (max_amount_credits, show_max_button, fee_hint) =
            if let Some(available_ceiling_duffs) = available_ceiling_duffs {
                let key_count = self.identity_keys.others.len() + 1; // +1 for master key
                let estimated_fee = self
                    .app_context
                    .fee_estimator()
                    .estimate_identity_create(key_count);
                let max_with_fee_reserved =
                    max_amount_after_fee_reserve(available_ceiling_duffs, estimated_fee);
                (
                    Some(max_with_fee_reserved),
                    true,
                    Some(format!(
                        "The estimated fee reserves about {}.",
                        format_credits_as_dash(estimated_fee),
                    )),
                )
            } else {
                (None, false, None)
            };

        let should_prefill = self.prefill_funding_amount;
        let amount_input = self.funding_amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::new_dash(0.0))
                .with_label("Amount (DASH):")
                .with_hint_text("Enter amount (e.g., 0.1234)")
                .with_max_button(show_max_button)
                .with_desired_width(150.0)
        });

        // Update max amount and max button visibility dynamically
        amount_input
            .set_max_amount(max_amount_credits)
            .set_show_max_button(show_max_button)
            .set_max_exceeded_hint(fee_hint);

        // Pre-fill (once) with the fee-reserve-capped maximum when a deposit just
        // arrived, so the amount and Create button are populated but still editable.
        if should_prefill && let Some(max) = max_amount_credits {
            amount_input.set_value(Amount::dash_from_credits(max));
        }

        let response = amount_input.show(ui);
        response.inner.update(&mut self.funding_amount);

        if should_prefill {
            self.prefill_funding_amount = false;
        }

        ui.add_space(10.0);
    }

    /// The optional local-alias step (design-spec §B.10: fund-first).
    ///
    /// Rendered by each funding-method branch just before its Create/Register
    /// button, once the amount or lock for that method is chosen. This is a
    /// Dash Evo Tool alias stored locally, not a DPNS username.
    fn render_alias_input(&mut self, ui: &mut egui::Ui, step_number: u32) {
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.heading(format!("{step_number}. Set a local alias (optional)."));
            crate::ui::helpers::info_icon_button(
                ui,
                "This is a local alias stored only in Dash Evo Tool to help you identify this identity.\n\n\
                This is NOT a DPNS username. DPNS names are registered on-chain after creating the identity.\n\n\
                You can change this alias anytime from the identity details screen.",
            );
        });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Alias:");
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(
                egui::TextEdit::singleline(&mut self.alias_input)
                    .hint_text(
                        egui::RichText::new("e.g., My Main Identity")
                            .color(DashColors::text_secondary(dark_mode)),
                    )
                    .desired_width(250.0),
            );
        });

        let dark_mode = ui.style().visuals.dark_mode;
        ui.label(
            egui::RichText::new("Note: This is a Dash Evo Tool alias, not a DPNS username.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );

        ui.add_space(10.0);
    }

    /// The key id (0 = master, others id = index + 1) whose derivation path
    /// matches `path`, used to file a returned WIF into the right row.
    fn key_id_for_path(&self, path: &DerivationPath) -> Option<u32> {
        if let Some(master) = &self.identity_keys.master
            && &master.derivation_path == path
        {
            return Some(0);
        }
        self.identity_keys
            .others
            .iter()
            .position(|entry| &entry.derivation_path == path)
            .map(|i| (i + 1) as u32)
    }

    /// Add one advanced-mode key at the next index, reading its **public** key
    /// from the auth-pubkey cache. On a cache miss the next index is warmed
    /// (the key appears once the cache fills); manually added keys carry no
    /// contract bounds.
    fn add_identity_key(
        &mut self,
        key_type: KeyType,
        purpose: Purpose,
        security_level: SecurityLevel,
    ) {
        let Some(wallet_lock) = self.selected_wallet.clone() else {
            return;
        };
        let seed_hash = wallet_lock.read_recover().seed_hash();
        let network = self.app_context.network;
        let identity_index = self.identity_id_number;
        let new_key_index = self.identity_keys.others.len() as u32 + 1;

        let Ok(backend) = self.app_context.wallet_backend() else {
            return;
        };
        let cache = backend.auth_pubkey_cache().get(network, &seed_hash);
        match cache.get(network, identity_index, new_key_index) {
            Some(public_key) => {
                self.identity_keys
                    .others
                    .push(IdentityKeyEntry::from_cached_public_key(
                        public_key,
                        network,
                        identity_index,
                        new_key_index,
                        key_type,
                        purpose,
                        security_level,
                        None,
                    ));
            }
            None => {
                // Warm enough keys to cover the new index; the chooser rebuilds
                // (and the key appears) once the cache is filled.
                self.warming_identity_keys = false;
                self.pending_warm_request = Some((seed_hash, identity_index));
            }
        }
    }
}

impl ScreenLike for AddNewIdentityScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            let mut step = self.step.write_recover();
            *step = step_after_task_failure(*step);
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if let Some((seed_hash, snapshot_generation)) = context.asset_lock_max_amount_request() {
            self.asset_lock_balance
                .mark_loading_failed(&seed_hash, snapshot_generation);
        }
        let selected_seed_hash = self
            .selected_wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()));
        if self.funding_address_request_in_flight
            && context.generated_receive_address_wallet() == selected_seed_hash
        {
            self.funding_address_request_in_flight = false;
            self.funding_address_request_failed = true;
        }
    }

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.asset_lock_max_amount_request().is_some()
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match &backend_task_success_result {
            BackendTaskSuccessResult::IdentityAuthPubkeysWarmed { .. } => {
                // Cache is now warm; re-read the public keys for the current
                // selection (cache hit, no seed access).
                self.warming_identity_keys = false;
                self.ensure_correct_identity_keys();
                return;
            }
            BackendTaskSuccessResult::WalletKeyForDisplay {
                derivation_path,
                wif,
                ..
            } => {
                if let Some(key_id) = self.key_id_for_path(derivation_path) {
                    self.revealed_wifs.insert(key_id, wif.clone());
                }
                return;
            }
            BackendTaskSuccessResult::TrackedAssetLocks { seed_hash, locks } => {
                self.asset_lock_cache.store(*seed_hash, locks.clone());
                return;
            }
            BackendTaskSuccessResult::AssetLockMaxAmount {
                seed_hash,
                snapshot_generation,
                amount_duffs,
            } => {
                self.asset_lock_balance
                    .store(*seed_hash, *snapshot_generation, *amount_duffs);
                return;
            }
            BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address } => {
                // Adopt the SPV-watched deposit address only for the selected
                // wallet, so a stale result for another wallet is ignored.
                let is_ours = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok())
                    .map(|w| w.seed_hash() == *seed_hash)
                    .unwrap_or(false);
                if is_ours {
                    self.funding_address_request_in_flight = false;
                    match address.parse::<Address<_>>() {
                        Ok(addr) => {
                            self.funding_address = Some(addr.assume_checked());
                            self.funding_address_request_failed = false;
                        }
                        Err(e) => {
                            self.funding_address_request_failed = true;
                            MessageBanner::set_global(
                                self.app_context.egui_ctx(),
                                "Could not prepare a deposit address. Choose a different \
                                 funding method, or try again.",
                                MessageType::Error,
                            )
                            .with_details(e);
                        }
                    }
                }
                return;
            }
            _ => {}
        }

        if let BackendTaskSuccessResult::RegisteredIdentity(qualified_identity, fee_result) =
            backend_task_success_result
        {
            self.successful_qualified_identity_id = Some(qualified_identity.identity.id());
            self.completed_fee_result = Some(fee_result);
            let mut step = self.step.write_recover();
            *step = WalletFundedScreenStep::Success;
            return;
        }

        let mut step = self.step.write_recover();
        let current_step = *step;
        match current_step {
            WalletFundedScreenStep::ChooseFundingMethod => {}
            WalletFundedScreenStep::WaitingOnFunds => {
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(_, outputs),
                ) = &backend_task_success_result
                {
                    let key_count = self.identity_keys.others.len() + 1; // +1 for master key
                    let minimum_credits = self
                        .app_context
                        .fee_estimator()
                        .estimate_identity_create(key_count);
                    let (next, prefill) = deposit_event_outcome(
                        current_step,
                        self.funding_address.as_ref(),
                        outputs,
                        minimum_credits,
                    );
                    // Pre-fill the amount with the fee-reserve-capped balance when
                    // the deposit lands, so the field and Create button populate.
                    if prefill.is_some() {
                        self.prefill_funding_amount = true;
                    }
                    *step = next;
                }
            }
            WalletFundedScreenStep::FundsReceived => {}
            WalletFundedScreenStep::ReadyToCreate => {}
            WalletFundedScreenStep::WaitingForAssetLock => {
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                ) = &backend_task_success_result
                    && let Some(TransactionPayload::AssetLockPayloadType(asset_lock_payload)) =
                        &tx.special_transaction_payload
                    && asset_lock_payload.credit_outputs.iter().any(|tx_out| {
                        let Ok(address) =
                            Address::from_script(&tx_out.script_pubkey, self.app_context.network)
                        else {
                            return false;
                        };
                        if let Some(wallet) = &self.selected_wallet {
                            let wallet = wallet.read_recover();
                            wallet.known_addresses.contains_key(&address)
                        } else {
                            false
                        }
                    })
                {
                    *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
                }
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {}
            WalletFundedScreenStep::Success => {}
        }
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        // Flip an in-flight asset-lock fetch to a retryable state so the picker
        // shows a Retry button instead of a permanent "Loading…".
        self.asset_lock_cache.mark_loading_failed();
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Create Identity", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;

            ScrollArea::vertical().show(ui, |ui| {
                let step = {*self.step.read_recover()};
                if step == WalletFundedScreenStep::Success {
                    inner_action |= self.show_success(ui);
                    return;
                }
                ui.add_space(10.0);

                // Heading with checkbox on the same line
                ui.horizontal(|ui| {
                    ui.heading("Follow these steps to create your identity.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Show Advanced Options");
                    });
                });
                ui.add_space(15.0);

                let mut step_number = 1;

                if self.render_wallet_selection(ui) {
                    // We had more than 1 wallet
                    step_number += 1;
                }

                if self.selected_wallet.is_none() {
                    ui.add_space(10.0);
                    ui.colored_label(
                        DashColors::WARNING,
                        "You need a wallet before you can create an identity.",
                    );
                    ui.add_space(8.0);
                    if ui.button("Set up a wallet").clicked() {
                        inner_action |= AppAction::SetMainScreenThenGoToMainScreen(
                            crate::ui::RootScreenType::RootScreenWalletsBalances,
                        );
                    }
                    return;
                };

                // Check if wallet needs unlocking
                let wallet = self
                    .selected_wallet
                    .as_ref()
                    .expect("invariant: selected_wallet checked Some above");

                // Try to open wallet without password if it doesn't use one
                if !self.wallet_open_attempted {
                    if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                            .disable_auto_dismiss();
                    }
                    self.wallet_open_attempted = true;
                }

                // If wallet needs password unlock
                if wallet_needs_unlock(wallet) {
                    // Show message and button to unlock
                    ui.add_space(10.0);
                    ui.colored_label(
                        Color32::from_rgb(200, 150, 50),
                        "Wallet is locked. Please unlock to continue.",
                    );
                    ui.add_space(8.0);
                    if ui.button("Unlock Wallet").clicked() {
                        self.wallet_unlock_popup.open();
                    }
                    return;
                }

                // Only show identity index and key selection in advanced mode
                if self.show_advanced_options {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Display the heading with an info icon that shows a tooltip on hover
                    ui.horizontal(|ui| {
                        let wallet_guard = self
                            .selected_wallet
                            .as_ref()
                            .expect("invariant: selected_wallet checked Some above");
                        let wallet = wallet_guard.read_recover();
                        if wallet.identities.is_empty() {
                            ui.heading(format!(
                                "{}. Choose an identity index for the wallet. Leaving this 0 is recommended.",
                                step_number
                            ));
                        } else {
                            ui.heading(format!(
                                "{}. Choose an identity index for the wallet. Leaving this {} is recommended.",
                                step_number,
                                self.next_identity_id(),
                            ));
                        }


                        // Create info icon button with tooltip
                        let response = crate::ui::helpers::info_icon_button(ui, "The identity index is an internal reference within the wallet. The wallet's seed phrase can always be used to recover any identity, including this one, by using the same index.");

                        // Check if the label was clicked
                        if response.clicked() {
                            self.show_pop_up_info = Some("The identity index is an internal reference within the wallet. The wallet's seed phrase can always be used to recover any identity, including this one, by using the same index.".to_string());
                        }
                    });

                    step_number += 1;

                    ui.add_space(8.0);

                    self.render_identity_index_input(ui);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Display the heading with an info icon that shows a tooltip on hover
                    ui.horizontal(|ui| {
                        ui.heading(format!(
                            "{}. Choose what keys you want to add to this new identity.",
                            step_number
                        ));

                        // Create info icon button with tooltip
                        let response = crate::ui::helpers::info_icon_button(ui, "Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.");

                        // Check if the label was clicked
                        if response.clicked() {
                            self.show_pop_up_info = Some("Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.".to_string());
                        }
                    });

                    step_number += 1;

                    ui.add_space(8.0);

                    self.render_key_selection(ui);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Fund-first (design-spec §B.10): the funding method chooser is the
                // first everyday-facing step. The local alias (optional) moves to a
                // later step, rendered just before the Create button for whichever
                // funding method is chosen (see `render_alias_input`).
                ui.heading(
                    format!("{}. Choose your funding method.", step_number).as_str()
                );
                step_number += 1;

                ui.add_space(10.0);
                self.render_funding_method(ui);
                ui.add_space(10.0);
                ui.separator();

                // Extract the funding method from the RwLock to minimize borrow scope
                let funding_method = *self.funding_method.read_recover();

                if funding_method == FundingMethod::NoSelection {
                    return;
                }

                match funding_method {
                    FundingMethod::NoSelection => (),
                    FundingMethod::UseUnusedAssetLock => {
                        inner_action |= self.render_ui_by_using_unused_asset_lock(ui, step_number);
                    },
                    FundingMethod::UseWalletBalance => {
                        inner_action |= self.render_ui_by_using_unused_balance(ui, step_number);
                    },
                    FundingMethod::UsePlatformAddress => {
                        inner_action |= self.render_ui_by_platform_address(ui, step_number);
                    },
                    FundingMethod::ReceiveDeposit => {
                        inner_action |= self.render_ui_by_receive_deposit(ui, step_number);
                    },
                }
            });
            inner_action
        });

        // Show the info popup if requested
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("create_identity_info_popup"),
                        "Identity Information",
                        &show_pop_up_info_text,
                    );
                    if popup.show(ui).inner {
                        self.show_pop_up_info = None;
                    }
                });
        }

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet was unlocked, update dependencies
                self.update_wallet(wallet.clone());
            }
        }

        // Drain the queued end-of-frame backend reads into one concurrent batch
        // so none clobbers another (`AppAction`'s `|=` keeps only the last
        // value).
        let mut pending_tasks: Vec<BackendTask> = Vec::new();

        // Auth-pubkey cache warm (cold-cache cover for the chooser, RK-2). One
        // in-flight at a time via `warming_identity_keys`.
        if let Some((seed_hash, identity_index)) = self.pending_warm_request.take() {
            // Warm at least the default range, plus a margin for any
            // advanced-mode keys already added beyond it.
            let key_count = self
                .default_key_count()
                .max(self.identity_keys.others.len() as u32 + 2);
            pending_tasks.push(BackendTask::WalletTask(
                WalletTask::WarmIdentityAuthPubkeys {
                    seed_hash,
                    identity_index,
                    key_count,
                },
            ));
        }

        // "Show WIF" derivation (advanced mode); the seed is fetched
        // just-in-time in the backend and only the WIF returns.
        if let Some((_key_id, derivation_path)) = self.pending_wif_request.take()
            && let Some(wallet) = &self.selected_wallet
        {
            let seed_hash = wallet.read_recover().seed_hash();
            pending_tasks.push(BackendTask::WalletTask(WalletTask::DeriveKeyForDisplay {
                seed_hash,
                derivation_path,
            }));
        }

        // Fetch the selected wallet's tracked asset locks once (off the UI
        // thread) so the funding-method gate and the picker can read them.
        if let Some(wallet) = &self.selected_wallet {
            let seed_hash = wallet.read_recover().seed_hash();
            if let Some(task) = self.asset_lock_cache.ensure_requested(seed_hash) {
                pending_tasks.push(task);
            }
        }

        // Derive the "Receive a new deposit" address off the UI thread; the QR
        // view queues this when it has no address yet.
        if let Some(seed_hash) = self.pending_funding_address_request.take() {
            self.funding_address_request_in_flight = true;
            pending_tasks.push(BackendTask::WalletTask(
                WalletTask::GenerateReceiveAddress { seed_hash },
            ));
        }

        match pending_tasks.pop() {
            None => {}
            Some(task) if pending_tasks.is_empty() => action |= AppAction::BackendTask(task),
            Some(task) => {
                pending_tasks.push(task);
                action |=
                    AppAction::BackendTasks(pending_tasks, BackendTasksExecutionMode::Concurrent)
            }
        }

        action
    }
}

#[cfg(test)]
mod funding_method_tests {
    use super::format_wallet_picker_label;

    /// The picker label pairs the wallet alias with its spendable balance,
    /// rendered in DASH, so the user can compare wallets before choosing one.
    /// 0.5 DASH == 50_000_000 duffs.
    #[test]
    fn wallet_picker_label_shows_spendable_balance_in_dash() {
        assert_eq!(
            format_wallet_picker_label("Main", 50_000_000),
            "Main — 0.5 DASH"
        );
    }

    /// A zero-balance wallet still renders a well-formed label rather than an
    /// empty or unit-less string.
    #[test]
    fn wallet_picker_label_renders_zero_balance() {
        assert_eq!(format_wallet_picker_label("Empty", 0), "Empty — 0 DASH");
    }

    /// Structural guard: the label keeps the alias, an em-dash separator, and
    /// the DASH unit — the shape UI code and any future i18n extraction rely on.
    #[test]
    fn wallet_picker_label_keeps_alias_separator_and_unit() {
        let label = format_wallet_picker_label("Savings", 12_345_678);
        assert!(label.starts_with("Savings"), "keeps the alias: {label}");
        assert!(label.contains(" — "), "uses an em-dash separator: {label}");
        assert!(label.ends_with(" DASH"), "shows the DASH unit: {label}");
    }
}
