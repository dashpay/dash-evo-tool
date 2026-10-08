mod by_platform_address;
mod by_receive_deposit;
mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::asset_lock::{AssetLockAmountError, validate_asset_lock_amount};
use crate::model::fee_estimation::{format_credits_as_dash, format_duffs_as_dash};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{MessageBanner, OptionOverlayExt, OverlayConfig, OverlayHandle};
use crate::ui::identity::funding_common::{
    FundingMethod, WalletFundedScreenStep, default_funding_state, deposit_event_outcome,
    max_amount_after_fee_reserve, receive_deposit_ceiling_duffs, spendable_covers_minimum,
    step_after_task_failure, wallet_selection_combo,
};
use crate::ui::state::{AssetLockBalanceCache, TrackedAssetLockCache};
use crate::ui::{
    MessageType, ScreenLike, append_concurrent_backend_tasks, can_append_concurrent_backend_tasks,
};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits, Duffs};
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use egui::{ComboBox, ScrollArea, Ui};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const WALLET_SELECTION_TOOLTIP: &str =
    "Choose the wallet that will supply or receive the Dash used to add funds to this identity.";

/// Blocking-overlay text for a running top-up. The task reports no progress to
/// the screen, so one sentence covers the whole run and names no stage.
const TOP_UP_IN_PROGRESS: &str = "Adding funds to your identity.";
/// Shown in place of the funding form while a top-up runs.
const TOP_UP_FORM_PAUSED: &str = "You can add more funds when this transfer finishes.";
/// Progress banner kept up while a top-up runs in the background.
pub(crate) const TOP_UP_IN_BACKGROUND: &str =
    "Adding funds to your identity in the background. You can keep using Dash Evo Tool.";
/// Confirmation banner for a top-up that finished in the background.
pub(crate) const TOP_UP_DONE_IN_BACKGROUND: &str = "The funds were added to your identity.";
const TOP_UP_BACKGROUND_LABEL: &str = "Continue in background";
const TOP_UP_BACKGROUND_ACTION_ID: &str = "identity:top_up:background";
const TOP_UP_BACKGROUND_OWNERS_ID: &str = "__identity_top_up_background_owners";
/// How long the blocking overlay waits before it offers to continue in the
/// background. A top-up normally finishes well inside this window.
const TOP_UP_BACKGROUND_OFFER_AFTER: Duration = Duration::from_secs(30);

/// Dispatches of the top-ups that were sent to the background and still run.
fn background_top_ups(ctx: &egui::Context) -> Vec<BackendTaskContext> {
    ctx.data(|data| data.get_temp(egui::Id::new(TOP_UP_BACKGROUND_OWNERS_ID)))
        .unwrap_or_default()
}

/// Whether a top-up of `identity_id` was sent to the background and is still
/// running.
fn top_up_runs_in_background(ctx: &egui::Context, identity_id: &Identifier) -> bool {
    background_top_ups(ctx)
        .iter()
        .any(|dispatch| dispatch.identity_top_up_identity() == Some(*identity_id))
}

/// Raise the background-progress banner for the top-up sent as `dispatch`. It
/// outlives the screen: only that dispatch's own result clears it.
pub(crate) fn show_top_up_background_banner(ctx: &egui::Context, dispatch: BackendTaskContext) {
    let mut dispatches = background_top_ups(ctx);
    if !dispatches.contains(&dispatch) {
        dispatches.push(dispatch);
    }
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(TOP_UP_BACKGROUND_OWNERS_ID), dispatches));
    MessageBanner::set_global(ctx, TOP_UP_IN_BACKGROUND, MessageType::Info).disable_auto_dismiss();
}

/// Record that the top-up sent as `dispatch` ended, dropping the progress
/// banner with the last one. Returns whether that top-up ran in the
/// background; another transfer to the same identity is not that top-up.
pub(crate) fn clear_top_up_background_banner(
    ctx: &egui::Context,
    dispatch: &BackendTaskContext,
) -> bool {
    let mut dispatches = background_top_ups(ctx);
    let Some(position) = dispatches.iter().position(|pending| pending == dispatch) else {
        return false;
    };
    dispatches.remove(position);
    if dispatches.is_empty() {
        MessageBanner::clear_global_message(ctx, TOP_UP_IN_BACKGROUND);
    }
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(TOP_UP_BACKGROUND_OWNERS_ID), dispatches));
    true
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
    asset_lock_balance: AssetLockBalanceCache,
    /// The dispatch of the top-up this screen is waiting on.
    top_up_context: Option<BackendTaskContext>,
    /// Blocks the app while the top-up runs, until it ends or the user
    /// continues in the background.
    top_up_overlay: Option<OverlayHandle>,
    /// Whether the overlay already carries its background button.
    top_up_background_offered: bool,
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
            asset_lock_balance: AssetLockBalanceCache::default(),
            top_up_context: None,
            top_up_overlay: None,
            top_up_background_offered: false,
        }
    }

    /// Dispatch a top-up: move to `step`, block the app behind the progress
    /// overlay, and tag the task so only its own result releases the screen.
    fn begin_top_up(&mut self, task: IdentityTask, step: WalletFundedScreenStep) -> AppAction {
        self.set_step(step);
        self.top_up_overlay.raise(
            self.app_context.egui_ctx(),
            TOP_UP_IN_PROGRESS,
            OverlayConfig::default(),
        );
        self.top_up_background_offered = false;
        let task = BackendTask::IdentityTask(task);
        let context = BackendTaskContext::for_dispatch_on(&task, self.app_context.network());
        self.top_up_context = Some(context.clone());
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Stop waiting on the top-up: lower the overlay and forget its dispatch.
    fn release_top_up(&mut self) {
        self.top_up_overlay.take_and_clear();
        self.top_up_context = None;
    }

    /// Whether a top-up of this identity is running — one this screen
    /// dispatched, or an earlier one still finishing in the background.
    fn top_up_in_flight(&self) -> bool {
        matches!(
            self.current_step(),
            WalletFundedScreenStep::WaitingForAssetLock
                | WalletFundedScreenStep::WaitingForPlatformAcceptance
        ) || top_up_runs_in_background(self.app_context.egui_ctx(), &self.identity.identity.id())
    }

    /// Offer the background button once the top-up runs long, and act on a
    /// click of it.
    fn sync_top_up_overlay(&mut self) {
        let Some(handle) = self.top_up_overlay.clone() else {
            self.resume_after_background_top_up();
            return;
        };
        if handle
            .take_actions()
            .iter()
            .any(|action_id| action_id == TOP_UP_BACKGROUND_ACTION_ID)
        {
            self.continue_top_up_in_background();
            return;
        }
        if !self.top_up_background_offered
            && handle
                .elapsed()
                .is_some_and(|elapsed| elapsed >= TOP_UP_BACKGROUND_OFFER_AFTER)
        {
            handle.with_secondary_action(TOP_UP_BACKGROUND_LABEL, TOP_UP_BACKGROUND_ACTION_ID);
            // The wait has no upper bound, so keyboard users need this exit too.
            handle.with_keyboard_escape(TOP_UP_BACKGROUND_ACTION_ID);
            self.top_up_background_offered = true;
        }
    }

    /// Unblock the app while the top-up keeps running: the overlay gives way
    /// to a progress banner that follows the user to other screens.
    fn continue_top_up_in_background(&mut self) {
        self.top_up_overlay.take_and_clear();
        if let Some(dispatch) = self.top_up_context.clone() {
            show_top_up_background_banner(self.app_context.egui_ctx(), dispatch);
        }
    }

    /// Bring the form back once a top-up this screen sent to the background
    /// has ended: its result goes to whichever screen is visible at the time.
    fn resume_after_background_top_up(&mut self) {
        let Some(dispatch) = &self.top_up_context else {
            return;
        };
        if background_top_ups(self.app_context.egui_ctx()).contains(dispatch) {
            return;
        }
        self.release_top_up();
        // The top-up may have succeeded, so the form must not offer it again.
        self.forget_sent_funding();
        self.set_step(step_after_task_failure(self.current_step()));
        // A successful top-up changed the stored balance and top-up count.
        if let Ok(Some(identity)) = self
            .app_context
            .get_identity_by_id(&self.identity.identity.id())
        {
            self.identity = identity;
        }
    }

    /// Forget what the last top-up was built from, so that a form shown
    /// after it cannot offer the same transfer again.
    fn forget_sent_funding(&mut self) {
        self.funding_address = None;
        self.funding_amount.clear();
        self.funding_amount_exact = None;
        self.funding_amount_input = None;
        self.funding_asset_lock = None;
        self.platform_top_up_amount = None;
        self.platform_top_up_amount_input = None;
        self.copied_to_clipboard = None;
    }

    /// Replaces the funding form while a top-up runs, so nothing on it can be
    /// changed or sent twice and no balance check describes a wallet whose
    /// funds are already on their way.
    fn render_top_up_in_flight(&self, ui: &mut Ui) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Spinner::new());
            ui.add_space(10.0);
            ui.heading(TOP_UP_FORM_PAUSED);
        });
        ui.add_space(40.0);
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

    /// Whether the loaded builder ceiling covers the top-up minimum.
    /// An unloaded quote does not block the funding option.
    fn wallet_balance_can_afford_top_up(&self, seed_hash: &WalletSeedHash) -> bool {
        let minimum = self.app_context.fee_estimator().estimate_identity_topup();
        self.asset_lock_balance
            .get(seed_hash)
            .is_none_or(|ceiling| spendable_covers_minimum(ceiling, minimum))
    }

    /// Whether the builder ceiling for the wallet's current spendable inputs
    /// is still being checked (no quote yet, or the quote predates an input
    /// change and is being revalidated).
    fn asset_lock_quote_is_loading(&self, seed_hash: &WalletSeedHash) -> bool {
        let (_, input_state, _) = self.app_context.asset_lock_probe_snapshot(seed_hash);
        self.asset_lock_balance
            .get_current(seed_hash, &input_state)
            .is_none()
    }

    /// Builder ceiling for Max and dispatch validation — one accessor for
    /// both, valid only while the quote matches current wallet inputs, so Max
    /// can never offer an amount validation would refuse.
    fn current_validation_ceiling_duffs(&self, funding_method: FundingMethod) -> Option<u64> {
        let seed_hash = self
            .wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok())
            .map(|wallet| wallet.seed_hash())?;
        let (_, input_state, _) = self.app_context.asset_lock_probe_snapshot(&seed_hash);
        let wallet_ceiling_duffs = self
            .asset_lock_balance
            .get_current(&seed_hash, &input_state)?;

        match funding_method {
            FundingMethod::UseWalletBalance => Some(wallet_ceiling_duffs),
            FundingMethod::ReceiveDeposit => Some(receive_deposit_ceiling_duffs(
                wallet_ceiling_duffs,
                self.funding_address_balance_duffs,
            )),
            _ => None,
        }
    }

    /// Whether `wallet` remains eligible; an unloaded ceiling does not block it.
    /// A busy wallet lock reads as ineligible rather than panicking.
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
                self.wallet_balance_can_afford_top_up(&w.seed_hash())
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
                .wallet_context()
                .wallets()
                .values()
                .cloned()
                .collect();

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
                            .and_then(|w| {
                                self.app_context.wallet_context().hd_alias(&w.seed_hash())
                            })
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
                            .is_some_and(|w| self.wallet_balance_can_afford_top_up(&w.seed_hash()));
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
            self.asset_lock_balance.invalidate();
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

    /// Reset wallet- and network-bound state after changing contexts.
    pub(crate) fn reset_for_network_switch(&mut self) {
        self.release_top_up();
        self.wallet = None;
        self.funding_asset_lock = None;
        self.reset_to_choose_funding();
        self.wallet_unlock_popup = WalletUnlockPopup::new();
        self.wallet_open_attempted = false;
        self.copied_to_clipboard = None;
        self.show_pop_up_info = None;
        self.selected_platform_address = None;
        self.platform_top_up_amount = None;
        self.platform_top_up_amount_input = None;
        self.completed_fee_result = None;
        self.asset_lock_cache.invalidate();
        self.asset_lock_balance.invalidate();
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

            {
                let wallets = self.app_context.wallet_context().wallets();
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
                    if self.wallet_balance_can_afford_top_up(&seed_hash) {
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

                    self.begin_top_up(
                        IdentityTask::TopUpIdentity(identity_input),
                        WalletFundedScreenStep::WaitingForPlatformAcceptance,
                    )
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
                let Some(max_amount) = self.current_validation_ceiling_duffs(funding_method) else {
                    let Ok(wallet) = selected_wallet.read() else {
                        return AppAction::None;
                    };
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        self.asset_lock_balance
                            .validation_unavailable_message(&wallet.seed_hash()),
                        MessageType::Warning,
                    );
                    return AppAction::None;
                };
                let identity_fee_duffs = self
                    .app_context
                    .fee_estimator()
                    .estimate_identity_topup()
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

                self.begin_top_up(
                    IdentityTask::TopUpIdentity(identity_input),
                    WalletFundedScreenStep::WaitingForAssetLock,
                )
            }
            _ => AppAction::None,
        }
    }

    fn top_up_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.current_funding_method();
        let available_ceiling_duffs = self.current_validation_ceiling_duffs(funding_method);

        let (max_amount, show_max_button, fee_hint) =
            if let Some(available_ceiling_duffs) = available_ceiling_duffs {
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_identity_topup();
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
    fn refresh_on_arrival(&mut self) {
        self.asset_lock_balance.invalidate();
    }

    fn refresh(&mut self) {
        self.asset_lock_balance.invalidate();
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        // Only the top-up's own failure releases the screen; an unrelated
        // error must leave a running top-up blocked.
        if self.top_up_context.as_ref() == Some(context) {
            self.release_top_up();
            self.set_step(step_after_task_failure(self.current_step()));
        }
        if let Some((seed_hash, snapshot_generation, request_id)) =
            context.asset_lock_max_amount_request()
        {
            self.asset_lock_balance.mark_loading_failed(
                &seed_hash,
                snapshot_generation,
                request_id,
            );
        }
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

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.asset_lock_max_amount_request().is_some()
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        backend_task_success_result: BackendTaskSuccessResult,
    ) {
        let BackendTaskSuccessResult::ToppedUpIdentity(qualified_identity, fee_result) =
            backend_task_success_result
        else {
            self.display_task_result(backend_task_success_result);
            return;
        };
        if qualified_identity.identity.id() != self.identity.identity.id() {
            return;
        }
        self.identity = qualified_identity;
        // Another transfer to this identity, such as one from Wallet Send, is
        // not the top-up this screen waits on.
        if self.top_up_context.as_ref() != Some(context) {
            return;
        }
        self.release_top_up();
        self.completed_fee_result = Some(fee_result);
        self.forget_sent_funding();
        self.set_step(WalletFundedScreenStep::Success);
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::AssetLockMaxAmount {
            seed_hash,
            snapshot_generation,
            request_id,
            amount_duffs,
            observed_inputs,
            is_partial,
        } = &backend_task_success_result
        {
            self.asset_lock_balance.store(
                *seed_hash,
                *snapshot_generation,
                *request_id,
                *amount_duffs,
                observed_inputs.clone(),
                *is_partial,
            );
            return;
        }
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
        self.sync_top_up_overlay();
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Identities", AppAction::OpenIdentityPicker),
                ("Add Funds", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentityHub,
        );

        let mut request_asset_lock_balance = false;
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

                    ui.label(self.app_context.identity_display_label(&self.identity));
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

                if self.top_up_in_flight() {
                    self.render_top_up_in_flight(ui);
                    return;
                }

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
                    let wallet_count = self.app_context.wallet_context().hd_count();

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
                        request_asset_lock_balance = true;
                        inner_action |= self.render_ui_by_using_unused_balance(ui, step_number);
                    }
                    FundingMethod::UsePlatformAddress => {
                        inner_action |= self.render_ui_by_platform_address(ui, step_number);
                    }
                    FundingMethod::ReceiveDeposit => {
                        request_asset_lock_balance = true;
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

        if can_append_concurrent_backend_tasks(&action) {
            // Fetch tracked asset locks once per wallet (off the UI thread). The
            // funding-method gate and wallet selector check every wallet, so all
            // are requested together as one concurrent batch.
            let seed_hashes: Vec<_> = self
                .app_context
                .wallet_context()
                .wallets()
                .values()
                .filter_map(|w| w.read().ok().map(|g| g.seed_hash()))
                .collect();
            let mut pending_tasks = self.asset_lock_cache.ensure_requested_many(seed_hashes);

            if request_asset_lock_balance
                && let Some(seed_hash) = self
                    .wallet
                    .as_ref()
                    .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()))
            {
                let (snapshot_generation, input_state, utxo_revision) =
                    self.app_context.asset_lock_probe_snapshot(&seed_hash);
                if let Some(task) = self.asset_lock_balance.ensure_requested(
                    seed_hash,
                    snapshot_generation,
                    input_state,
                    utxo_revision,
                ) {
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

            action = append_concurrent_backend_tasks(action, pending_tasks);
        }

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::BackendTasksExecutionMode;
    use crate::context::test_support::{test_app_context, test_app_context_for_network};
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use crate::ui::Screen;
    use crate::ui::components::ProgressOverlay;
    use crate::wallet_backend::AssetLockInputState;
    use dash_sdk::dpp::dashcore::{Network, OutPoint, Txid, hashes::Hash};
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;

    fn different_asset_lock_inputs(seed_byte: u8) -> AssetLockInputState {
        AssetLockInputState::from_inputs([(
            OutPoint::new(Txid::from_byte_array([seed_byte; 32]), 0),
            1,
        )])
    }

    fn wallet_balance_screen(
        seed_byte: u8,
    ) -> (TopUpIdentityScreen, WalletSeedHash, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let wallet = Arc::new(RwLock::new(
            Wallet::new_from_seed([seed_byte; 64], Network::Testnet, None, None).expect("wallet"),
        ));
        let seed_hash = wallet.read().expect("wallet lock").seed_hash();
        let mut screen = TopUpIdentityScreen::new(test_identity(Network::Testnet), &app_context);
        screen.wallet = Some(wallet);
        screen.funding_amount_exact = Some(1);
        (screen, seed_hash, temp_dir)
    }

    fn asset_lock_request_id(task: Option<BackendTask>) -> u64 {
        match task {
            Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                request_id, ..
            })) => request_id,
            other => panic!("expected asset-lock maximum request, got {other:?}"),
        }
    }

    fn test_identity(network: Network) -> QualifiedIdentity {
        QualifiedIdentity {
            identity: Identity::new_with_id_and_keys(
                Identifier::random(),
                BTreeMap::new(),
                PlatformVersion::latest(),
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: Vec::new(),
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: Some(0),
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network,
        }
    }

    #[test]
    fn same_frame_dispatch_keeps_probe_and_other_backend_tasks() {
        let lock_seed_a = [1u8; 32];
        let probe_seed = [2u8; 32];
        let receive_seed = [3u8; 32];
        let action = append_concurrent_backend_tasks(
            AppAction::BackendTask(BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks {
                seed_hash: lock_seed_a,
            })),
            vec![
                BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                    seed_hash: probe_seed,
                    snapshot_generation: 9,
                    request_id: 17,
                }),
                BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
                    seed_hash: receive_seed,
                }),
            ],
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
            BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                seed_hash,
                snapshot_generation: 9,
                request_id: 17,
            }) if *seed_hash == probe_seed
        )));
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash })
                if *seed_hash == receive_seed
        )));
    }

    #[test]
    fn receive_deposit_dispatch_rejects_amount_above_deposit_address_balance() {
        const DEPOSIT_ADDRESS_DUFFS: u64 = 10_000_000;
        const REQUESTED_DUFFS: u64 = 20_000_000;
        const WALLET_CEILING_DUFFS: u64 = 100_000_000;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let wallet = Arc::new(RwLock::new(
            Wallet::new_from_seed([0x32; 64], Network::Testnet, None, None).expect("wallet"),
        ));
        let seed_hash = wallet.read().expect("wallet lock").seed_hash();
        let mut screen = TopUpIdentityScreen::new(test_identity(Network::Testnet), &app_context);
        screen.wallet = Some(wallet);
        screen.funding_address_balance_duffs = DEPOSIT_ADDRESS_DUFFS;
        screen.funding_amount_exact = Some(REQUESTED_DUFFS);
        let (generation, final_funds, revision) = app_context.asset_lock_probe_snapshot(&seed_hash);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            generation,
            final_funds.clone(),
            revision,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            generation,
            request_id,
            WALLET_CEILING_DUFFS,
            final_funds,
            false,
        );

        assert!(matches!(
            screen.top_up_identity_clicked(FundingMethod::ReceiveDeposit),
            AppAction::None
        ));
    }

    #[test]
    fn top_up_dispatch_rejects_quote_for_stale_utxo_composition() {
        let (mut screen, seed_hash, _temp_dir) = wallet_balance_screen(0x37);
        let (_, current_final_funds, current_revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let stale_inputs = different_asset_lock_inputs(0x37);

        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            7,
            current_final_funds,
            current_revision,
        ));
        screen
            .asset_lock_balance
            .store(seed_hash, 7, request_id, 10_000_000, stale_inputs, false);

        assert!(matches!(
            screen.top_up_identity_clicked(FundingMethod::UseWalletBalance),
            AppAction::None
        ));
        let ctx = screen.app_context.egui_ctx();
        assert!(MessageBanner::has_global(ctx));
        MessageBanner::clear_global_message(
            ctx,
            "Your wallet's available amount is still being checked. Wait a moment and try again.",
        );
        assert!(
            !MessageBanner::has_global(ctx),
            "stale composition must surface the loading warning rather than dispatch"
        );
    }

    #[test]
    fn top_up_dispatch_distinguishes_failed_probe_from_loading() {
        let (mut screen, seed_hash, _temp_dir) = wallet_balance_screen(0x38);
        let ctx = screen.app_context.egui_ctx().clone();
        let (generation, final_funds, revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            generation,
            final_funds,
            revision,
        ));

        assert!(matches!(
            screen.top_up_identity_clicked(FundingMethod::UseWalletBalance),
            AppAction::None
        ));
        assert!(MessageBanner::has_global(&ctx));
        MessageBanner::clear_global_message(
            &ctx,
            "Your wallet's available amount is still being checked. Wait a moment and try again.",
        );
        assert!(
            !MessageBanner::has_global(&ctx),
            "loading dispatch must use the loading-specific warning"
        );

        screen
            .asset_lock_balance
            .mark_loading_failed(&seed_hash, generation, request_id);
        assert!(screen.asset_lock_balance.is_failed(&seed_hash));

        assert!(matches!(
            screen.top_up_identity_clicked(FundingMethod::UseWalletBalance),
            AppAction::None
        ));
        assert!(MessageBanner::has_global(&ctx));
        MessageBanner::clear_global_message(
            &ctx,
            "The available amount could not be checked. Use Retry and try again.",
        );
        assert!(
            !MessageBanner::has_global(&ctx),
            "failed dispatch must use the failed-specific retry warning"
        );
    }

    const NOT_ENOUGH_DASH: &str = "does not have enough Dash";

    /// Store a builder ceiling that matches the wallet's current inputs.
    fn store_current_quote(
        screen: &mut TopUpIdentityScreen,
        seed_hash: WalletSeedHash,
        amount_duffs: u64,
    ) {
        let (generation, inputs, revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            generation,
            inputs.clone(),
            revision,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            generation,
            request_id,
            amount_duffs,
            inputs,
            false,
        );
    }

    /// Render the whole screen once and report whether a label containing
    /// `text` is on it.
    fn screen_shows(screen: &mut TopUpIdentityScreen, text: &str) -> bool {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui(|ui| {
                screen.ui(ui);
            });
        // A fixed step count: the in-progress spinner never stops repainting.
        harness.run_steps(2);
        harness.query_by_label_contains(text).is_some()
    }

    /// A wallet-balance screen whose remaining balance cannot cover another
    /// top-up — the state the wallet is in right after its funds were sent.
    fn drained_wallet_balance_screen(
        seed_byte: u8,
    ) -> (TopUpIdentityScreen, WalletSeedHash, tempfile::TempDir) {
        let (mut screen, seed_hash, temp_dir) = wallet_balance_screen(seed_byte);
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::UseWalletBalance;
        store_current_quote(&mut screen, seed_hash, 1);
        (screen, seed_hash, temp_dir)
    }

    /// Sending the funds drains the wallet, so the balance check under the
    /// running top-up reads as "not enough Dash". That warning describes the
    /// next top-up, not the one in progress, and must not be shown.
    #[test]
    fn in_flight_top_up_never_reports_missing_funds() {
        let (mut screen, _seed_hash, _temp_dir) = drained_wallet_balance_screen(0x41);

        screen.set_step(WalletFundedScreenStep::ReadyToCreate);
        assert!(
            screen_shows(&mut screen, NOT_ENOUGH_DASH),
            "an idle screen with a drained wallet reports the missing funds"
        );

        screen.set_step(WalletFundedScreenStep::WaitingForAssetLock);
        assert!(
            !screen_shows(&mut screen, NOT_ENOUGH_DASH),
            "a running top-up must not report missing funds"
        );
        assert!(screen_shows(&mut screen, TOP_UP_FORM_PAUSED));
        assert!(
            !screen_shows(&mut screen, "Add funds"),
            "a running top-up must not offer to send the funds again"
        );
    }

    const ADD_FUNDS_BUTTON: &str = "Add funds";

    /// A wallet-balance screen on its form with an amount entered, so the
    /// form offers to send it.
    fn funded_form_screen(seed_byte: u8) -> (TopUpIdentityScreen, tempfile::TempDir) {
        let (mut screen, seed_hash, temp_dir) = wallet_balance_screen(seed_byte);
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::UseWalletBalance;
        store_current_quote(&mut screen, seed_hash, 10_000_000);
        screen.set_step(WalletFundedScreenStep::ReadyToCreate);
        // The amount field fills itself with the largest amount it can send.
        screen.prefill_funding_amount = true;
        (screen, temp_dir)
    }

    /// A wallet-balance screen with a dispatched top-up, plus its dispatch.
    fn dispatched_top_up_screen(
        seed_byte: u8,
    ) -> (TopUpIdentityScreen, BackendTaskContext, tempfile::TempDir) {
        let (mut screen, seed_hash, temp_dir) = wallet_balance_screen(seed_byte);
        store_current_quote(&mut screen, seed_hash, 10_000_000);
        let action = screen.top_up_identity_clicked(FundingMethod::UseWalletBalance);
        let AppAction::BackendTaskWithContext { context, .. } = action else {
            panic!("expected an attributed top-up dispatch, got {action:?}");
        };
        (screen, context, temp_dir)
    }

    #[test]
    fn top_up_dispatch_blocks_the_app_until_its_own_failure() {
        let (mut screen, context, _temp_dir) = dispatched_top_up_screen(0x42);
        let ctx = screen.app_context.egui_ctx().clone();
        assert_eq!(
            context.identity_top_up_identity(),
            Some(screen.identity.identity.id())
        );
        assert!(ProgressOverlay::has_global(&ctx));
        assert!(screen.top_up_in_flight());

        // An unrelated failure reaches the visible screen too.
        screen
            .display_backend_task_error(&BackendTaskContext::Other, &TaskError::NoIdentitiesFound);
        screen.display_message("Background refresh failed.", MessageType::Error);
        assert!(
            ProgressOverlay::has_global(&ctx),
            "an unrelated error must not unblock a running top-up"
        );
        assert!(screen.top_up_in_flight());

        screen.display_backend_task_error(&context, &TaskError::NoIdentitiesFound);
        assert!(!ProgressOverlay::has_global(&ctx));
        assert_eq!(screen.current_step(), WalletFundedScreenStep::ReadyToCreate);
    }

    /// Wallet Send can add funds to the same identity without blocking the
    /// app, so its success may arrive while this screen waits on its own.
    #[test]
    fn top_up_success_releases_only_the_screen_that_dispatched_it() {
        let (mut screen, context, _temp_dir) = dispatched_top_up_screen(0x43);
        let ctx = screen.app_context.egui_ctx().clone();
        let other_transfer = BackendTaskContext::IdentityTopUp(screen.identity.identity.id());
        let topped_up = |identity: QualifiedIdentity| {
            BackendTaskSuccessResult::ToppedUpIdentity(identity, FeeResult::new(1, 1))
        };

        screen.display_backend_task_result(&context, topped_up(test_identity(Network::Testnet)));
        assert!(
            ProgressOverlay::has_global(&ctx),
            "another identity's top-up must not release this screen"
        );

        let mut refreshed = screen.identity.clone();
        refreshed.alias = Some("refreshed".to_owned());
        screen.display_backend_task_result(&other_transfer, topped_up(refreshed));
        assert!(
            ProgressOverlay::has_global(&ctx),
            "another transfer to this identity must not release a running top-up"
        );
        assert!(screen.top_up_in_flight());
        assert_eq!(
            screen.identity.alias.as_deref(),
            Some("refreshed"),
            "another transfer's result still carries the current identity"
        );

        screen.display_backend_task_result(&context, topped_up(screen.identity.clone()));
        assert!(!ProgressOverlay::has_global(&ctx));
        assert_eq!(screen.current_step(), WalletFundedScreenStep::Success);
    }

    /// Everything the global overlay paints for one frame of `ctx`.
    fn overlay_text(ctx: &egui::Context) -> String {
        fn collect(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(shape) => {
                    text.push_str(shape.galley.text());
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
                _ => {}
            }
        }
        let mut text = String::new();
        // Two frames: the overlay card only measures itself on its first one.
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ProgressOverlay::render_global(ui.ctx(), false);
            });
            text.clear();
            for clipped in std::mem::take(&mut output.shapes) {
                collect(&clipped.shape, &mut text);
            }
            output.drop_without_applying_deltas();
        }
        text
    }

    /// The task reports no progress to the screen, so the dialog keeps one
    /// sentence for the whole run instead of naming a stage it cannot know,
    /// and the status under it does not repeat the dialog.
    #[test]
    fn running_top_up_keeps_one_dialog_message_and_a_different_status() {
        let (mut screen, _context, _temp_dir) = dispatched_top_up_screen(0x46);
        let ctx = screen.app_context.egui_ctx().clone();

        for step in [
            WalletFundedScreenStep::WaitingForAssetLock,
            WalletFundedScreenStep::WaitingForPlatformAcceptance,
        ] {
            screen.set_step(step);
            assert!(screen_shows(&mut screen, TOP_UP_FORM_PAUSED));
            assert!(
                !screen_shows(&mut screen, TOP_UP_IN_PROGRESS),
                "the status under the dialog must not repeat it"
            );
            let dialog = overlay_text(&ctx);
            assert!(
                dialog.contains(TOP_UP_IN_PROGRESS),
                "the dialog must keep its message at every stage, got {dialog:?}"
            );
        }
    }

    #[cfg(feature = "testing")]
    #[test]
    fn long_running_top_up_offers_to_continue_in_background() {
        let (mut screen, _context, _temp_dir) = dispatched_top_up_screen(0x44);

        screen.sync_top_up_overlay();
        assert!(
            !screen.top_up_background_offered,
            "a top-up that just started must stay a hard block"
        );

        screen
            .top_up_overlay
            .as_ref()
            .expect("overlay raised")
            .backdate(TOP_UP_BACKGROUND_OFFER_AFTER);
        screen.sync_top_up_overlay();
        assert!(screen.top_up_background_offered);
    }

    #[test]
    fn top_up_continued_in_background_unblocks_the_app_but_not_the_form() {
        let (mut screen, context, _temp_dir) = dispatched_top_up_screen(0x45);
        let ctx = screen.app_context.egui_ctx().clone();

        screen.continue_top_up_in_background();
        assert!(!ProgressOverlay::has_global(&ctx));
        assert!(MessageBanner::has_global(&ctx));
        assert!(screen.top_up_in_flight());

        // Reopening the screen for the same identity must not offer the form
        // while its earlier top-up still runs.
        let mut reopened = TopUpIdentityScreen::new(screen.identity.clone(), &screen.app_context);
        assert!(reopened.top_up_in_flight());
        assert!(screen_shows(&mut reopened, TOP_UP_FORM_PAUSED));
        assert!(
            !screen_shows(&mut reopened, TOP_UP_IN_BACKGROUND),
            "the status must not repeat the banner above it"
        );
        let other = TopUpIdentityScreen::new(test_identity(Network::Testnet), &screen.app_context);
        assert!(!other.top_up_in_flight());

        // Another transfer to the same identity is not this top-up.
        let other_transfer =
            BackendTaskContext::IdentityTopUp(context.identity_top_up_identity().unwrap());
        assert!(!clear_top_up_background_banner(&ctx, &other_transfer));
        assert!(reopened.top_up_in_flight());

        assert!(clear_top_up_background_banner(&ctx, &context));
        assert!(!MessageBanner::has_global(&ctx));
        assert!(!reopened.top_up_in_flight());
        assert!(
            !clear_top_up_background_banner(&ctx, &context),
            "a top-up that never ran in the background has no banner to end"
        );
    }

    /// The result of a background top-up reaches whichever screen is visible,
    /// so the screen that started it has to notice the end by itself. It is
    /// not told how the top-up ended, so the form it brings back must not
    /// still hold the transfer that was just sent.
    #[test]
    fn form_comes_back_clean_when_a_background_top_up_ends_out_of_sight() {
        let (mut screen, _temp_dir) = funded_form_screen(0x47);
        let ctx = screen.app_context.egui_ctx().clone();
        assert!(screen_shows(&mut screen, ADD_FUNDS_BUTTON));
        // Choices left over from the other funding methods.
        screen.funding_asset_lock = Some(OutPoint::null());
        screen.platform_top_up_amount = Some(Amount::new_dash(1.0));
        let action = screen.top_up_identity_clicked(FundingMethod::UseWalletBalance);
        let AppAction::BackendTaskWithContext { context, .. } = action else {
            panic!("expected a dispatched top-up, got {action:?}");
        };

        screen.continue_top_up_in_background();
        assert!(screen_shows(&mut screen, TOP_UP_FORM_PAUSED));

        assert!(clear_top_up_background_banner(&ctx, &context));
        assert!(
            !screen_shows(&mut screen, TOP_UP_FORM_PAUSED),
            "the form must come back once the background top-up ended"
        );
        assert!(!screen.top_up_in_flight());
        assert_eq!(screen.current_step(), WalletFundedScreenStep::ReadyToCreate);
        assert!(
            !screen_shows(&mut screen, ADD_FUNDS_BUTTON),
            "the form must not offer the amount that was just sent"
        );
        assert_eq!(screen.funding_amount_exact, None);
        assert_eq!(screen.funding_asset_lock, None);
        assert!(screen.platform_top_up_amount.is_none());
    }

    #[test]
    fn network_switch_and_refresh_invalidate_asset_lock_balance() {
        let old_dir = tempfile::tempdir().expect("old context dir");
        let new_dir = tempfile::tempdir().expect("new context dir");
        let old_context = test_app_context(old_dir.path());
        let new_context = test_app_context_for_network(new_dir.path(), Network::Mainnet);
        let wallet = Arc::new(RwLock::new(
            Wallet::new_from_seed([0x34; 64], Network::Testnet, None, None).expect("wallet"),
        ));
        let seed_hash = wallet.read().expect("wallet lock").seed_hash();
        let mut screen = TopUpIdentityScreen::new(test_identity(Network::Testnet), &old_context);
        screen.wallet = Some(wallet);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            7,
            AssetLockInputState::default(),
            1,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            7,
            request_id,
            900,
            AssetLockInputState::default(),
            false,
        );

        let mut screen = Screen::TopUpIdentityScreen(screen);
        screen.change_context(new_context.clone());
        let Screen::TopUpIdentityScreen(mut screen) = screen else {
            panic!("screen variant changed");
        };
        assert!(Arc::ptr_eq(&screen.app_context, &new_context));
        assert_eq!(screen.app_context.network(), Network::Mainnet);
        assert!(screen.wallet.is_none());
        assert_eq!(screen.asset_lock_balance.get(&seed_hash), None);

        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            8,
            AssetLockInputState::default(),
            1,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            8,
            request_id,
            800,
            AssetLockInputState::default(),
            false,
        );
        screen.refresh_on_arrival();
        assert_eq!(screen.asset_lock_balance.get(&seed_hash), None);

        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            9,
            AssetLockInputState::default(),
            1,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            9,
            request_id,
            700,
            AssetLockInputState::default(),
            false,
        );
        screen.refresh();
        assert_eq!(screen.asset_lock_balance.get(&seed_hash), None);
    }
}
