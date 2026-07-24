mod by_platform_address;
mod by_receive_deposit;
mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod success_screen;

use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::core::CoreItem;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::MessageBanner;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::funding_common::{
    FundingMethod, WalletFundedScreenStep, default_funding_state, deposit_event_outcome,
    max_amount_after_fee_reserve, spendable_covers_minimum, step_after_task_failure,
    wallet_selection_combo,
};
use crate::ui::state::TrackedAssetLockCache;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits, Duffs};
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{ComboBox, ScrollArea, Ui};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

const WALLET_SELECTION_TOOLTIP: &str =
    "Choose the wallet that will supply or receive the Dash used to add funds to this identity.";

fn pending_backend_tasks_action(
    mut lock_fetches: Vec<BackendTask>,
    funding_address_request: Option<BackendTask>,
) -> AppAction {
    if let Some(task) = funding_address_request {
        lock_fetches.push(task);
    }
    match lock_fetches.pop() {
        None => AppAction::None,
        Some(task) if lock_fetches.is_empty() => AppAction::BackendTask(task),
        Some(task) => {
            lock_fetches.push(task);
            AppAction::BackendTasks(lock_fetches, BackendTasksExecutionMode::Concurrent)
        }
    }
}

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    /// Outpoint of an asset lock tracked by the upstream `AssetLockManager`,
    /// chosen by the user from the picker. Routed to the backend as
    /// `TopUpIdentityFundingMethod::UseAssetLock`.
    funding_asset_lock: Option<OutPoint>,
    wallet: Option<Arc<RwLock<Wallet>>>,
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
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    funding_amount_input: Option<AmountInput>,
    copied_to_clipboard: Option<Option<String>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    show_pop_up_info: Option<String>,
    pub app_context: Arc<AppContext>,
    // Platform address fields
    selected_platform_address: Option<(Address, PlatformAddress, Credits)>,
    platform_top_up_amount: Option<Amount>,
    platform_top_up_amount_input: Option<AmountInput>,
    /// Fee result from completed top-up
    completed_fee_result: Option<FeeResult>,
    /// Tracked asset locks per wallet, fetched off the UI thread via the App
    /// Task System. Backs the funding-method gate, the wallet selector, and the
    /// asset-lock picker.
    asset_lock_cache: TrackedAssetLockCache,
}

impl TopUpIdentityScreen {
    pub fn new(qualified_identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            identity: qualified_identity,
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            wallet: None,
            funding_address: None,
            pending_funding_address_request: None,
            funding_address_request_in_flight: false,
            funding_address_request_failed: false,
            funding_address_balance_duffs: 0,
            prefill_funding_amount: false,
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "".to_string(),
            funding_amount_exact: None,
            funding_amount_input: None,
            copied_to_clipboard: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            show_pop_up_info: None,
            app_context: app_context.clone(),
            selected_platform_address: None,
            platform_top_up_amount: None,
            platform_top_up_amount_input: None,
            completed_fee_result: None,
            asset_lock_cache: TrackedAssetLockCache::default(),
        }
    }

    /// Current funding step, defaulting to the initial chooser step if the lock
    /// is momentarily poisoned rather than panicking.
    fn current_step(&self) -> WalletFundedScreenStep {
        self.step
            .read()
            .map(|s| *s)
            .unwrap_or(WalletFundedScreenStep::ChooseFundingMethod)
    }

    /// Set the funding step, silently skipping the write if the lock is
    /// poisoned (a poisoned step lock never blocks the UI).
    fn set_step(&self, step: WalletFundedScreenStep) {
        if let Ok(mut s) = self.step.write() {
            *s = step;
        }
    }

    /// Current funding method, defaulting to `NoSelection` if the lock is
    /// momentarily poisoned rather than panicking.
    fn current_funding_method(&self) -> FundingMethod {
        self.funding_method
            .read()
            .map(|m| *m)
            .unwrap_or(FundingMethod::NoSelection)
    }

    /// Whether `wallet` currently has the resources the given funding method
    /// needs. A busy wallet lock reads as "no resources" rather than panicking.
    fn wallet_has_resources_for(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        method: FundingMethod,
    ) -> bool {
        let Ok(w) = wallet.read() else {
            return false;
        };
        match method {
            FundingMethod::UseWalletBalance => {
                self.app_context.snapshot_has_balance(&w.seed_hash())
            }
            FundingMethod::UseUnusedAssetLock => self.asset_lock_cache.has_unused(&w.seed_hash()),
            _ => true,
        }
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        let mut selected_wallet_update: Option<Arc<RwLock<Wallet>>> = None;
        let mut step_update_method: Option<FundingMethod> = None;

        let rendered = if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets: Vec<_> = self
                .app_context
                .wallets
                .read()
                .map(|guard| guard.values().cloned().collect())
                .unwrap_or_default();

            if wallets.len() > 1 {
                let funding_method = self.current_funding_method();
                selected_wallet_update = wallet_selection_combo(
                    ui,
                    "select_wallet",
                    &wallets,
                    self.wallet.as_ref(),
                    |wallet| {
                        wallet
                            .read()
                            .ok()
                            .and_then(|w| w.alias.clone())
                            .unwrap_or_else(|| "Unnamed Wallet".to_string())
                    },
                    |wallet| self.wallet_has_resources_for(wallet, funding_method),
                );
                if selected_wallet_update.is_some() {
                    step_update_method = Some(funding_method);
                }
                true
            } else if let Some(wallet) = wallets.first() {
                if self.wallet.is_none() {
                    // §B.9 / QA-006: the very first time a wallet resolves with
                    // nothing chosen yet, apply the same pre-selection the
                    // create-identity wizard uses (`default_funding_state`) —
                    // recommend `UseWalletBalance` only when this wallet can
                    // actually cover the estimated top-up fee. A dust or locked
                    // balance (positive but below the fee) must not pre-select a
                    // path the next render blocks on.
                    if self.current_funding_method() == FundingMethod::NoSelection {
                        let can_afford = wallet
                            .read()
                            .ok()
                            .map(|w| {
                                let spendable = self
                                    .app_context
                                    .snapshot_balance(&w.seed_hash())
                                    .spendable();
                                let minimum =
                                    self.app_context.fee_estimator().estimate_identity_topup();
                                spendable_covers_minimum(spendable, minimum)
                            })
                            .unwrap_or(false);
                        let (recommended, _) = default_funding_state(can_afford);
                        if let Ok(mut m) = self.funding_method.write() {
                            *m = recommended;
                        }
                    }

                    let funding_method = self.current_funding_method();
                    if funding_method != FundingMethod::NoSelection
                        && self.wallet_has_resources_for(wallet, funding_method)
                    {
                        // Automatically select the only available wallet.
                        selected_wallet_update = Some(wallet.clone());
                        step_update_method = Some(funding_method);
                    }
                }
                false
            } else {
                false
            }
        } else {
            false
        };

        if let Some(wallet) = selected_wallet_update {
            self.wallet = Some(wallet);
            self.wallet_open_attempted = false;
            self.funding_address = None;
            self.pending_funding_address_request = None;
            self.funding_address_request_in_flight = false;
            self.funding_address_request_failed = false;
            self.funding_address_balance_duffs = 0;
            self.prefill_funding_amount = false;
            self.funding_asset_lock = None;
            self.funding_amount_input = None;
            self.copied_to_clipboard = None;

            if let Some(method) = step_update_method {
                self.update_step_after_wallet_change(method);
            } else {
                self.set_step(WalletFundedScreenStep::ChooseFundingMethod);
            }
        }

        rendered
    }

    /// Adjust the current step to match the funding method after a wallet switch.
    fn update_step_after_wallet_change(&mut self, funding_method: FundingMethod) {
        self.set_step(match funding_method {
            FundingMethod::UseUnusedAssetLock
            | FundingMethod::UseWalletBalance
            | FundingMethod::UsePlatformAddress => WalletFundedScreenStep::ReadyToCreate,
            FundingMethod::ReceiveDeposit => WalletFundedScreenStep::WaitingOnFunds,
            FundingMethod::NoSelection => WalletFundedScreenStep::ChooseFundingMethod,
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
        self.set_step(step);
        self.funding_address = None;
        self.pending_funding_address_request = None;
        self.funding_address_request_in_flight = false;
        self.funding_address_request_failed = false;
        self.funding_address_balance_duffs = 0;
        self.prefill_funding_amount = false;
        self.funding_amount_input = None;
        self.funding_amount_exact = None;
        self.funding_amount.clear();
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let funding_method_arc = self.funding_method.clone();
        let Ok(mut funding_method) = funding_method_arc.write() else {
            return;
        };

        // Check if any wallet has unused asset locks, balance, or Platform address balance
        let (has_any_unused_asset_lock, has_any_balance, has_any_platform_balance) = {
            let mut has_unused_asset_lock = false;
            let mut has_balance = false;
            let mut has_platform_balance = false;

            if let Ok(wallets) = self.app_context.wallets.read() {
                for wallet in wallets.values() {
                    let Ok(wallet) = wallet.read() else {
                        continue;
                    };
                    let seed_hash = wallet.seed_hash();
                    // Offer the option on a failed fetch too, so the user can
                    // reach the picker's Retry rather than the option vanishing.
                    if self.asset_lock_cache.has_unused(&seed_hash)
                        || self.asset_lock_cache.is_failed(&seed_hash)
                    {
                        has_unused_asset_lock = true;
                    }
                    if self.app_context.snapshot_has_balance(&seed_hash) {
                        has_balance = true;
                    }
                    if wallet.total_platform_balance() > 0 {
                        has_platform_balance = true;
                    }
                    if has_unused_asset_lock && has_balance && has_platform_balance {
                        break; // No need to check further
                    }
                }
            }

            (has_unused_asset_lock, has_balance, has_platform_balance)
        };

        ComboBox::from_id_salt("funding_method")
            .selected_text(funding_method.top_up_label())
            .height(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::NoSelection,
                    FundingMethod::NoSelection.top_up_label(),
                );

                ui.add_enabled_ui(has_any_unused_asset_lock, |ui| {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseUnusedAssetLock,
                            FundingMethod::UseUnusedAssetLock.top_up_label(),
                        )
                        .changed()
                    {
                        self.set_step(WalletFundedScreenStep::ReadyToCreate);
                    }
                });

                ui.add_enabled_ui(has_any_balance, |ui| {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseWalletBalance,
                            FundingMethod::UseWalletBalance.top_up_label(),
                        )
                        .changed()
                    {
                        self.set_step(WalletFundedScreenStep::ReadyToCreate);
                    }
                });

                ui.add_enabled_ui(has_any_platform_balance, |ui| {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UsePlatformAddress,
                            FundingMethod::UsePlatformAddress.top_up_label(),
                        )
                        .changed()
                    {
                        self.set_step(WalletFundedScreenStep::ReadyToCreate);
                    }
                });

                // "Receive a new deposit" is always offered: it needs no existing
                // balance or asset lock, it creates the funds the top-up will use.
                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::ReceiveDeposit,
                        FundingMethod::ReceiveDeposit.top_up_label(),
                    )
                    .changed()
                {
                    self.set_step(WalletFundedScreenStep::WaitingOnFunds);
                    self.funding_address = None;
                    self.pending_funding_address_request = None;
                    self.funding_address_request_in_flight = false;
                    self.funding_address_request_failed = false;
                    self.funding_address_balance_duffs = 0;
                    self.prefill_funding_amount = false;
                    self.funding_amount_input = None;
                    self.funding_amount_exact = None;
                    self.funding_amount.clear();
                }
            });
    }

    fn top_up_identity_clicked(&mut self, funding_method: FundingMethod) -> AppAction {
        let Some(selected_wallet) = &self.wallet else {
            return AppAction::None;
        };
        match funding_method {
            FundingMethod::UseUnusedAssetLock => {
                if let Some(out_point) = self.funding_asset_lock {
                    let identity_index = self.identity.wallet_index.unwrap_or(u32::MAX >> 1);
                    let top_up_index = self
                        .identity
                        .top_ups
                        .keys()
                        .max()
                        .cloned()
                        .map(|i| i + 1)
                        .unwrap_or_default();
                    let identity_input = IdentityTopUpInfo {
                        qualified_identity: self.identity.clone(),
                        wallet: Arc::clone(selected_wallet),
                        identity_funding_method: TopUpIdentityFundingMethod::UseAssetLock {
                            out_point,
                            identity_index,
                            top_up_index,
                        },
                    };

                    self.set_step(WalletFundedScreenStep::WaitingForPlatformAcceptance);

                    AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                        identity_input,
                    )))
                } else {
                    AppAction::None
                }
            }
            // A received deposit lands in the wallet balance, so it tops up
            // through the same wallet-balance path once it arrives.
            FundingMethod::UseWalletBalance | FundingMethod::ReceiveDeposit => {
                // Parse the funding amount or fall back to the default value
                let amount = self.funding_amount_exact.unwrap_or_else(|| {
                    (self.funding_amount.parse::<f64>().unwrap_or(0.0) * 1e8) as u64
                });

                if amount == 0 {
                    return AppAction::None;
                }
                if funding_method == FundingMethod::ReceiveDeposit {
                    let fee_credits = self.app_context.fee_estimator().estimate_identity_topup();
                    let available_credits = max_amount_after_fee_reserve(
                        self.funding_address_balance_duffs,
                        fee_credits,
                    );
                    if amount.saturating_mul(CREDITS_PER_DUFF) > available_credits {
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "That deposit cannot cover this amount. Wait for more funds or choose a smaller amount.",
                            MessageType::Warning,
                        );
                        return AppAction::None;
                    }
                }
                let identity_input = IdentityTopUpInfo {
                    qualified_identity: self.identity.clone(),
                    wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                    identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                        amount,
                        self.identity.wallet_index.unwrap_or(u32::MAX >> 1),
                        self.identity
                            .top_ups
                            .keys()
                            .max()
                            .cloned()
                            .map(|i| i + 1)
                            .unwrap_or_default(),
                    ),
                };

                self.set_step(WalletFundedScreenStep::WaitingForAssetLock);

                // Create the backend task to top_up the identity
                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                    identity_input,
                )))
            }
            _ => AppAction::None,
        }
    }

    fn top_up_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.current_funding_method();

        // Apply the max-amount restriction for wallet-balance funding and for a
        // received deposit (which also spends from the wallet balance).
        let (max_amount, show_max_button, fee_hint) = if matches!(
            funding_method,
            FundingMethod::UseWalletBalance | FundingMethod::ReceiveDeposit
        ) {
            let max_spendable_duffs = if funding_method == FundingMethod::ReceiveDeposit {
                self.funding_address_balance_duffs
            } else {
                self.wallet
                    .as_ref()
                    .and_then(|w| w.read().ok())
                    .map(|w| {
                        self.app_context
                            .snapshot_balance(&w.seed_hash())
                            .spendable()
                    })
                    .unwrap_or(0)
            };
            let fee_estimator = self.app_context.fee_estimator();
            let estimated_fee = fee_estimator.estimate_identity_topup();
            let max_with_fee_reserved =
                max_amount_after_fee_reserve(max_spendable_duffs, estimated_fee);
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

        // Lazy initialization of the AmountInput component
        let should_prefill = self.prefill_funding_amount;
        let amount_input = self.funding_amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::new_dash(0.0))
                .with_label("Amount:")
                .with_max_button(show_max_button)
                .with_max_amount(max_amount)
        });

        // Update max amount and button visibility in case funding method or wallet balance changed
        amount_input.set_max_amount(max_amount);
        amount_input.set_show_max_button(show_max_button);
        amount_input.set_max_exceeded_hint(fee_hint);

        // Pre-fill (once) with the fee-reserve-capped maximum when a deposit just
        // arrived, so the amount and Add funds button are populated but still editable.
        if should_prefill && let Some(max) = max_amount {
            amount_input.set_value(Amount::dash_from_credits(max));
        }

        let response = amount_input.show(ui);

        // Update the funding_amount_exact from the parsed amount
        if let Some(amount) = response.inner.parsed_amount {
            // Amount.value() returns credits, convert to duffs (divide by 1000)
            self.funding_amount_exact = Some(amount.value() / 1000);
            // Keep the string in sync for backward compatibility
            self.funding_amount = format!("{}", amount.value() as f64 / 100_000_000_000.0);
        } else {
            self.funding_amount_exact = None;
        }

        if should_prefill {
            self.prefill_funding_amount = false;
        }

        ui.add_space(10.0);
    }
}

impl ScreenLike for TopUpIdentityScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.set_step(step_after_task_failure(self.current_step()));
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        let selected_seed_hash = self
            .wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()));
        if self.funding_address_request_in_flight
            && context.generated_receive_address_wallet() == selected_seed_hash
        {
            self.funding_address_request_in_flight = false;
            self.funding_address_request_failed = true;
        }
    }
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::TrackedAssetLocks { seed_hash, locks } =
            backend_task_success_result
        {
            self.asset_lock_cache.store(seed_hash, locks);
            return;
        }

        if let BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address } =
            &backend_task_success_result
        {
            // Adopt the SPV-watched deposit address only for the selected wallet.
            let is_ours = self
                .wallet
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

        if self.current_step() == WalletFundedScreenStep::WaitingOnFunds
            && let BackendTaskSuccessResult::CoreItem(CoreItem::ReceivedAvailableUTXOTransaction(
                _,
                outputs,
            )) = &backend_task_success_result
        {
            let minimum_credits = self.app_context.fee_estimator().estimate_identity_topup();
            let (next, prefill) = deposit_event_outcome(
                WalletFundedScreenStep::WaitingOnFunds,
                self.funding_address.as_ref(),
                outputs,
                minimum_credits,
            );
            // Pre-fill the amount with the fee-reserve-capped balance when the
            // deposit lands, so the field and Add funds button populate.
            if prefill.is_some() {
                self.prefill_funding_amount = true;
            }
            self.set_step(next);
            return;
        }

        if let BackendTaskSuccessResult::ToppedUpIdentity(qualified_identity, fee_result) =
            backend_task_success_result
        {
            self.identity = qualified_identity;
            self.completed_fee_result = Some(fee_result);
            self.funding_address = None;
            self.funding_amount.clear();
            self.funding_amount_exact = None;
            self.funding_amount_input = None;
            self.copied_to_clipboard = None;

            self.set_step(WalletFundedScreenStep::Success);
            return;
        }

        if self.current_step() == WalletFundedScreenStep::WaitingForAssetLock
            && let BackendTaskSuccessResult::CoreItem(CoreItem::ReceivedAvailableUTXOTransaction(
                tx,
                _,
            )) = &backend_task_success_result
            && let Some(TransactionPayload::AssetLockPayloadType(asset_lock_payload)) =
                &tx.special_transaction_payload
            && asset_lock_payload.credit_outputs.iter().any(|tx_out| {
                let Ok(address) =
                    Address::from_script(&tx_out.script_pubkey, self.app_context.network)
                else {
                    return false;
                };
                match &self.wallet {
                    Some(wallet) => wallet
                        .read()
                        .is_ok_and(|w| w.known_addresses.contains_key(&address)),
                    None => false,
                }
            })
        {
            self.set_step(WalletFundedScreenStep::WaitingForPlatformAcceptance);
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
                ("Add Funds", AppAction::None),
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
                let step = self.current_step();
                if step == WalletFundedScreenStep::Success {
                    inner_action |= self.show_success(ui);
                    return;
                }

                ui.add_space(10.0);

                // Display identity info
                ui.horizontal(|ui| {
                    ui.label("Identity:");

                    // Show alias if available, otherwise show ID
                    if let Some(alias) = &self.identity.alias {
                        ui.label(alias);
                    } else {
                        ui.label(self.identity.identity.id().to_string(Encoding::Base58));
                    }
                });

                // Show current balance
                ui.horizontal(|ui| {
                    ui.label("Balance:");
                    let balance_dash = self.identity.identity.balance() as f64 * 1e-11;
                    ui.label(format!("{:.4} DASH", balance_dash));
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.heading("Follow these steps to add funds to your identity:");
                ui.add_space(15.0);

                let mut step_number = 1;
                ui.heading(format!("{}. Choose your funding method.", step_number).as_str());
                step_number += 1;
                ui.add_space(10.0);

                self.render_funding_method(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Extract the funding method from the RwLock to minimize borrow scope
                let funding_method = self.current_funding_method();
                if funding_method == FundingMethod::NoSelection {
                    return;
                }

                if funding_method == FundingMethod::UseWalletBalance
                    || funding_method == FundingMethod::UseUnusedAssetLock
                    || funding_method == FundingMethod::UsePlatformAddress
                    || funding_method == FundingMethod::ReceiveDeposit
                {
                    // Check if there's more than one wallet to show selection UI
                    let wallet_count = self
                        .app_context
                        .wallets
                        .read()
                        .map(|w| w.len())
                        .unwrap_or(0);

                    if wallet_count > 1 {
                        ui.horizontal(|ui| {
                            ui.heading(format!(
                                "{step_number}. Choose the wallet to use to add funds to this \
                                 identity."
                            ));
                            ui.add_space(10.0);

                            // Add info icon with hover tooltip and click popup
                            if crate::ui::helpers::info_icon_button(ui, WALLET_SELECTION_TOOLTIP)
                                .clicked()
                            {
                                self.show_pop_up_info = Some(WALLET_SELECTION_TOOLTIP.to_string());
                            }
                        });
                        step_number += 1;

                        ui.add_space(10.0);
                    }

                    self.render_wallet_selection(ui);

                    if self.wallet.is_none() {
                        return;
                    };

                    if let Some(wallet) = &self.wallet {
                        if !self.wallet_open_attempted {
                            if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                                MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                                    .disable_auto_dismiss();
                            }
                            self.wallet_open_attempted = true;
                        }
                        if wallet_needs_unlock(wallet) {
                            ui.add_space(10.0);
                            ui.colored_label(
                                egui::Color32::from_rgb(200, 150, 50),
                                "Wallet is locked. Please unlock to continue.",
                            );
                            ui.add_space(8.0);
                            if ui.button("Unlock Wallet").clicked() {
                                self.wallet_unlock_popup.open();
                            }
                            return;
                        }
                    }

                    if wallet_count > 1 {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                    }
                }

                match funding_method {
                    FundingMethod::NoSelection => (),
                    FundingMethod::UseUnusedAssetLock => {
                        inner_action |= self.render_ui_by_using_unused_asset_lock(ui, step_number);
                    }
                    FundingMethod::UseWalletBalance => {
                        inner_action |= self.render_ui_by_using_unused_balance(ui, step_number);
                    }
                    FundingMethod::UsePlatformAddress => {
                        inner_action |= self.render_ui_by_platform_address(ui, step_number);
                    }
                    FundingMethod::ReceiveDeposit => {
                        inner_action |= self.render_ui_by_receive_deposit(ui, step_number);
                    }
                }
            });

            inner_action
        });

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("identity_top_up_wallet_selection_info_popup"),
                        "Wallet Selection Info",
                        &show_pop_up_info_text,
                    );
                    if popup.show(ui).inner {
                        self.show_pop_up_info = None;
                    }
                });
        }

        // Fetch tracked asset locks once per wallet (off the UI thread). The
        // funding-method gate and wallet selector check every wallet, so all
        // are requested together as one concurrent batch.
        let seed_hashes: Vec<_> = self
            .app_context
            .wallets
            .read()
            .map(|wallets| {
                wallets
                    .values()
                    .filter_map(|w| w.read().ok().map(|g| g.seed_hash()))
                    .collect()
            })
            .unwrap_or_default();
        let lock_fetches = self.asset_lock_cache.ensure_requested_many(seed_hashes);

        // Derive the "Receive a new deposit" address off the UI thread; the QR
        // view queues this when it has no address yet.
        let funding_address_request =
            self.pending_funding_address_request
                .take()
                .map(|seed_hash| {
                    BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash })
                });
        if funding_address_request.is_some() {
            self.funding_address_request_in_flight = true;
        }
        action |= pending_backend_tasks_action(lock_fetches, funding_address_request);

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_frame_dispatch_keeps_lock_fetches_and_receive_address_request() {
        let lock_seed_a = [1u8; 32];
        let lock_seed_b = [2u8; 32];
        let receive_seed = [3u8; 32];
        let action = pending_backend_tasks_action(
            vec![
                BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks {
                    seed_hash: lock_seed_a,
                }),
                BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks {
                    seed_hash: lock_seed_b,
                }),
            ],
            Some(BackendTask::WalletTask(
                WalletTask::GenerateReceiveAddress {
                    seed_hash: receive_seed,
                },
            )),
        );

        let AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Concurrent) = action else {
            panic!("same-frame tasks must be dispatched as one concurrent batch");
        };
        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks { seed_hash })
                if *seed_hash == lock_seed_a
        )));
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks { seed_hash })
                if *seed_hash == lock_seed_b
        )));
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash })
                if *seed_hash == receive_seed
        )));
    }
}
