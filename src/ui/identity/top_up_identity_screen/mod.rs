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
use crate::model::asset_lock::{
    AssetLockAmountError, asset_lock_user_amount_range, validate_asset_lock_amount,
    validate_asset_lock_minimum,
};
use crate::model::fee_estimation::{
    format_credits_as_dash, format_duffs_as_dash, identity_topup_min_funding_duffs,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::balance_summary::{CoreFigure, WalletChoice};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_selector::WalletSelector;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{
    BannerHandle, MessageBanner, OptionBannerExt, OptionOverlayExt, OverlayConfig, OverlayHandle,
};
use crate::ui::identity::funding_common::{
    FundingMethod, WalletFundedScreenStep, default_funding_state, deposit_event_outcome,
    max_amount_after_fee_reserve, receive_deposit_ceiling_duffs, step_after_task_failure,
};
use crate::ui::state::{AssetLockBalanceCache, TrackedAssetLockCache};
use crate::ui::theme::DashColors;
use crate::ui::{
    MessageType, ScreenLike, append_concurrent_backend_tasks, can_append_concurrent_backend_tasks,
};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits, Duffs};
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{ComboBox, ScrollArea, Ui};
use std::ops::RangeInclusive;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const WALLET_SELECTION_TOOLTIP: &str =
    "Choose the wallet that will supply or receive the Dash used to add funds to this identity.";
/// Shown on a greyed-out wallet whose usable Dash cannot cover a top-up.
const WALLET_LACKS_DASH: &str = "This wallet does not have enough Dash to add funds to this \
     identity. Choose another wallet or add Dash to this one.";
/// Shown on a greyed-out wallet that holds no reusable funding transaction.
const WALLET_HAS_NO_FUNDING: &str = "This wallet has no existing funding transaction to use. \
     Choose another wallet or another funding method.";
/// Shown on a greyed-out wallet whose state cannot be read right now.
const WALLET_BUSY: &str = "Wallet is busy. Try again in a moment.";

/// Blocking-overlay text for a running top-up. The task reports no progress to
/// the screen, so one sentence covers the whole run and names no stage.
const TOP_UP_IN_PROGRESS: &str = "Adding funds to your identity.";
/// Shown in place of the funding form while a top-up runs.
const TOP_UP_FORM_PAUSED: &str = "You can add more funds when this transfer finishes.";
/// Shown instead of an amount when nothing the wallet can send covers the fee.
const TOP_UP_FEE_NOT_COVERED: &str = "The amount you can use is too small to cover the network fee. Add more Dash to your wallet and try again.";
/// Progress banner kept up while a top-up runs in the background.
const TOP_UP_IN_BACKGROUND: &str =
    "Adding funds to your identity in the background. You can keep using Dash Evo Tool.";
const TOP_UP_BACKGROUND_LABEL: &str = "Continue in background";
const TOP_UP_BACKGROUND_ACTION_ID: &str = "identity:top_up:background";
const TOP_UPS_IN_FLIGHT_ID: &str = "__identity_top_ups_in_flight";
const BACKGROUND_TOP_UP_BANNER_ID: &str = "__identity_background_top_up_banner";
/// How long the blocking overlay waits before it offers to continue in the
/// background. A top-up normally finishes well inside this window.
const TOP_UP_BACKGROUND_OFFER_AFTER: Duration = Duration::from_secs(30);

/// Confirmation banner for a top-up that ended out of the user's sight. Two
/// identities can share a name, and a banner already on screen is not raised
/// again for the same text, so the ID keeps their confirmations apart.
fn top_up_done_message(name: Option<&str>, id: &Identifier) -> String {
    let id = id.to_string(Encoding::Base58);
    match name {
        Some(name) => format!("The funds were added to the identity {name} (ID: {id})."),
        None => format!("The funds were added to the identity {id}."),
    }
}

/// A top-up sent from Add Funds whose result has not arrived yet.
#[derive(Clone)]
struct TopUpInFlight {
    dispatch: BackendTaskContext,
    /// Shown when the top-up succeeds out of the user's sight.
    confirmation: String,
    /// Followed by the progress banner rather than by the blocking overlay.
    in_background: bool,
}

/// The top-ups in flight. They are kept in egui temp data rather than in a
/// screen, so a top-up stays recorded when its screen is closed or the
/// network is switched.
fn top_ups_in_flight(ctx: &egui::Context) -> Vec<TopUpInFlight> {
    ctx.data(|data| data.get_temp(egui::Id::new(TOP_UPS_IN_FLIGHT_ID)))
        .unwrap_or_default()
}

fn store_top_ups_in_flight(ctx: &egui::Context, top_ups: Vec<TopUpInFlight>) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(TOP_UPS_IN_FLIGHT_ID), top_ups));
}

/// Whether a top-up of `identity_id` is in flight.
fn top_up_in_flight_for(ctx: &egui::Context, identity_id: &Identifier) -> bool {
    top_ups_in_flight(ctx)
        .iter()
        .any(|top_up| top_up.dispatch.identity_top_up_identity() == Some(*identity_id))
}

/// Record the top-up sent as `dispatch`. It stays in flight until
/// [`finish_top_up`] sees that dispatch's own result.
fn track_top_up(ctx: &egui::Context, dispatch: BackendTaskContext, confirmation: String) {
    let mut top_ups = top_ups_in_flight(ctx);
    top_ups.push(TopUpInFlight {
        dispatch,
        confirmation,
        in_background: false,
    });
    store_top_ups_in_flight(ctx, top_ups);
}

/// Raise the background-progress banner and keep its handle, the only witness
/// of the banner cap dropping it later.
fn raise_top_up_background_banner(ctx: &egui::Context) {
    let banner = MessageBanner::set_global(ctx, TOP_UP_IN_BACKGROUND, MessageType::Info);
    banner.disable_auto_dismiss();
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(BACKGROUND_TOP_UP_BANNER_ID), banner));
}

/// Bring the background-progress banner back when the banner cap dropped it
/// while a background top-up still runs. A banner the user closed stays
/// closed. Runs every frame, as no screen may be left to do it.
pub(crate) fn restore_top_up_background_banner(ctx: &egui::Context) {
    let banner: Option<BannerHandle> =
        ctx.data(|data| data.get_temp(egui::Id::new(BACKGROUND_TOP_UP_BANNER_ID)));
    if banner.was_evicted()
        && top_ups_in_flight(ctx)
            .iter()
            .any(|top_up| top_up.in_background)
    {
        raise_top_up_background_banner(ctx);
    }
}

/// Follow the top-up sent as `dispatch` with the progress banner instead of
/// the blocking overlay. The banner outlives the screen.
fn send_top_up_to_background(ctx: &egui::Context, dispatch: &BackendTaskContext) {
    let mut top_ups = top_ups_in_flight(ctx);
    let Some(top_up) = top_ups
        .iter_mut()
        .find(|top_up| top_up.dispatch == *dispatch)
    else {
        return;
    };
    top_up.in_background = true;
    store_top_ups_in_flight(ctx, top_ups);
    raise_top_up_background_banner(ctx);
}

/// A network switch drops every overlay, banner and stacked screen while the
/// top-ups sent before it keep running. Follow those with the progress banner.
pub(crate) fn follow_top_ups_after_network_switch(ctx: &egui::Context) {
    let mut top_ups = top_ups_in_flight(ctx);
    if top_ups.is_empty() {
        return;
    }
    for top_up in &mut top_ups {
        top_up.in_background = true;
    }
    store_top_ups_in_flight(ctx, top_ups);
    raise_top_up_background_banner(ctx);
}

/// End the top-up sent as `dispatch` on its own result, and confirm a success
/// the user did not watch: the screen that sent it may be gone. Returns
/// whether `dispatch` is a top-up sent from Add Funds; another transfer to the
/// same identity is not.
pub(crate) fn finish_top_up(
    ctx: &egui::Context,
    dispatch: &BackendTaskContext,
    succeeded: bool,
) -> bool {
    let mut top_ups = top_ups_in_flight(ctx);
    let Some(position) = top_ups
        .iter()
        .position(|top_up| top_up.dispatch == *dispatch)
    else {
        return false;
    };
    let ended = top_ups.remove(position);
    if !top_ups.iter().any(|top_up| top_up.in_background) {
        MessageBanner::clear_global_message(ctx, TOP_UP_IN_BACKGROUND);
        // The handle holds the egui context, which must not stay stored in itself.
        ctx.data_mut(|data| {
            data.remove::<BannerHandle>(egui::Id::new(BACKGROUND_TOP_UP_BANNER_ID))
        });
    }
    store_top_ups_in_flight(ctx, top_ups);
    if succeeded && ended.in_background {
        MessageBanner::set_global(ctx, ended.confirmation, MessageType::Success);
    }
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
    wallet_selector: Option<WalletSelector>,
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
            wallet_selector: None,
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
        track_top_up(
            self.app_context.egui_ctx(),
            context.clone(),
            top_up_done_message(
                self.app_context.identity_name(&self.identity).as_deref(),
                &self.identity.identity.id(),
            ),
        );
        self.top_up_context = Some(context.clone());
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Whether `dispatch` is the top-up this screen sent and still waits on.
    pub(crate) fn awaits_top_up(&self, dispatch: &BackendTaskContext) -> bool {
        self.top_up_context.as_ref() == Some(dispatch)
    }

    /// Stop waiting on the top-up: lower the overlay and forget its dispatch.
    fn release_top_up(&mut self) {
        self.top_up_overlay.take_and_clear();
        self.top_up_context = None;
    }

    /// Whether a top-up of this identity is running — one this screen
    /// dispatched, or one another Add Funds screen sent.
    fn top_up_in_flight(&self) -> bool {
        matches!(
            self.current_step(),
            WalletFundedScreenStep::WaitingForAssetLock
                | WalletFundedScreenStep::WaitingForPlatformAcceptance
        ) || top_up_in_flight_for(self.app_context.egui_ctx(), &self.identity.identity.id())
    }

    /// Notice the end of the top-up, offer the background button once it runs
    /// long, and act on a click of it.
    fn sync_top_up_overlay(&mut self) {
        self.resume_after_unseen_top_up_end();
        let Some(handle) = self.top_up_overlay.clone() else {
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
        if let Some(dispatch) = &self.top_up_context {
            send_top_up_to_background(self.app_context.egui_ctx(), dispatch);
        }
    }

    /// Bring back an empty form once the top-up this screen sent has ended
    /// without its result reaching the screen: only a screen in view when its
    /// top-up ends is told how it ended.
    fn resume_after_unseen_top_up_end(&mut self) {
        let Some(dispatch) = &self.top_up_context else {
            return;
        };
        if top_ups_in_flight(self.app_context.egui_ctx())
            .iter()
            .any(|top_up| top_up.dispatch == *dispatch)
        {
            return;
        }
        self.release_top_up();
        // The top-up may have succeeded, so the form must not offer it again.
        self.forget_sent_funding();
        self.set_step(step_after_task_failure(self.current_step()));
        self.reload_identity();
    }

    /// Re-read the identity: a top-up changes its stored balance and top-up
    /// count, and this screen may not be the one that is told about it.
    fn reload_identity(&mut self) {
        if let Ok(Some(identity)) = self
            .app_context
            .get_identity_by_id(&self.identity.identity.id())
        {
            self.identity = identity;
        }
    }

    /// Test seam: enter the state "Add funds" enters and return the dispatch,
    /// without handing the task to the backend.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn begin_top_up_for_test(&mut self) -> Option<BackendTaskContext> {
        // The task is dropped on purpose: no funds may move in a test.
        let _task = self.begin_top_up(
            IdentityTask::TopUpIdentityFromPlatformAddresses {
                identity: self.identity.clone(),
                inputs: Default::default(),
                wallet_seed_hash: Default::default(),
            },
            WalletFundedScreenStep::WaitingForPlatformAcceptance,
        );
        self.top_up_context.clone()
    }

    /// Test seam: age the blocking overlay of the running top-up by `by`.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn backdate_top_up_for_test(&self, by: Duration) {
        if let Some(overlay) = &self.top_up_overlay {
            overlay.backdate(by);
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
        self.asset_lock_balance
            .get(seed_hash)
            .is_none_or(|ceiling| self.top_up_amount_range(ceiling).is_some())
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

    /// Why `wallet` cannot serve `method`, or `None` while it remains
    /// eligible; an unloaded ceiling does not block it. A busy wallet lock
    /// reads as ineligible rather than panicking.
    fn wallet_unavailable_reason(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        method: FundingMethod,
    ) -> Option<&'static str> {
        let Ok(w) = wallet.read() else {
            return Some(WALLET_BUSY);
        };
        let seed_hash = w.seed_hash();
        match method {
            FundingMethod::UseWalletBalance
                if !self.wallet_balance_can_afford_top_up(&seed_hash) =>
            {
                Some(WALLET_LACKS_DASH)
            }
            // Only a completed read can show there is nothing to use. While it
            // is unfinished or failed the wallet stays selectable, so the
            // form's retry stays reachable.
            FundingMethod::UseUnusedAssetLock
                if self.asset_lock_cache.get(&seed_hash).is_some()
                    && !self.asset_lock_cache.has_unused(&seed_hash) =>
            {
                Some(WALLET_HAS_NO_FUNDING)
            }
            _ => None,
        }
    }

    /// Whether `wallet` remains eligible for `method`.
    fn wallet_has_resources_for(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        method: FundingMethod,
    ) -> bool {
        self.wallet_unavailable_reason(wallet, method).is_none()
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        let mut selected_wallet_update: Option<Arc<RwLock<Wallet>>> = None;
        let mut step_update_method: Option<FundingMethod> = None;

        let rendered = if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = self.app_context.wallet_context().wallets();

            if wallets.len() > 1 {
                let funding_method = self.current_funding_method();
                let unavailable: Vec<_> = wallets
                    .iter()
                    .filter_map(|(seed_hash, wallet)| {
                        let reason = self.wallet_unavailable_reason(wallet, funding_method)?;
                        Some((WalletChoice::Hd(*seed_hash), reason.to_string()))
                    })
                    .collect();
                let selector = self.wallet_selector.get_or_insert_with(|| {
                    WalletSelector::new("select_wallet").with_core_figure(CoreFigure::Usable)
                });
                // The row shows the balance the chosen funding method draws on.
                selector.set_balance_kinds(&[funding_method.balance_kind()]);
                selector.set_entries(self.app_context.wallet_selector_entries(false));
                selector.set_unavailable(unavailable);
                selector.set_selected(self.wallet.as_deref().map(WalletChoice::of_hd_wallet));
                let response = selector.show(ui).inner;
                if response.has_changed()
                    && let Some(WalletChoice::Hd(seed_hash)) = response.changed_value()
                {
                    selected_wallet_update = wallets.get(seed_hash).cloned();
                    step_update_method = Some(funding_method);
                }
                true
            } else if let Some(wallet) = wallets.values().next() {
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
                if let Some(message) = self.network_fee_refusal(amount, max_amount) {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        message,
                        MessageType::Warning,
                    );
                    return AppAction::None;
                }
                let identity_fee_duffs = self.top_up_reserve_duffs();
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

    /// Smallest top-up, in duffs, the network accepts. `None` when it cannot
    /// be read for the protocol version in use; the backend then refuses the
    /// top-up instead.
    fn minimum_top_up_duffs(&self) -> Option<u64> {
        identity_topup_min_funding_duffs(self.app_context.sdk_platform_version()).ok()
    }

    /// Fee reserve, in duffs, kept back from the builder ceiling.
    fn top_up_reserve_duffs(&self) -> u64 {
        self.app_context
            .fee_estimator()
            .estimate_identity_topup()
            .div_ceil(CREDITS_PER_DUFF)
    }

    /// Amounts, in duffs, a wallet that can build `ceiling_duffs` may top up
    /// with. `None` when nothing it can send covers the network fee.
    fn top_up_amount_range(&self, ceiling_duffs: u64) -> Option<RangeInclusive<u64>> {
        asset_lock_user_amount_range(
            ceiling_duffs,
            self.top_up_reserve_duffs(),
            self.minimum_top_up_duffs().unwrap_or(0),
        )
    }

    /// What a wallet must hold, in credits, before it can send any top-up. Every
    /// "add at least" text and deposit threshold reads this, so none of them
    /// asks for an amount the form then cannot use.
    fn required_wallet_credits(&self) -> u64 {
        self.minimum_top_up_duffs()
            .unwrap_or(0)
            .saturating_add(self.top_up_reserve_duffs())
            .saturating_mul(CREDITS_PER_DUFF)
    }

    /// Why `amount_duffs` cannot be sent, for network-fee reasons, from a wallet
    /// that can build `ceiling_duffs` — as banner text. `None` when the fee is
    /// covered, or cannot be read here (the backend then decides).
    fn network_fee_refusal(&self, amount_duffs: u64, ceiling_duffs: u64) -> Option<String> {
        let minimum_duffs = self.minimum_top_up_duffs()?;
        if self.top_up_amount_range(ceiling_duffs).is_none() {
            return Some(TOP_UP_FEE_NOT_COVERED.to_string());
        }
        validate_asset_lock_minimum(amount_duffs, minimum_duffs)
            .err()
            .map(|error| {
                TaskError::AssetLockAmountBelowNetworkFee {
                    amount_duffs,
                    minimum_duffs: error.minimum_amount_duffs,
                }
                .to_string()
            })
    }

    fn top_up_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.current_funding_method();
        let available_ceiling_duffs = self.current_validation_ceiling_duffs(funding_method);

        // Offer no amount at all when none covers the network fee, so neither
        // Max nor the prefill can propose one the network would refuse.
        if available_ceiling_duffs.is_some_and(|c| self.top_up_amount_range(c).is_none()) {
            self.funding_amount_exact = None;
            ui.colored_label(DashColors::WARNING, TOP_UP_FEE_NOT_COVERED);
            ui.add_space(10.0);
            return;
        }
        let minimum_duffs = self.minimum_top_up_duffs();

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
        if let Some(minimum) = minimum_duffs {
            amount_input.set_min_amount(Some(minimum.saturating_mul(CREDITS_PER_DUFF)));
            amount_input.set_caption(Some(format!(
                "The network fee is taken from this amount, so it must be at least {}.",
                format_duffs_as_dash(minimum)
            )));
        }

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
        self.reload_identity();
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
            let minimum_credits = self.required_wallet_credits();
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
    use crate::ui::components::message_banner::{MAX_BANNERS, global_banner_texts};
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
        screen.funding_amount_exact = Some(100_000);
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
    }

    /// A screen funding from the wallet balance, with two named wallets loaded
    /// and the first one chosen.
    fn two_wallet_screen() -> (TopUpIdentityScreen, [WalletSeedHash; 2], tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_context = test_app_context(temp_dir.path());
        let mut wallets = Vec::new();
        for (seed_byte, alias) in [(0x61, "Alpha"), (0x62, "Beta")] {
            let wallet = Wallet::new_from_seed(
                [seed_byte; 64],
                Network::Testnet,
                Some(alias.to_string()),
                None,
            )
            .expect("wallet");
            let seed_hash = wallet.seed_hash();
            let wallet = Arc::new(RwLock::new(wallet));
            app_context
                .wallet_context()
                .insert_test_wallet(seed_hash, wallet.clone());
            wallets.push((seed_hash, wallet));
        }
        app_context.has_wallet.store(true, Ordering::Relaxed);
        let mut screen = TopUpIdentityScreen::new(test_identity(Network::Testnet), &app_context);
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::UseWalletBalance;
        screen.wallet = Some(wallets[0].1.clone());
        (screen, [wallets[0].0, wallets[1].0], temp_dir)
    }

    /// The wallet selector names the wallet type and shows how much the wallet
    /// can put towards the top-up — a bare wallet name tells the user nothing
    /// about which wallet can pay.
    #[test]
    fn wallet_selector_shows_the_balance_next_to_the_wallet_name() {
        use egui_kittest::{Harness, kittest::Queryable};
        let (mut screen, _seed_hashes, _temp_dir) = two_wallet_screen();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui(|ui| {
                screen.ui(ui);
            });
        harness.run_steps(2);

        assert!(
            harness.query_by_value("HD: Alpha — 0 DASH").is_some(),
            "the closed selector must show the chosen wallet with its balance"
        );
    }

    /// A wallet the funding method cannot draw on is greyed out, and the row
    /// carries the reason instead of leaving the user to guess.
    #[test]
    fn wallet_that_cannot_fund_the_top_up_is_greyed_out_with_a_reason() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::{NodeT, Queryable};
        let (mut screen, [alpha, beta], _temp_dir) = two_wallet_screen();
        store_current_quote(&mut screen, beta, 1);
        let wallets = screen.app_context.wallet_context().wallets();

        assert_eq!(
            screen.wallet_unavailable_reason(&wallets[&beta], FundingMethod::UseWalletBalance),
            Some(WALLET_LACKS_DASH)
        );
        assert_eq!(
            screen.wallet_unavailable_reason(&wallets[&alpha], FundingMethod::UseWalletBalance),
            None,
            "a wallet whose usable amount is still being checked stays available"
        );
        screen.asset_lock_cache.store(alpha, Vec::new());
        assert_eq!(
            screen.wallet_unavailable_reason(&wallets[&alpha], FundingMethod::UseUnusedAssetLock),
            Some(WALLET_HAS_NO_FUNDING)
        );
        assert_eq!(
            screen.wallet_unavailable_reason(&wallets[&beta], FundingMethod::ReceiveDeposit),
            None
        );

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui(|ui| {
                screen.ui(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("HD: Alpha — 0 DASH").click();
        harness.run_steps(2);
        assert!(
            harness
                .get_by_label("HD: Beta — 0 DASH")
                .accesskit_node()
                .is_disabled(),
            "the wallet that cannot pay must be greyed out"
        );
    }

    /// A read that is unfinished or failed says nothing about what the wallet
    /// holds, so only a completed read with nothing to use greys the wallet out.
    #[test]
    fn wallet_stays_available_until_a_read_shows_no_funding_transaction() {
        let (mut screen, [alpha, beta], _temp_dir) = two_wallet_screen();
        let wallets = screen.app_context.wallet_context().wallets();
        let reason = |screen: &TopUpIdentityScreen, seed_hash: &WalletSeedHash| {
            screen.wallet_unavailable_reason(&wallets[seed_hash], FundingMethod::UseUnusedAssetLock)
        };

        assert_eq!(reason(&screen, &alpha), None, "not read yet");
        let _ = screen.asset_lock_cache.ensure_requested_many([alpha, beta]);
        assert_eq!(reason(&screen, &alpha), None, "still loading");
        screen.asset_lock_cache.mark_loading_failed();
        assert_eq!(reason(&screen, &alpha), None, "the read failed");

        screen.asset_lock_cache.store(alpha, Vec::new());
        assert_eq!(reason(&screen, &alpha), Some(WALLET_HAS_NO_FUNDING));
        assert_eq!(reason(&screen, &beta), None, "another wallet's read");
    }

    /// When no wallet's funding transactions could be loaded, the user can
    /// still pick a wallet and reach the form's retry.
    #[test]
    fn wallet_whose_funding_read_failed_can_be_picked_to_retry() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::{NodeT, Queryable};
        let (mut screen, [alpha, beta], _temp_dir) = two_wallet_screen();
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::UseUnusedAssetLock;
        screen.wallet = None;
        let _ = screen.asset_lock_cache.ensure_requested_many([alpha, beta]);
        screen.asset_lock_cache.mark_loading_failed();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui(|ui| {
                screen.ui(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("Select a wallet").click();
        harness.run_steps(2);
        let row = harness.get_by_label("HD: Alpha — 0 DASH");
        assert!(
            !row.accesskit_node().is_disabled(),
            "a wallet whose read failed must stay selectable"
        );
        row.click();
        harness.run_steps(2);

        assert!(
            harness.query_by_label("Retry").is_some(),
            "picking the wallet must lead to the retry"
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

    /// The form of a running top-up could send the same funds a second time.
    #[test]
    fn in_flight_top_up_offers_no_way_to_send_again() {
        let (mut screen, _temp_dir) = funded_form_screen(0x48);
        assert!(
            screen_shows(&mut screen, ADD_FUNDS_BUTTON),
            "an idle form with an amount entered offers to send it"
        );

        let action = screen.top_up_identity_clicked(FundingMethod::UseWalletBalance);
        assert!(
            matches!(action, AppAction::BackendTaskWithContext { .. }),
            "expected a dispatched top-up, got {action:?}"
        );
        assert!(screen_shows(&mut screen, TOP_UP_FORM_PAUSED));
        assert!(
            !screen_shows(&mut screen, ADD_FUNDS_BUTTON),
            "a running top-up must not offer to send the funds again"
        );
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

    /// One frame of the app's own egui context in which `key` is pressed and
    /// released while the dialog claims the keyboard, as the app loop has it do.
    #[cfg(feature = "testing")]
    fn press_key_on_dialog(ctx: &egui::Context, key: egui::Key) {
        let key_event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput {
            // Without the release egui reads the next press as a key repeat.
            events: vec![key_event(true), key_event(false)],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| ProgressOverlay::claim_input(ui.ctx()))
            .drop_without_applying_deltas();
    }

    /// The offered button is the only way out of a dialog that can stay up
    /// for as long as the network takes, so its keys have to reach it.
    #[cfg(feature = "testing")]
    #[test]
    fn enter_or_space_on_the_offered_button_continues_the_top_up_in_background() {
        for (seed_byte, key) in [(0x4a, egui::Key::Enter), (0x4b, egui::Key::Space)] {
            let (mut screen, _context, _temp_dir) = dispatched_top_up_screen(seed_byte);
            let ctx = screen.app_context.egui_ctx().clone();

            press_key_on_dialog(&ctx, key);
            screen.sync_top_up_overlay();
            assert!(
                ProgressOverlay::has_global(&ctx),
                "a top-up that just started has no way out, by {key:?} or otherwise"
            );
            assert!(!overlay_text(&ctx).contains(TOP_UP_BACKGROUND_LABEL));

            screen
                .top_up_overlay
                .as_ref()
                .expect("overlay raised")
                .backdate(TOP_UP_BACKGROUND_OFFER_AFTER);
            screen.sync_top_up_overlay();
            assert!(
                overlay_text(&ctx).contains(TOP_UP_BACKGROUND_LABEL),
                "a top-up that runs long shows the button on its dialog"
            );

            press_key_on_dialog(&ctx, key);
            screen.sync_top_up_overlay();
            assert!(
                !ProgressOverlay::has_global(&ctx),
                "{key:?} must give the app back"
            );
            assert!(MessageBanner::has_global(&ctx));
            assert!(screen_shows(&mut screen, TOP_UP_FORM_PAUSED));
        }
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
        let other = TopUpIdentityScreen::new(test_identity(Network::Testnet), &screen.app_context);
        assert!(!other.top_up_in_flight());

        // Another transfer to the same identity is not this top-up.
        let other_transfer =
            BackendTaskContext::IdentityTopUp(context.identity_top_up_identity().unwrap());
        assert!(!finish_top_up(&ctx, &other_transfer, false));
        assert!(reopened.top_up_in_flight());

        assert!(finish_top_up(&ctx, &context, false));
        assert!(!MessageBanner::has_global(&ctx));
        assert!(!reopened.top_up_in_flight());
    }

    fn top_up_dispatch(identity_id: Identifier, dispatch_id: u64) -> BackendTaskContext {
        BackendTaskContext::Dispatched {
            dispatch_id,
            operation: Box::new(BackendTaskContext::IdentityTopUp(identity_id)),
        }
    }

    /// Record a top-up of `identity_id` the way a dispatch from Add Funds does.
    fn tracked_top_up(
        ctx: &egui::Context,
        identity_id: Identifier,
        dispatch_id: u64,
    ) -> BackendTaskContext {
        let dispatch = top_up_dispatch(identity_id, dispatch_id);
        track_top_up(
            ctx,
            dispatch.clone(),
            top_up_done_message(Some("Savings"), &identity_id),
        );
        dispatch
    }

    #[test]
    fn top_up_result_ends_its_banner_and_confirms_a_background_success() {
        let ctx = egui::Context::default();
        let identity_id = Identifier::from([1; 32]);

        // A top-up the user watched to the end is confirmed by its screen.
        let watched = tracked_top_up(&ctx, identity_id, 1);
        assert!(finish_top_up(&ctx, &watched, true));
        assert!(!MessageBanner::has_global(&ctx));

        let succeeded = tracked_top_up(&ctx, identity_id, 2);
        send_top_up_to_background(&ctx, &succeeded);
        assert!(MessageBanner::has_global(&ctx));
        assert!(finish_top_up(&ctx, &succeeded, true));
        assert!(
            MessageBanner::has_global(&ctx),
            "a successful background top-up must show its confirmation"
        );
        MessageBanner::clear_global_message(
            &ctx,
            top_up_done_message(Some("Savings"), &identity_id),
        );
        assert!(
            !MessageBanner::has_global(&ctx),
            "success must swap the progress banner for a confirmation naming the identity"
        );

        let failed = tracked_top_up(&ctx, identity_id, 3);
        send_top_up_to_background(&ctx, &failed);
        assert!(finish_top_up(&ctx, &failed, false));
        assert!(
            !MessageBanner::has_global(&ctx),
            "a failed top-up ends its progress banner without confirming"
        );
        assert!(
            !finish_top_up(&ctx, &failed, false),
            "a top-up ends only once"
        );
    }

    /// Wallet Send can move funds to the same identity while its top-up runs
    /// in the background; that transfer's result is not the top-up's.
    #[test]
    fn another_transfer_to_the_same_identity_leaves_its_top_up_in_flight() {
        let ctx = egui::Context::default();
        let identity_id = Identifier::from([1; 32]);
        let top_up = tracked_top_up(&ctx, identity_id, 1);
        send_top_up_to_background(&ctx, &top_up);

        for other in [
            BackendTaskContext::Other,
            BackendTaskContext::IdentityTopUp(identity_id),
            top_up_dispatch(identity_id, 2),
            top_up_dispatch(Identifier::from([2; 32]), 1),
        ] {
            assert!(!finish_top_up(&ctx, &other, true));
        }

        assert!(
            top_up_in_flight_for(&ctx, &identity_id),
            "another transfer to the identity must leave its top-up in flight"
        );
        assert!(MessageBanner::has_global(&ctx));
        MessageBanner::clear_global_message(&ctx, TOP_UP_IN_BACKGROUND);
        assert!(
            !MessageBanner::has_global(&ctx),
            "another transfer's success must not confirm the top-up"
        );
    }

    #[test]
    fn progress_banner_stays_until_the_last_background_top_up_ends() {
        let ctx = egui::Context::default();
        let first = tracked_top_up(&ctx, Identifier::from([1; 32]), 1);
        let second = tracked_top_up(&ctx, Identifier::from([2; 32]), 2);
        let watched = tracked_top_up(&ctx, Identifier::from([3; 32]), 3);
        send_top_up_to_background(&ctx, &first);
        send_top_up_to_background(&ctx, &second);

        assert!(finish_top_up(&ctx, &first, false));
        assert!(finish_top_up(&ctx, &watched, false));
        assert!(
            MessageBanner::has_global(&ctx),
            "the banner follows the top-up still running in the background"
        );

        assert!(finish_top_up(&ctx, &second, false));
        assert!(!MessageBanner::has_global(&ctx));
    }

    /// How many background-progress banners the global list holds.
    fn background_notices(ctx: &egui::Context) -> usize {
        global_banner_texts(ctx)
            .iter()
            .filter(|text| *text == TOP_UP_IN_BACKGROUND)
            .count()
    }

    /// The dispatches of the top-ups in flight.
    fn tracked_dispatches(ctx: &egui::Context) -> Vec<BackendTaskContext> {
        top_ups_in_flight(ctx)
            .into_iter()
            .map(|top_up| top_up.dispatch)
            .collect()
    }

    fn unrelated_notification(round: u8, n: usize) -> String {
        format!("Unrelated notification {round}-{n}.")
    }

    /// Raise as many unrelated notifications as the global list holds, which
    /// pushes every older banner out of it.
    fn flood_banners(ctx: &egui::Context, round: u8) {
        for n in 0..MAX_BANNERS {
            MessageBanner::set_global(ctx, unrelated_notification(round, n), MessageType::Info);
        }
    }

    /// The global banner list is capped, so later notifications can push the
    /// progress banner out while its top-up still runs.
    #[test]
    fn background_top_up_banner_returns_after_the_banner_cap_drops_it() {
        let ctx = egui::Context::default();
        let top_up = tracked_top_up(&ctx, Identifier::from([1; 32]), 1);
        send_top_up_to_background(&ctx, &top_up);

        flood_banners(&ctx, 1);
        assert_eq!(
            background_notices(&ctx),
            0,
            "the flood must have pushed the progress banner out"
        );

        // Two frames: a banner that is back must not be raised again.
        restore_top_up_background_banner(&ctx);
        restore_top_up_background_banner(&ctx);
        assert_eq!(
            background_notices(&ctx),
            1,
            "a top-up still running in the background must get its banner back"
        );
        assert_eq!(
            tracked_dispatches(&ctx),
            vec![top_up.clone()],
            "bringing the banner back must leave the top-up tracked exactly once"
        );

        // The banner that came back is protected like the first one.
        flood_banners(&ctx, 2);
        assert_eq!(background_notices(&ctx), 0);
        restore_top_up_background_banner(&ctx);
        assert_eq!(background_notices(&ctx), 1);

        // Dropped once more, and this time the top-up ends before the next frame.
        flood_banners(&ctx, 3);
        assert!(finish_top_up(&ctx, &top_up, false));
        restore_top_up_background_banner(&ctx);
        assert_eq!(
            background_notices(&ctx),
            0,
            "a top-up that ended must not get its progress banner back"
        );
    }

    /// Only the banner cap is undone. A user who closed the progress banner
    /// asked for it to go away, also when the one closed had come back before.
    #[test]
    fn closed_background_top_up_banner_stays_closed() {
        use egui_kittest::{Harness, kittest::Queryable};
        const DISMISS: &str = "\u{274C}";
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui(MessageBanner::show_global);
        let ctx = harness.ctx.clone();
        let top_up = tracked_top_up(&ctx, Identifier::from([1; 32]), 1);
        send_top_up_to_background(&ctx, &top_up);
        flood_banners(&ctx, 1);
        restore_top_up_background_banner(&ctx);
        assert_eq!(background_notices(&ctx), 1);

        // Leave the progress banner alone on screen, then close it by its button.
        for n in 0..MAX_BANNERS {
            MessageBanner::clear_global_message(&ctx, unrelated_notification(1, n));
        }
        harness.run_steps(2);
        harness.get_by_label(DISMISS).click();
        harness.run_steps(2);
        assert_eq!(
            background_notices(&ctx),
            0,
            "the dismiss button must close the progress banner"
        );

        flood_banners(&ctx, 2);
        restore_top_up_background_banner(&ctx);
        assert_eq!(
            background_notices(&ctx),
            0,
            "a progress banner the user closed must stay closed"
        );
        assert_eq!(tracked_dispatches(&ctx), vec![top_up]);
    }

    #[test]
    fn confirmation_of_a_named_identity_gives_its_name_and_id() {
        let id = Identifier::from([7; 32]);
        let full_id = id.to_string(Encoding::Base58);
        assert_eq!(
            top_up_done_message(Some("Savings"), &id),
            format!("The funds were added to the identity Savings (ID: {full_id}).")
        );
    }

    /// An identity without a name is otherwise shown by a shortened ID, which
    /// must not appear next to the full one.
    #[test]
    fn confirmation_of_a_nameless_identity_gives_its_id_once() {
        let id = Identifier::from([7; 32]);
        let full_id = id.to_string(Encoding::Base58);
        let message = top_up_done_message(None, &id);
        assert_eq!(
            message,
            format!("The funds were added to the identity {full_id}.")
        );
        assert_eq!(message.matches(&full_id).count(), 1);
        assert!(!message.contains("(ID:") && !message.contains('…'));
    }

    /// Two identities can carry the same name, and a banner is not raised
    /// again for a text that is already on screen.
    #[test]
    fn identities_sharing_a_name_get_a_confirmation_each() {
        let ctx = egui::Context::default();
        let first_id = Identifier::from([1; 32]);
        let second_id = Identifier::from([2; 32]);
        let first = tracked_top_up(&ctx, first_id, 1);
        let second = tracked_top_up(&ctx, second_id, 2);
        send_top_up_to_background(&ctx, &first);
        send_top_up_to_background(&ctx, &second);

        // The second ends while the confirmation of the first is still shown.
        assert!(finish_top_up(&ctx, &first, true));
        assert!(finish_top_up(&ctx, &second, true));

        let confirmations = global_banner_texts(&ctx);
        assert_eq!(
            confirmations.len(),
            2,
            "each top-up is confirmed by a banner of its own, got {confirmations:?}"
        );
        assert!(
            confirmations[0].contains(&first_id.to_string(Encoding::Base58))
                && confirmations[1].contains(&second_id.to_string(Encoding::Base58)),
            "each confirmation tells which identity it is about, got {confirmations:?}"
        );
    }

    /// Two top-ups in the background share the one progress banner, so the
    /// end of the first must leave the second everything it still needs.
    #[test]
    fn background_banner_serves_the_top_up_that_outlasts_another() {
        let ctx = egui::Context::default();
        let first = tracked_top_up(&ctx, Identifier::from([1; 32]), 1);
        let second = tracked_top_up(&ctx, Identifier::from([2; 32]), 2);
        send_top_up_to_background(&ctx, &first);
        send_top_up_to_background(&ctx, &second);
        assert_eq!(
            background_notices(&ctx),
            1,
            "two background top-ups share one progress banner"
        );

        assert!(finish_top_up(&ctx, &first, true));
        assert_eq!(tracked_dispatches(&ctx), vec![second.clone()]);
        assert_eq!(
            background_notices(&ctx),
            1,
            "the progress banner stays for the top-up still running"
        );

        // The banner cap strikes between the two ends.
        flood_banners(&ctx, 1);
        assert_eq!(background_notices(&ctx), 0);
        restore_top_up_background_banner(&ctx);
        assert_eq!(
            background_notices(&ctx),
            1,
            "the top-up still running must get its banner back"
        );

        // Dropped again, and the last top-up ends before the next frame.
        flood_banners(&ctx, 2);
        assert!(finish_top_up(&ctx, &second, true));
        assert!(tracked_dispatches(&ctx).is_empty());
        assert!(
            ctx.data(|data| {
                data.get_temp::<BannerHandle>(egui::Id::new(BACKGROUND_TOP_UP_BANNER_ID))
            })
            .is_none(),
            "nothing of the banner is kept once the last top-up has ended"
        );
        restore_top_up_background_banner(&ctx);
        flood_banners(&ctx, 3);
        restore_top_up_background_banner(&ctx);
        assert_eq!(
            background_notices(&ctx),
            0,
            "no progress banner comes back once the last top-up has ended"
        );
    }

    /// A screen that is not in view when its top-up ends never learns how it
    /// ended, so the form it brings back must not still hold the amount that
    /// was just sent.
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

        assert!(finish_top_up(&ctx, &context, true));
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

    /// A network switch drops the screen, its dialog and every banner while
    /// the top-up it sent keeps running.
    #[test]
    fn top_up_stays_in_flight_when_its_screen_and_dialog_are_dropped() {
        let (screen, context, _temp_dir) = dispatched_top_up_screen(0x49);
        let ctx = screen.app_context.egui_ctx().clone();
        let app_context = screen.app_context.clone();
        let identity = screen.identity.clone();

        drop(screen);
        ProgressOverlay::clear_all_global(&ctx);
        MessageBanner::clear_all_global(&ctx);
        follow_top_ups_after_network_switch(&ctx);
        assert!(
            MessageBanner::has_global(&ctx),
            "a top-up that lost its dialog is followed by the banner"
        );

        let mut reopened = TopUpIdentityScreen::new(identity, &app_context);
        assert!(
            reopened.top_up_in_flight(),
            "a top-up outlives the screen that sent it"
        );
        assert!(screen_shows(&mut reopened, TOP_UP_FORM_PAUSED));

        assert!(finish_top_up(&ctx, &context, true));
        assert!(!reopened.top_up_in_flight());
        assert!(
            MessageBanner::has_global(&ctx),
            "a top-up that lost its screen is confirmed by a banner"
        );
        // The fixture identity has no name, so its ID alone tells which one it is.
        let id = reopened.identity.identity.id().to_string(Encoding::Base58);
        MessageBanner::clear_global_message(
            &ctx,
            format!("The funds were added to the identity {id}."),
        );
        assert!(
            !MessageBanner::has_global(&ctx),
            "an identity without a name is confirmed by its ID, written once"
        );
    }

    #[test]
    fn network_switch_without_a_running_top_up_raises_no_banner() {
        let ctx = egui::Context::default();
        follow_top_ups_after_network_switch(&ctx);
        assert!(!MessageBanner::has_global(&ctx));
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

    /// What the wallet in the reported case could build: 5 237 duffs remain
    /// after the fee reserve, far below the fee the network takes.
    const REPORTED_CEILING_DUFFS: u64 = 55_737;
    const AMOUNT_FIELD: &str = "Amount:";

    /// A wallet-balance form whose wallet can build at most `ceiling_duffs`.
    fn wallet_form_with_ceiling(
        seed_byte: u8,
        ceiling_duffs: u64,
    ) -> (TopUpIdentityScreen, tempfile::TempDir) {
        let (mut screen, seed_hash, temp_dir) = wallet_balance_screen(seed_byte);
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::UseWalletBalance;
        store_current_quote(&mut screen, seed_hash, ceiling_duffs);
        screen.set_step(WalletFundedScreenStep::ReadyToCreate);
        (screen, temp_dir)
    }

    /// The reported case: the form used to fill itself with 0.00005237 DASH and
    /// offer to send it, and the network then refused the funding.
    #[test]
    fn no_amount_is_offered_when_the_wallet_cannot_cover_the_network_fee() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x51, REPORTED_CEILING_DUFFS);
        screen.prefill_funding_amount = true;

        assert!(
            screen_shows(&mut screen, NOT_ENOUGH_DASH),
            "the wallet must be reported as too small to top up from"
        );
        assert!(
            screen_shows(&mut screen, "Add at least 0.00101 DASH to continue."),
            "the amount to add must leave a top-up the network accepts"
        );
        assert!(!screen_shows(&mut screen, AMOUNT_FIELD));
        assert!(!screen_shows(&mut screen, ADD_FUNDS_BUTTON));
    }

    #[test]
    fn form_states_the_smallest_amount_it_accepts() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x52, 10_000_000);
        assert!(screen_shows(
            &mut screen,
            "The network fee is taken from this amount, so it must be at least 0.000505 DASH."
        ));
    }

    /// The reported amount typed into a form that could send far more.
    #[test]
    fn form_does_not_offer_to_send_an_amount_below_the_network_fee() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x53, 10_000_000);
        let mut typed = AmountInput::new(Amount::new_dash(0.0)).with_label(AMOUNT_FIELD);
        typed.set_value(Amount::dash_from_duffs(5_237));
        screen.funding_amount_input = Some(typed);

        assert!(screen_shows(
            &mut screen,
            "Amount must be at least 0.000505"
        ));
        assert!(!screen_shows(&mut screen, ADD_FUNDS_BUTTON));
    }

    #[test]
    fn top_up_dispatch_refuses_an_amount_below_the_network_fee() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x54, 10_000_000);
        screen.funding_amount_exact = Some(5_237);

        let action = screen.top_up_identity_clicked(FundingMethod::UseWalletBalance);

        assert!(
            matches!(action, AppAction::None),
            "expected no dispatch, got {action:?}"
        );
        assert_eq!(
            global_banner_texts(screen.app_context.egui_ctx()),
            vec![
                "This amount is too small to cover the network fee. \
                 Enter at least 0.000505 DASH and try again."
                    .to_string()
            ]
        );
    }

    /// The network accepts a funding equal to its fee, so the form must too.
    #[test]
    fn top_up_dispatch_accepts_an_amount_equal_to_the_network_fee() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x55, 10_000_000);
        screen.funding_amount_exact = Some(50_500);

        let action = screen.top_up_identity_clicked(FundingMethod::UseWalletBalance);

        assert!(
            matches!(action, AppAction::BackendTaskWithContext { .. }),
            "expected a dispatched top-up, got {action:?}"
        );
    }

    /// An amount above what the reported wallet can send used to be answered
    /// with "You can transfer up to 0.00005237 DASH" — a doomed suggestion.
    #[test]
    fn top_up_dispatch_never_suggests_an_amount_the_network_refuses() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x56, REPORTED_CEILING_DUFFS);
        screen.funding_amount_exact = Some(50_500);

        let action = screen.top_up_identity_clicked(FundingMethod::UseWalletBalance);

        assert!(
            matches!(action, AppAction::None),
            "expected no dispatch, got {action:?}"
        );
        assert_eq!(
            global_banner_texts(screen.app_context.egui_ctx()),
            vec![TOP_UP_FEE_NOT_COVERED.to_string()]
        );
    }

    /// A deposit that leaves less than the network fee after the reserve must
    /// not be turned into an amount, by Max or by the prefill.
    #[test]
    fn deposit_form_offers_no_amount_when_none_covers_the_network_fee() {
        let (mut screen, _temp_dir) = wallet_form_with_ceiling(0x59, 10_000_000);
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::ReceiveDeposit;
        screen.funding_address_balance_duffs = REPORTED_CEILING_DUFFS;
        screen.set_step(WalletFundedScreenStep::FundsReceived);
        screen.prefill_funding_amount = true;

        assert!(screen_shows(&mut screen, TOP_UP_FEE_NOT_COVERED));
        assert!(!screen_shows(&mut screen, AMOUNT_FIELD));
        assert!(!screen_shows(&mut screen, ADD_FUNDS_BUTTON));
    }

    /// A deposit-funded screen showing its deposit address.
    fn deposit_screen(seed_byte: u8) -> (TopUpIdentityScreen, Address, tempfile::TempDir) {
        use dash_sdk::dpp::dashcore::PubkeyHash;
        use dash_sdk::dpp::dashcore::address::Payload;

        let (mut screen, _seed_hash, temp_dir) = wallet_balance_screen(seed_byte);
        *screen.funding_method.write().expect("funding method lock") =
            FundingMethod::ReceiveDeposit;
        let address = Address::new(
            Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array([seed_byte; 20])),
        );
        screen.funding_address = Some(address.clone());
        screen.set_step(WalletFundedScreenStep::WaitingOnFunds);
        (screen, address, temp_dir)
    }

    /// The deposit request used to ask for 0.0006 DASH, which leaves less than
    /// the network fee once the reserve is kept back.
    #[test]
    fn deposit_request_asks_for_enough_to_leave_a_top_up_the_network_accepts() {
        let (mut screen, _address, _temp_dir) = deposit_screen(0x57);
        assert!(screen_shows(
            &mut screen,
            "Send at least 0.0011 DASH to this address to top up your identity."
        ));
    }

    /// A deposit of what the old request asked for must keep waiting instead of
    /// opening a form that can only offer an amount the network refuses.
    #[test]
    fn deposit_too_small_for_the_network_fee_keeps_waiting() {
        use dash_sdk::dpp::dashcore::{Transaction, TxOut};

        let (mut screen, address, _temp_dir) = deposit_screen(0x58);
        let deposit = |duffs: u64| {
            BackendTaskSuccessResult::CoreItem(CoreItem::ReceivedAvailableUTXOTransaction(
                Transaction {
                    version: 3,
                    lock_time: 0,
                    input: Vec::new(),
                    output: Vec::new(),
                    special_transaction_payload: None,
                },
                vec![(
                    OutPoint::null(),
                    TxOut {
                        value: duffs,
                        script_pubkey: address.script_pubkey(),
                    },
                    address.clone(),
                )],
            ))
        };

        screen.display_task_result(deposit(60_000));
        assert_eq!(
            screen.current_step(),
            WalletFundedScreenStep::WaitingOnFunds
        );

        screen.display_task_result(deposit(110_000));
        assert_eq!(screen.current_step(), WalletFundedScreenStep::FundsReceived);
    }
}
